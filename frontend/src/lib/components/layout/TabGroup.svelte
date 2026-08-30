<script lang="ts">
	/**
	 * One tile: a strip of tabs and whatever the front one draws.
	 *
	 * The `+` menu offers the panels that are not open anywhere, so opening one twice
	 * is not something the workspace can be talked into — two copies of the 3D rig
	 * would be two scenes rendering the same room.
	 */

	import type { LayoutNode } from '$lib/generated/index.js';
	import { panelsIn, type Path } from '$lib/layout.js';
	import { PANELS, PANEL_IDS, isPanel, panelTitle, type PanelMeta } from '$lib/layout/panels.js';
	import {
		beginTabDrag,
		closePanel,
		dragging,
		maximised,
		openPanel,
		tree
	} from '$lib/stores/layout.js';
	import DropZones from './DropZones.svelte';
	import EditToggle from './EditToggle.svelte';

	let { node, path }: { node: Extract<LayoutNode, { type: 'Tabs' }>; path: Path } = $props();

	let adding = $state(false);

	const shown = $derived(node.panels[Math.min(node.active, node.panels.length - 1)] ?? null);
	// Annotated rather than inferred: `as const satisfies` narrows each entry to its
	// own literal type, so an entry that does not set `editable` has no such property
	// at all and asking about it is an error rather than `undefined`.
	const meta: PanelMeta | null = $derived(shown && isPanel(shown) ? PANELS[shown] : null);
	const spare = $derived(PANEL_IDS.filter((id) => !panelsIn($tree).includes(id)));

	function show(panel: string) {
		const at = node.panels.indexOf(panel);
		if (at >= 0 && at !== node.active) openPanel(path, panel);
	}
</script>

<div class="tile">
	<div class="chrome">
		<div class="strip">
			{#each node.panels as panel, index (panel)}
				<button
					class="tab"
					class:on={index === node.active}
					class:ghost={$dragging?.panel === panel}
					onpointerdown={(e) => {
						show(panel);
						beginTabDrag(path, panel, e);
					}}
				>
					{panelTitle(panel)}
					<span
						class="close"
						role="button"
						tabindex="-1"
						aria-label="Close {panelTitle(panel)}"
						onpointerdown={(e) => {
							e.stopPropagation();
							closePanel(path, panel);
						}}
					>✕</span>
				</button>
			{/each}
		</div>

		<!-- Outside the strip, so many tabs scroll without taking these with them —
		     and so the menu below is not clipped by the strip's own scrolling. -->
		<div class="add">
			<button class="chip" aria-label="Add a panel" onclick={() => (adding = !adding)}>+</button>
			{#if adding}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div class="menu" onpointerleave={() => (adding = false)}>
					{#if spare.length === 0}
						<span class="none">Every panel is already open.</span>
					{:else}
						{#each spare as id (id)}
							<button
								onclick={() => {
									openPanel(path, id);
									adding = false;
								}}
							>{PANELS[id].title}</button>
						{/each}
					{/if}
				</div>
			{/if}
		</div>

		<!-- Before the maximise chip, so the toggle sits in the same place whether or
		     not a tile can be maximised, and never moves under a thumb. -->
		{#if shown && meta?.editable}
			<EditToggle panel={shown} />
		{/if}

		{#if shown}
			<button
				class="chip"
				title={$maximised === shown ? 'Back to the workspace' : 'Fill the workspace'}
				aria-label="Maximise"
				onclick={() => maximised.set($maximised === shown ? null : shown)}
			>{$maximised === shown ? '⤡' : '⤢'}</button>
		{/if}
	</div>

	<div class="body" class:fills={meta?.fills}>
		{#if meta}
			{@const Panel = meta.component}
			<Panel />
		{:else if shown}
			<p class="unknown">This console has no panel called “{shown}”.</p>
		{:else}
			<p class="unknown">Nothing here. Add a panel with the + above.</p>
		{/if}
	</div>

	{#if $dragging}
		<DropZones {path} />
	{/if}
</div>

<style>
	.tile {
		position: relative;
		display: flex;
		flex-direction: column;
		min-width: 0;
		min-height: 0;
		height: 100%;
		background: var(--bg);
	}

	.chrome {
		display: flex;
		align-items: stretch;
		flex: none;
		background: var(--bg-chrome);
		border-bottom: 1px solid var(--line);
	}

	.strip {
		display: flex;
		align-items: stretch;
		gap: 1px;
		flex: 1;
		min-width: 0;
		overflow-x: auto;
		scrollbar-width: none;
	}

	.tab {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 5px 8px 5px 10px;
		border: none;
		border-bottom: 2px solid transparent;
		background: none;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-sm);
		white-space: nowrap;
		cursor: grab;
		touch-action: none;
	}
	.tab:hover {
		color: #ddd;
	}
	.tab.on {
		color: var(--text-bright);
		background: var(--bg);
		border-bottom-color: var(--accent-solid);
	}
	.tab.ghost {
		opacity: 0.4;
	}

	.close {
		color: transparent;
		font-size: 9px;
		line-height: 1;
	}
	.tab:hover .close,
	.tab.on .close {
		color: var(--text-faint);
	}
	.close:hover {
		color: var(--bad);
	}

	.add {
		position: relative;
		display: flex;
	}

	.chip {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-sm);
		padding: 4px 9px;
		cursor: pointer;
	}
	.chip:hover {
		color: var(--text-bright);
	}

	.menu {
		position: absolute;
		top: 100%;
		/* Opening leftwards: the + sits at the right edge of the tile, and a menu
		   hanging off it would be cut off by the tile next door. */
		right: 0;
		z-index: 30;
		display: flex;
		flex-direction: column;
		min-width: 132px;
		padding: 3px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		box-shadow: 0 6px 18px #0008;
	}
	.menu button {
		text-align: left;
		background: none;
		border: none;
		border-radius: 3px;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 4px 8px;
		cursor: pointer;
	}
	.menu button:hover {
		background: var(--bg-hover);
	}
	.none {
		color: var(--text-faint);
		font-size: var(--font-xs);
		padding: 4px 8px;
	}

	.body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}
	.body.fills {
		overflow: hidden;
	}

	.unknown {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 14px;
	}
</style>
