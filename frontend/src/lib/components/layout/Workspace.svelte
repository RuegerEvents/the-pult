<script lang="ts">
	/**
	 * The whole screen: a tree of tiles, or one panel filling it.
	 *
	 * This replaced a fixed sidebar and a row of six tabs, which could never show the
	 * rig, the values and the cue list at once — which is exactly the arrangement
	 * programming needs.
	 */

	import { type PanelMeta } from '$lib/layout/panels.js';
	import { dragging, maximized, tree } from '$lib/stores/layout.js';
	import { allPanels } from '$lib/stores/plugins.js';
	import { categoryColor } from '$lib/layout/categories.js';
	import Tile from './Tile.svelte';

	const big: PanelMeta | null = $derived($maximized ? ($allPanels[$maximized] ?? null) : null);
</script>

<div class="workspace" class:dragging={$dragging}>
	{#if big && $maximized}
		{@const Panel = big.component}
		{@const Icon = big.icon}
		<div class="full">
			<div class="bar">
				<span class="title" style:--cat={categoryColor(big.category)}>
					<Icon size={14} />{big.title}
				</span>
				<span class="spacer"></span>
				<button class="chip" onclick={() => maximized.set(null)}>⤡ Back to the workspace</button>
			</div>
			<div class="body" class:fills={big.fills}><Panel {...(big.props ?? {})} /></div>
		</div>
	{:else}
		<Tile node={$tree} />
	{/if}
</div>

<style>
	.workspace {
		height: 100%;
		min-height: 0;
	}
	/* Nothing else takes the pointer while a tab is in the air, so a drag cannot be
	   swallowed by a fader or a 3D canvas on its way to a drop zone. */
	.workspace.dragging :global(*) {
		user-select: none;
	}

	.full {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 5px 12px;
		background: var(--bg-chrome);
		border-bottom: 1px solid var(--line);
		flex: none;
	}
	.title {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		color: var(--text-bright);
		font-size: var(--font-sm);
	}
	.title :global(svg) {
		color: var(--cat);
	}
	.chip {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.chip:hover {
		color: var(--text-bright);
	}

	.body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}
	.body.fills {
		overflow: hidden;
	}
	.bar .spacer {
		flex: 1;
	}
</style>
