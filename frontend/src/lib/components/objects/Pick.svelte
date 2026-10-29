<script lang="ts" generics="Row extends { id: string; name?: string | null; folder?: string | null }">
	/**
	 * Picks one show object — a sequence, a cue, a timeline, a variable — by typing a
	 * bit of its name, the objects grouped by folder.
	 *
	 * In place of a native `<select>`, which cannot be searched and draws eighty
	 * sequences as eighty lines to scroll. Pressing the button opens a list with a
	 * search field already focused; arrows move, Enter takes, Escape leaves. The
	 * search reads the folder path as well as the name (`pick.ts`).
	 *
	 * **The list is drawn on the page's body**, not beside the button. A flow node sits
	 * inside a transformed canvas, where `position: fixed` is measured from the canvas
	 * rather than the screen and a list there is cut off by the node's own tile — so it
	 * is moved out from under all of that and placed against the button's rectangle,
	 * above it where there is no room below. It closes on a press anywhere else.
	 *
	 * **A scroll moves it, and does not close it.** Closing on any scroll was the first
	 * version, and in a flow it never opened: pressing a picker in a node near the edge
	 * of the board focuses it, the browser scrolls the board's clipped container to
	 * show it, and that scroll shut the list in the same frame it opened. So a scroll
	 * puts the list back against the button, and only a button gone off the screen
	 * closes it.
	 */
	import { tick } from 'svelte';

	import { matches, pickGroups, pickGroupsBy } from '$lib/pick.js';
	import { folders } from '$lib/stores/folders.js';

	let {
		rows,
		value,
		onpick,
		label = (row: Row) => row.name ?? '',
		hint,
		group,
		none,
		extras = [],
		placeholder = 'Choose…',
		disabled = false,
		title,
		class: className = '',
		style = ''
	}: {
		rows: Row[];
		/** The id picked, or `null` for nothing. */
		value: string | null | undefined;
		onpick: (id: string | null) => void;
		label?: (row: Row) => string;
		/** Something to draw quieter beside the name: "running", a cue's time. */
		hint?: (row: Row) => string;
		/** Group by this rather than by folder: the cues of a show by their sequence. */
		group?: (row: Row) => string;
		/** Offer "nothing" as the first choice, under this name. */
		none?: string;
		/** Choices that are not objects, after all of them: "+ New sequence…". */
		extras?: { id: string; label: string }[];
		/** Said when nothing is picked and there is no `none` to say instead. */
		placeholder?: string;
		disabled?: boolean;
		title?: string;
		class?: string;
		style?: string;
	} = $props();

	const current = $derived(rows.find((row) => row.id === value) ?? null);
	const extra = $derived(extras.find((e) => e.id === value) ?? null);
	const shown = $derived(
		current ? label(current) : extra ? extra.label : value ? 'missing' : (none ?? placeholder)
	);

	let open = $state(false);
	let query = $state('');
	let active = $state(0);
	let button = $state<HTMLButtonElement | null>(null);
	let list = $state<HTMLUListElement | null>(null);
	const shownExtras = $derived(open ? extras.filter((e) => matches(query, e.label, '')) : []);
	let place = $state({ left: 0, top: 0, width: 0, maxHeight: 320, above: false });

	const groups = $derived(
		!open ? [] : group ? pickGroupsBy(rows, query, label, group) : pickGroups(rows, $folders, query, label)
	);
	/** Everything choosable, in the order the arrows walk it. `null` is "nothing". */
	const choices = $derived<(string | null)[]>([
		...(none !== undefined && !query.trim() ? [null] : []),
		...groups.flatMap((group) => group.rows.map((row) => row.id)),
		...shownExtras.map((e) => e.id)
	]);

	async function show(initial = '') {
		if (disabled || !button) return;
		query = initial;
		measure();
		open = true;
		const at = choices.indexOf(value ?? null);
		active = at >= 0 ? at : 0;
		await tick();
		scrollToActive();
	}

	function measure() {
		if (!button) return;
		const r = button.getBoundingClientRect();
		const below = window.innerHeight - r.bottom - 8;
		const above = r.top - 8;
		const flip = below < 220 && above > below;
		place = {
			left: Math.min(r.left, window.innerWidth - Math.max(r.width, 240) - 8),
			top: flip ? r.top - 4 : r.bottom + 4,
			width: Math.max(r.width, 240),
			maxHeight: Math.max(160, Math.min(380, flip ? above : below)),
			above: flip
		};
	}

	function close(refocus = false) {
		open = false;
		query = '';
		if (refocus) button?.focus({ preventScroll: true });
	}

	function take(id: string | null) {
		close(true);
		if (id !== (value ?? null)) onpick(id);
	}

	function scrollToActive() {
		list?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' });
	}

	async function onSearchKey(event: KeyboardEvent) {
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			if (choices.length === 0) return;
			const step = event.key === 'ArrowDown' ? 1 : -1;
			active = (active + step + choices.length) % choices.length;
			await tick();
			scrollToActive();
		} else if (event.key === 'Enter') {
			event.preventDefault();
			if (choices[active] !== undefined) take(choices[active]);
		} else if (event.key === 'Escape') {
			event.preventDefault();
			close(true);
		} else if (event.key === 'Tab') {
			close();
		}
		// Nothing typed here is the canvas's or the page's: not a flow's Backspace, not
		// a cue's Space.
		event.stopPropagation();
	}

	function onButtonKey(event: KeyboardEvent) {
		if (open) return;
		if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			void show();
		} else if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) {
			// Typing on the closed picker starts the search with what was typed.
			event.preventDefault();
			void show(event.key);
		}
	}

	/** Moves the list onto the body, out of every transformed or clipping ancestor. */
	function portal(node: HTMLElement) {
		document.body.appendChild(node);
		return { destroy: () => node.remove() };
	}

	/** A press outside, or a scroll anywhere but the list itself, closes it. */
	function outside(node: HTMLElement) {
		const pressed = (e: PointerEvent) => {
			const target = e.target as Node | null;
			if (target && (node.contains(target) || button?.contains(target))) return;
			close();
		};
		const scrolled = (e: Event) => {
			if (e.target instanceof Node && node.contains(e.target)) return;
			if (!button?.isConnected) return close();
			const r = button.getBoundingClientRect();
			if (r.width === 0 || r.bottom < 0 || r.top > window.innerHeight) return close();
			measure();
		};
		document.addEventListener('pointerdown', pressed, true);
		window.addEventListener('scroll', scrolled, true);
		window.addEventListener('resize', scrolled);
		return {
			destroy() {
				document.removeEventListener('pointerdown', pressed, true);
				window.removeEventListener('scroll', scrolled, true);
				window.removeEventListener('resize', scrolled);
			}
		};
	}

	function focusSearch(node: HTMLInputElement) {
		node.focus();
		node.setSelectionRange(node.value.length, node.value.length);
	}

	const indexOf = (id: string | null) => choices.indexOf(id);
