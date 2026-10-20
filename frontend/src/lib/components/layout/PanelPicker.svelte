<script lang="ts">
	/**
	 * Add a panel: every panel the console has, in its category's column.
	 *
	 * It replaced a plain dropdown of thirty-nine names. The columns are what an
	 * operator scans by — "something about playback" is a green column, not a word —
	 * and the search is what they type when they know the name. Open panels stay in
	 * the list as "go to", so this is also where a panel buried in a tab gets found.
	 *
	 * Keyboard first: the search has the focus, the arrows walk the entries in reading
	 * order, Enter takes the highlighted one.
	 */

	import Dialog from '$lib/components/Dialog.svelte';
	import { focusOnMount } from '$lib/actions.js';
	import { CATEGORIES } from '$lib/layout/categories.js';
	import { flatEntries, pickerColumns, type PickerEntry } from '$lib/layout/picker.js';
	import type { Path } from '$lib/layout.js';
	import { panelsIn } from '$lib/layout.js';
	import { openPanel, revealPanel, tree } from '$lib/stores/layout.js';
	import { allPanels } from '$lib/stores/plugins.js';
	import { openSetup } from '$lib/stores/setup.js';

	let { path, onclose }: { path: Path; onclose: () => void } = $props();

	let query = $state('');
	let cursor = $state(0);

	const open = $derived(new Set(panelsIn($tree)));
	const columns = $derived(pickerColumns($allPanels, open, query));
	const flat = $derived(flatEntries(columns));

	$effect(() => {
		// A new query starts at its best match.
		void query;
		cursor = 0;
	});

	function choose(entry: PickerEntry) {
		if (entry.action === 'setup') openSetup(entry.id);
		else if (entry.action === 'goto') revealPanel(entry.id);
		else openPanel(path, entry.id);
		onclose();
	}

	function onkey(event: KeyboardEvent) {
		if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
			event.preventDefault();
			cursor = Math.min(flat.length - 1, cursor + 1);
		} else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
			event.preventDefault();
			cursor = Math.max(0, cursor - 1);
		} else if (event.key === 'Enter') {
			event.preventDefault();
			const entry = flat[cursor];
			if (entry) choose(entry);
		}
	}

	const label = (action: PickerEntry['action']) =>
		action === 'goto' ? 'open — go to' : action === 'setup' ? 'in Setup' : '';
</script>

<Dialog title="Add a panel" size="wide" {onclose}>
	{#snippet actions()}
		<input
			class="input search"
			placeholder="Search panels…"
			aria-label="Search panels"
			bind:value={query}
			use:focusOnMount
			onkeydown={onkey}
		/>
	{/snippet}
	<div class="columns">
		{#each columns as column (column.category)}
			<section style:--cat={CATEGORIES[column.category].color}>
				<h3>{CATEGORIES[column.category].label}</h3>
				{#each column.entries as entry (entry.id)}
					{@const meta = $allPanels[entry.id]}
					{@const Icon = meta.icon}
					<button
						class="entry"
						class:on={flat[cursor] === entry}
						class:open={entry.action === 'goto'}
						onclick={() => choose(entry)}
						onpointerenter={() => (cursor = flat.indexOf(entry))}
					>
						<span class="mark"><Icon size={15} /></span>
						<span class="name">{entry.title}</span>
						{#if label(entry.action)}<span class="tag">{label(entry.action)}</span>{/if}
					</button>
				{/each}
			</section>
		{:else}
			<p class="none">No panel is called anything like “{query}”.</p>
		{/each}
	</div>
</Dialog>

<style>
	.search {
		width: 220px;
	}

	.columns {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
		gap: 14px 18px;
		padding: 14px;
	}

	section {
		display: flex;
		flex-direction: column;
		gap: 1px;
	}

	h3 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--cat);
		padding: 0 8px 6px;
		border-bottom: 2px solid color-mix(in srgb, var(--cat) 35%, transparent);
		margin-bottom: 4px;
	}

	.entry {
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 34px;
		background: none;
		border: 1px solid transparent;
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		text-align: left;
		padding: 4px 8px;
		cursor: pointer;
	}
	.entry.on {
		background: var(--bg-hover);
		border-color: var(--line-strong);
		color: var(--text-bright);
	}
	.entry:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}
	.entry.open .name {
		color: var(--text-dim);
	}

	.mark {
		display: grid;
		place-items: center;
		width: 24px;
		height: 24px;
		flex: none;
		border-radius: 5px;
		color: var(--cat);
		background: color-mix(in srgb, var(--cat) 14%, transparent);
	}

	.name {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.tag {
		font-size: 10px;
		color: var(--text-faint);
		white-space: nowrap;
	}

	.none {
		color: var(--text-faint);
		font-style: italic;
		font-size: var(--font-sm);
	}
</style>
