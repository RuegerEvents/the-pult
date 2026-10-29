<script lang="ts">
	/**
	 * The workspace on a phone: the same layout, one panel at a time.
	 *
	 * A desk's tiles divided across 390 pixels are panels nobody can use, so a narrow
	 * screen reads the layout as a list — every panel in it, in the tree's order — and
	 * shows the one picked under a strip of tabs above it, the way a tile does. Nothing
	 * is written: the layout is the show's and is the same tiles again on a wider
	 * screen, and opening a panel here adds it to the tree exactly as the desk's `+`
	 * does.
	 */

	import { categoryColor } from '$lib/layout/categories.js';
	import { findPanel, panelsIn, panelsShown } from '$lib/layout.js';
	import { closePanel, firstTile, tree } from '$lib/stores/layout.js';
	import { allPanels } from '$lib/stores/plugins.js';
	import { compactPanel } from '$lib/stores/viewport.js';
	import PanelPicker from './PanelPicker.svelte';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';

	let adding = $state(false);

	const panels = $derived(panelsIn($tree));
	// What a phone opens on is what the desk had in front: the first tile's front tab.
	const shown = $derived(
		$compactPanel && panels.includes($compactPanel) ? $compactPanel : (panelsShown($tree)[0] ?? panels[0] ?? null)
	);
	const meta = $derived(shown ? ($allPanels[shown] ?? null) : null);
	const titleOf = (id: string): string => $allPanels[id]?.title ?? id;

	function close(panel: string) {
		const path = findPanel($tree, panel);
		if (path) closePanel(path, panel);
		if ($compactPanel === panel) compactPanel.set(null);
	}

	/** Keep the tab being looked at in sight when the strip is longer than the screen. */
	function inView(node: HTMLElement, on: boolean) {
		const reveal = (yes: boolean) => yes && node.scrollIntoView({ block: 'nearest', inline: 'nearest' });
		reveal(on);
		return { update: reveal };
	}
</script>

<div class="compact">
	<nav class="strip" aria-label="Panels">
		{#each panels as panel (panel)}
			{@const Icon = $allPanels[panel]?.icon}
			<button
				class="tab"
				class:on={panel === shown}
				style:--cat={categoryColor($allPanels[panel]?.category)}
				use:inView={panel === shown}
				onclick={() => compactPanel.set(panel)}
			>
				{#if Icon}<span class="mark"><Icon size={15} /></span>{/if}
				<span class="title">{titleOf(panel)}</span>
				{#if panel === shown}
					<span
						class="close"
						role="button"
						tabindex="-1"
						aria-label="Close {titleOf(panel)}"
						onclick={(e) => {
							e.stopPropagation();
							close(panel);
						}}
						onkeydown={() => {}}
					><X size={13} /></span>
				{/if}
			</button>
		{/each}
		<button class="tab add" aria-label="Add a panel" onclick={() => (adding = true)}><Plus size={16} /></button>
	</nav>
	<div class="body" class:fills={meta?.fills}>
		{#if meta}
			{@const Panel = meta.component}
			{#key shown}<Panel {...(meta.props ?? {})} />{/key}
		{:else if shown}
			<p class="unknown">This console has no panel called “{shown}”.</p>
		{:else}
			<p class="unknown">Nothing here. Add a panel with the + above.</p>
		{/if}
	</div>

</div>

{#if adding}
	<PanelPicker path={firstTile($tree)} onclose={() => (adding = false)} />
{/if}

<style>
	.compact {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
	.body {
		flex: 1;
		min-height: 0;
		overflow: auto;
		padding-bottom: env(safe-area-inset-bottom);
	}
	.body.fills {
		overflow: hidden;
	}

	.strip {
		display: flex;
		flex: none;
		overflow-x: auto;
		scrollbar-width: none;
		background: var(--bg-chrome);
		border-bottom: 1px solid var(--line-strong);
	}
	.tab {
		display: flex;
		align-items: center;
		gap: 6px;
		flex: none;
		min-height: 40px;
		padding: 0 12px;
		border: none;
		border-bottom: 2px solid transparent;
		background: none;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-sm);
		white-space: nowrap;
		touch-action: manipulation;
		-webkit-tap-highlight-color: transparent;
	}
	.tab.on {
		color: var(--text-bright);
		background: var(--bg);
		border-bottom-color: var(--cat);
	}
	.mark {
		display: inline-flex;
		color: var(--cat);
	}
	.close {
		display: inline-flex;
		padding: 4px;
		margin-right: -6px;
		color: var(--text-faint);
	}
	.add {
		color: var(--text-faint);
	}

	.unknown {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 14px;
	}
</style>
