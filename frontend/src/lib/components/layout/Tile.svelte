<script lang="ts">
	/**
	 * One node of the workspace tree, drawn.
	 *
	 * A split lays its children out along one axis with a gutter between each pair; a
	 * tab group is a tile. The component draws itself for the children, which is the
	 * shortest way to say "a tree" in a template.
	 */

	import type { LayoutNode } from '$lib/generated/index.js';
	import type { Path } from '$lib/layout.js';
	import { dragGutter } from '$lib/stores/layout.js';
	import Gutter from './Gutter.svelte';
	import TabGroup from './TabGroup.svelte';
	import Tile from './Tile.svelte';

	let { node, path = [] }: { node: LayoutNode; path?: Path } = $props();

	let width = $state(0);
	let height = $state(0);
	const extent = $derived(node.type === 'Split' && node.direction === 'Row' ? width : height);
</script>

{#if node.type === 'Tabs'}
	<TabGroup {node} {path} />
{:else}
	<div
		class="split {node.direction === 'Row' ? 'row' : 'column'}"
		bind:clientWidth={width}
		bind:clientHeight={height}
	>
		{#each node.children as child, index (index)}
			{#if index > 0}
				<Gutter
					axis={node.direction === 'Row' ? 'x' : 'y'}
					{extent}
					onmove={(delta) => dragGutter(path, index - 1, delta)}
				/>
			{/if}
			<div class="cell" style:flex="{node.sizes[index] ?? 1 / node.children.length} 1 0">
				<Tile node={child} path={[...path, index]} />
			</div>
		{/each}
	</div>
{/if}

<style>
	.split {
		display: flex;
		width: 100%;
		height: 100%;
		min-width: 0;
		min-height: 0;
	}
	.split.row {
		flex-direction: row;
	}
	.split.column {
		flex-direction: column;
	}

	.cell {
		min-width: 0;
		min-height: 0;
		overflow: hidden;
	}
</style>
