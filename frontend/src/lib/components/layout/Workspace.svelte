<script lang="ts">
	/**
	 * The whole screen: a tree of tiles, or one panel filling it.
	 *
	 * This replaced a fixed sidebar and a row of six tabs, which could never show the
	 * rig, the values and the cue list at once — which is exactly the arrangement
	 * programming needs.
	 */

	import { type PanelMeta } from '$lib/layout/panels.js';
	import { dragging, maximised, tree } from '$lib/stores/layout.js';
	import { allPanels } from '$lib/stores/plugins.js';
	import EditToggle from './EditToggle.svelte';
	import Tile from './Tile.svelte';

	const big: PanelMeta | null = $derived($maximised ? ($allPanels[$maximised] ?? null) : null);
</script>

<div class="workspace" class:dragging={$dragging}>
	{#if big && $maximised}
		{@const Panel = big.component}
		<div class="full">
			<div class="bar">
				<span class="title">{big.title}</span>
				<span class="spacer"></span>
				<!-- The maximised view draws its own chrome, and left this out. Filling
				     the screen with a panel is exactly when there is room to edit it,
				     so leaving the toggle behind in the tile made maximising the one
				     place the lock could not be undone. -->
				{#if big.editable}
					<EditToggle panel={$maximised} />
				{/if}
				<button class="chip" onclick={() => maximised.set(null)}>⤡ Back to the workspace</button>
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
		color: var(--text-bright);
		font-size: var(--font-sm);
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
