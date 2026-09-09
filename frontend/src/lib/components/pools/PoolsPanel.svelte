<script lang="ts">
	/**
	 * The pools: saved groups, and presets.
	 *
	 * Two rows of buttons, which is what every other desk calls direct selects. A group
	 * button recalls a *question* about the rig; a preset button recalls a look. Both
	 * are the show's, so they are here on every console rather than in one browser's
	 * storage.
	 *
	 * Sequences are deliberately not here. A pool of sequences would be an executor
	 * page, which is a bound physical thing and a different piece of work — the
	 * Playback and Cues panels are what runs a sequence today.
	 *
	 * **The group tags are derived and never stored.** A preset that grows a colour is
	 * a colour preset from that moment, which is why the pool is one flat list with a
	 * filter over it rather than five pools somebody has to file things into.
	 *
	 * And a preset button says **how much of it applies**: with a selection it is the
	 * intersection, and "4 of 6" is the difference between a button that will do what
	 * it looks like and one that will not.
	 */

	import type { Preset } from '$lib/generated/index.js';
	import type { FadeGroup } from '$lib/generated/index.js';
	import { GROUP_TAGS, presetGroups, presetReach } from '$lib/programmer.js';
	import { collection } from '$lib/stores/show.js';
	import { entries, recallPreset, storePreset, updatePreset } from '$lib/stores/programmer.js';
	import { viewPreset, presetInView } from '$lib/stores/cues.js';
	import { recall, selection } from '$lib/stores/selection.js';
	import { addToast } from '$lib/toasts.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { focusOnMount } from '$lib/actions.js';

	const data = getDataContext();
	const groups = collection('groups');
	const presets = collection('presets');

	const GROUP_ORDER: FadeGroup[] = ['Intensity', 'Position', 'Color', 'Beam', 'Other'];
	let filter = $state<FadeGroup | null>(null);
	let naming = $state(false);
	let draft = $state('');
	/** The preset whose menu is open — long-press on a tablet, right-click with a mouse. */
	let menuFor = $state<string | null>(null);
	let renaming = $state<string | null>(null);

	const shown = $derived(
		filter === null
			? ($presets as Preset[])
			: ($presets as Preset[]).filter((p) => presetGroups(p).includes(filter as FadeGroup))
	);

	async function makePreset() {
		const name = draft.trim();
		if (!name) return;
		naming = false;
		draft = '';
		try {
			await storePreset(name);
			addToast(`Preset “${name}” stored`, 'success');
		} catch (e) {
			addToast(`${e}`);
		}
	}

	async function apply(preset: Preset) {
		const count = await recallPreset(preset, $selection);
		if (count === 0) addToast(`“${preset.name}” says nothing about what is selected`);
	}

	/** Long-press, which is how a tablet reaches a right-click menu. */
	function longPress(node: HTMLElement, id: string) {
		let timer: ReturnType<typeof setTimeout> | null = null;
		const down = () => {
			timer = setTimeout(() => {
				timer = null;
				menuFor = id;
			}, 500);
		};
		const up = () => {
			if (timer) clearTimeout(timer);
			timer = null;
		};
		node.addEventListener('pointerdown', down);
		node.addEventListener('pointerup', up);
		node.addEventListener('pointercancel', up);
		node.addEventListener('pointerleave', up);
		return {
			destroy() {
				up();
				node.removeEventListener('pointerdown', down);
				node.removeEventListener('pointerup', up);
				node.removeEventListener('pointercancel', up);
				node.removeEventListener('pointerleave', up);
			}
		};
	}
</script>