</script>

<button
	bind:this={button}
	type="button"
	class="pick-trigger nodrag nowheel {className}"
	class:empty={!current && !extra}
	class:missing={!!value && !current && !extra}
	{style}
	{disabled}
	title={title ?? (current ? label(current) : undefined)}
	aria-haspopup="listbox"
	aria-expanded={open}
	onclick={() => (open ? close() : show())}
	onkeydown={onButtonKey}
>
	<span class="shown">{shown}</span>
</button>

{#if open}
	<div
		class="pick-popup"
		class:above={place.above}
		style:left="{place.left}px"
		style:top="{place.top}px"
		style:width="{place.width}px"
		style:max-height="{place.maxHeight}px"
		use:portal
		use:outside
	>
		<input
			class="search"
			type="text"
			placeholder="Search…"
			autocomplete="off"
			spellcheck="false"
			bind:value={query}
			oninput={() => (active = 0)}
			onkeydown={onSearchKey}
			use:focusSearch
		/>
		<ul class="choices" role="listbox" bind:this={list}>
			{#if none !== undefined && !query.trim()}
				<li
					role="option"
					class="choice none"
					aria-selected={!value}
					data-active={active === indexOf(null)}
					onpointerenter={() => (active = indexOf(null))}
					onclick={() => take(null)}
					onkeydown={() => {}}
				>
					{none}
				</li>
			{/if}
			{#each groups as group (group.path)}
				{#if group.path}<li class="heading" role="presentation">{group.path}</li>{/if}
				{#each group.rows as row (row.id)}
					<li
						role="option"
						class="choice"
						class:filed={!!group.path}
						aria-selected={row.id === value}
						data-active={active === indexOf(row.id)}
						onpointerenter={() => (active = indexOf(row.id))}
						onclick={() => take(row.id)}
						onkeydown={() => {}}
					>
						<span class="name">{label(row) || '(unnamed)'}</span>
						{#if hint}<span class="hint">{hint(row)}</span>{/if}
					</li>
				{/each}
			{/each}
			{#each shownExtras as e (e.id)}
				<li
					role="option"
					class="choice extra"
					aria-selected={e.id === value}
					data-active={active === indexOf(e.id)}
					onpointerenter={() => (active = indexOf(e.id))}
					onclick={() => take(e.id)}
					onkeydown={() => {}}
				>
					{e.label}
				</li>
			{/each}
			{#if choices.length === 0}
				<li class="nothing" role="presentation">Nothing called that.</li>
			{/if}
		</ul>
	</div>
{/if}

<style>
	/* The closed picker reads as every other select on the console: same box, same
	   arrow (`styles/controls.css`), so a panel cannot tell the two apart by look. */
	.pick-trigger {
		display: inline-flex;
		align-items: center;
		min-height: 28px;
		min-width: 0;
		max-width: 100%;
		padding: 0 24px 0 8px;
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		background-color: var(--bg-sunken);
		background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' fill='none' stroke='%23999' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
		background-repeat: no-repeat;
		background-position: right 8px center;
		background-size: 10px 6px;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		text-align: left;
		cursor: pointer;
	}
	.pick-trigger:focus-visible,
	.pick-trigger[aria-expanded='true'] {
		outline: none;
		border-color: var(--accent);
	}
	.pick-trigger:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.pick-trigger.empty .shown {
		color: var(--text-dim);
	}
	.pick-trigger.missing .shown {
		color: var(--bad);
		font-style: italic;
	}
	.shown {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.pick-popup {
		position: fixed;
		z-index: 2000;
		display: flex;
		flex-direction: column;
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		box-shadow: 0 8px 24px #000a;
		overflow: hidden;
		font-size: var(--font-sm);
		color: var(--text);
	}
	/* Opened above the button: the box grows upwards from its bottom edge. */
	.pick-popup.above {
		transform: translateY(-100%);
	}
	.search {
		flex: none;
		margin: 6px;
		padding: 5px 8px;
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		background: var(--bg-sunken);
		color: var(--text);
		font: inherit;
	}
	.search:focus {
		outline: none;
		border-color: var(--accent);
	}
	.choices {
		list-style: none;
		margin: 0;
		padding: 0 0 6px;
		overflow-y: auto;
		min-height: 0;
	}
	.heading {
		padding: 6px 10px 2px;
		color: var(--text-dim);
		font-size: var(--font-xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
	.choice {
		display: flex;
		align-items: baseline;
		gap: 8px;
		padding: 5px 10px;
		cursor: pointer;
	}
	.choice.filed {
		padding-left: 18px;
	}
	.choice[data-active='true'] {
		background: var(--bg-hover);
	}
	.choice[aria-selected='true'] .name {
		color: var(--accent);
	}
	.choice.extra {
		border-top: 1px solid var(--line);
		color: var(--accent);
	}
	.choice.extra + .choice.extra {
		border-top: none;
	}
	.choice.none {
		color: var(--text-dim);
		font-style: italic;
	}
	.name {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.hint {
		flex: none;
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.nothing {
		padding: 8px 10px;
		color: var(--text-dim);
	}
</style>
