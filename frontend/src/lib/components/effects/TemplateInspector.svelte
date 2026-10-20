<script lang="ts">
	/**
	 * An effect template: its layers, each a parameter and a shape, and the two acts —
	 * apply it to the selection, or open it in the Effects panel to change it.
	 */

	import type { EffectTemplate } from '$lib/generated/index.js';
	import { applyToSelection, openTemplate } from '$lib/kinds/effects.kind.js';
	import { spreadLabel } from '$lib/effects.js';
	import { displayLabel } from '$lib/patch.js';
	import { revealPanel } from '$lib/stores/layout.js';
	import { selection } from '$lib/stores/selection.js';

	let { rows }: { rows: EffectTemplate[] } = $props();

	const rate = (r: EffectTemplate['layers'][number]['rate']) =>
		'Hz' in r ? `${r.Hz} Hz` : 'from a speed master';
	const shape = (c: EffectTemplate['layers'][number]['curve']) =>
		'Shape' in c ? c.Shape : `${c.Steps.length} steps`;
</script>

{#each rows as template (template.id)}
	<div class="detail">
		{#if rows.length > 1}<h4>{template.name}</h4>{/if}
		<ul>
			{#each template.layers as layer, i (i)}
				<li>
					<span class="param">{displayLabel(layer.kind)}</span>
					<span class="dim">{shape(layer.curve)} · {rate(layer.rate)} · {spreadLabel(layer.spread)}</span>
				</li>
			{/each}
		</ul>
		<div class="row">
			<button class="btn btn-primary" disabled={$selection.length === 0} onclick={() => applyToSelection(template)}>
				Apply to the selection
			</button>
			<button
				class="btn"
				onclick={() => {
					openTemplate.set(template);
					revealPanel('effects');
				}}>Open in Effects</button
			>
		</div>
	</div>
{/each}

<style>
	.detail {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	h4 {
		font-size: var(--font-sm);
	}
	ul {
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: var(--font-sm);
	}
	.param {
		margin-right: 8px;
	}
	.dim {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
</style>
