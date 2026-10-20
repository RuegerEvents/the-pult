<script lang="ts">
	/**
	 * One tile: a strip of tabs and whatever the front one draws.
	 *
	 * A tab is its panel's icon and title, and the active one wears its category's
	 * color along its bottom edge — so a tile reads as "playback" or "stage" before a
	 * word of it is read.
	 *
	 * The `+` opens the picker (`PanelPicker.svelte`). A panel already open somewhere
	 * is offered there as "go to" rather than as a second copy — two copies of the 3D
	 * rig would be two scenes rendering the same room.
	 */

	import type { LayoutNode } from '$lib/generated/index.js';
	import { type Path } from '$lib/layout.js';
	import { categoryColor } from '$lib/layout/categories.js';
	import { type PanelMeta } from '$lib/layout/panels.js';
	import {
		beginTabDrag,
		closePanel,
		dragging,
		maximized,
		openPanel
	} from '$lib/stores/layout.js';
	import { allPanels } from '$lib/stores/plugins.js';
	import DropZones from './DropZones.svelte';
	import PanelPicker from './PanelPicker.svelte';
	import Maximize2 from '@lucide/svelte/icons/maximize-2';
	import Minimize2 from '@lucide/svelte/icons/minimize-2';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';

	let { node, path }: { node: Extract<LayoutNode, { type: 'Tabs' }>; path: Path } = $props();

	let adding = $state(false);

	const shown = $derived(node.panels[Math.min(node.active, node.panels.length - 1)] ?? null);
	// The merged registry: built-ins plus whatever the station's plugins offer
	// right now. A panel id nobody recognizes falls through to the message below.
	const meta: PanelMeta | null = $derived(shown ? ($allPanels[shown] ?? null) : null);
	const titleOf = (id: string): string => $allPanels[id]?.title ?? id;

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
					style:--cat={categoryColor($allPanels[panel]?.category)}
					class:on={index === node.active}
					class:ghost={$dragging?.panel === panel}
					onpointerdown={(e) => {
						show(panel);
						beginTabDrag(path, panel, e);
					}}
				>
					{#if $allPanels[panel]}
						{@const Icon = $allPanels[panel].icon}
						<span class="mark" style:color={categoryColor($allPanels[panel].category)}><Icon size={13} /></span>
					{/if}
					<span class="title">{titleOf(panel)}</span>
					<span
						class="close"
						role="button"
						tabindex="-1"
						aria-label="Close {titleOf(panel)}"
						onpointerdown={(e) => {
							e.stopPropagation();
							closePanel(path, panel);
						}}
					><X size={11} /></span>
				</button>
			{/each}
		</div>

		<!-- Outside the strip, so many tabs scroll without taking these with them. -->
		<button class="chip" aria-label="Add a panel" title="Add a panel" onclick={() => (adding = true)}>
			<Plus size={14} />
		</button>

		{#if shown}
			<button
				class="chip"
				title={$maximized === shown ? 'Back to the workspace' : 'Fill the workspace'}
				aria-label="Maximize"
				onclick={() => maximized.set($maximized === shown ? null : shown)}
			>
				{#if $maximized === shown}<Minimize2 size={13} />{:else}<Maximize2 size={13} />{/if}
			</button>
		{/if}
	</div>

	<div class="body" class:fills={meta?.fills}>
		{#if meta}
			{@const Panel = meta.component}
			<Panel {...(meta.props ?? {})} />
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

{#if adding}
	<PanelPicker {path} onclose={() => (adding = false)} />
{/if}

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
		color: var(--text);
	}
	.tab.on {
		color: var(--text-bright);
		background: var(--bg);
		border-bottom-color: var(--cat);
	}
	.mark {
		display: inline-flex;
		opacity: 0.75;
	}
	.tab.on .mark {
		opacity: 1;
	}
	.tab.ghost {
		opacity: 0.4;
	}

	.close {
		display: inline-flex;
		color: transparent;
		border-radius: 3px;
		line-height: 1;
	}
	.tab:hover .close,
	.tab.on .close {
		color: var(--text-faint);
	}
	.close:hover {
		color: var(--bad);
	}

	.chip {
		display: inline-flex;
		align-items: center;
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
	.chip:focus-visible,
	.tab:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
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