<div class="pools">
	<section>
		<h2>Groups</h2>
		{#if $groups.length === 0}
			<p class="empty">No saved groups. The Selection panel is where one is made.</p>
		{:else}
			<div class="row">
				{#each $groups as group (group.id)}
					<button class="pad group" onclick={() => recall(group.query)}>{group.name}</button>
				{/each}
			</div>
		{/if}
	</section>

	<section>
		<header>
			<h2>Presets</h2>
			<div class="filter">
				<button class="tag" class:on={filter === null} onclick={() => (filter = null)}>All</button>
				{#each GROUP_ORDER as group (group)}
					<button
						class="tag"
						class:on={filter === group}
						title={group}
						onclick={() => (filter = filter === group ? null : group)}
					>
						{GROUP_TAGS[group]}
					</button>
				{/each}
			</div>
			{#if naming}
				<form
					onsubmit={(e) => {
						e.preventDefault();
						makePreset();
					}}
				>
					<input
						class="text"
						placeholder="Preset name…"
						bind:value={draft}
						use:focusOnMount
						onkeydown={(e) => e.key === 'Escape' && (naming = false)}
					/>
					<button class="chip" type="submit">Store</button>
				</form>
			{:else}
				<button class="chip" disabled={$entries.length === 0} onclick={() => (naming = true)}>
					+ From programmer
				</button>
			{/if}
		</header>

		{#if $presets.length === 0}
			<p class="empty">
				No presets yet. Build a look in the programmer and store it here; a cue that
				references one changes when the preset does.
			</p>
		{:else if shown.length === 0}
			<p class="empty">No preset holds anything in that group.</p>
		{:else}
			<div class="row">
				{#each shown as preset (preset.id)}
					{@const reach = presetReach(preset, $selection)}
					<div class="slot">
						<button
							class="pad"
							class:shown={$presetInView === preset.id}
							class:none={reach.of === 0}
							use:longPress={preset.id}
							oncontextmenu={(e) => {
								e.preventDefault();
								menuFor = preset.id;
							}}
							onclick={() => apply(preset)}
						>
							{#if renaming === preset.id}
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<input
									class="text"
									value={preset.name}
									use:focusOnMount
									onclick={(e) => e.stopPropagation()}
									onkeydown={(e) => e.stopPropagation()}
									onblur={async (e) => {
										const name = e.currentTarget.value.trim();
										renaming = null;
										if (name) await data.presets.byId(preset.id).name.set(name);
									}}
								/>
							{:else}
								<span class="name">{preset.name}</span>
							{/if}
							<span class="tags">
								{#each presetGroups(preset) as group (group)}
									<i title={group}>{GROUP_TAGS[group]}</i>
								{/each}
								{#if $selection.length > 0}
									<em>{reach.of} of {reach.all}</em>
								{/if}
							</span>
						</button>
						{#if menuFor === preset.id}
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<div class="menu" onpointerleave={() => (menuFor = null)}>
								<button
									disabled={$entries.length === 0}
									onclick={async () => {
										menuFor = null;
										const n = await updatePreset(preset);
										addToast(
											n > 0
												? `“${preset.name}” updated — every cue using it followed`
												: 'the programmer is empty'
										);
									}}>Update from programmer</button
								>
								<button
									onclick={() => {
										menuFor = null;
										viewPreset(preset.id);
									}}>Show in the sheet</button
								>
								<button
									onclick={() => {
										menuFor = null;
										renaming = preset.id;
									}}>Rename</button
								>
								<button
									class="danger"
									onclick={async () => {
										menuFor = null;
										await data.presets.byId(preset.id).delete();
									}}>Delete</button
								>
							</div>
						{/if}
					</div>
				{/each}
			</div>
			<p class="note">
				Deleting a preset changes no cue: each one keeps the value it was stored with
				and says the preset is missing.
			</p>
		{/if}
	</section>
</div>

<style>
	.pools {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 10px;
	}

	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	header {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
	}
	header form {
		display: flex;
		gap: 6px;
		align-items: center;
	}

	h2 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}

	.filter {
		display: flex;
		gap: 2px;
		margin-left: auto;
	}
	.tag {
		background: none;
		border: 1px solid transparent;
		border-radius: var(--radius);
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		padding: 2px 7px;
		cursor: pointer;
	}
	.tag:hover {
		color: var(--text);
	}
	.tag.on {
		border-color: var(--accent);
		color: var(--accent);
	}

	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.slot {
		position: relative;
	}

	.pad {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		justify-content: space-between;
		gap: 4px;
		min-width: 7rem;
		min-height: 44px;
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 6px 9px;
		cursor: pointer;
		text-align: left;
	}
	.pad:hover {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.pad.shown {
		border-color: var(--accent);
	}
	/* Nothing selected that this preset knows about: it would do nothing, and a
	   button that looks ready and does nothing is worse than one that says so. */
	.pad.none {
		color: var(--text-faint);
	}
	.group {
		min-height: 34px;
	}

	.name {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		max-width: 12rem;
	}
	.tags {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.tags i {
		font-style: normal;
		font-size: 9px;
		line-height: 1;
		padding: 2px 4px;
		border-radius: 2px;
		background: var(--bg-sunken);
		color: var(--text-dim);
	}
	.tags em {
		font-style: normal;
		font-size: 9px;
		color: var(--text-faint);
	}

	.menu {
		position: absolute;
		top: 100%;
		left: 0;
		z-index: 5;
		display: flex;
		flex-direction: column;
		min-width: 12rem;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		box-shadow: 0 8px 24px rgb(0 0 0 / 50%);
		overflow: hidden;
	}
	.menu button {
		background: none;
		border: none;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		text-align: left;
		padding: 7px 10px;
		cursor: pointer;
	}
	.menu button:hover:not(:disabled) {
		background: var(--bg-hover);
		color: var(--text-bright);
	}
	.menu button:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}
	.menu .danger:hover {
		color: var(--bad);
	}

	.empty,
	.note {
		color: var(--text-faint);
		font-size: var(--font-xs);
		font-style: italic;
		max-width: 60ch;
	}

	.chip {
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 4px 10px;
		cursor: pointer;
	}
	.chip:hover:not(:disabled) {
		color: var(--text-bright);
		border-color: var(--line-input);
	}
	.chip:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}

	.text {
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 3px 6px;
		max-width: 10rem;
	}
</style>
