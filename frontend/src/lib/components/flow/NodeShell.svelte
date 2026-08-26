<script lang="ts">
	import { Handle, Position } from '@xyflow/svelte';
	import type { Snippet } from 'svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { inputPorts, nodeCategory, outputPorts } from '$lib/flow.js';

	let {
		node,
		title,
		children
	}: { node: FlowNode; title: string; children?: Snippet } = $props();

	const data = getDataContext();
	const ins = $derived(inputPorts(node.kind));
	const outs = $derived(outputPorts(node.kind));
	const category = $derived(nodeCategory(node.kind));
</script>

<div class="node {category}" class:lit={node.active}>
	{#each ins as port, i (i)}
		<Handle
			type="target"
			position={Position.Left}
			id={`in-${i}`}
			class={port.toLowerCase()}
			style={`top: ${28 + i * 18}px`}
		/>
	{/each}

	<header>
		<span class="title">{title}</span>
		<button
			class="remove"
			title="Delete node"
			onclick={() => data.flow_nodes.byId(node.id).delete()}>×</button
		>
	</header>

	{#if children}
		<div class="body">{@render children()}</div>
	{/if}

	{#each outs as port, i (i)}
		<Handle
			type="source"
			position={Position.Right}
			id={`out-${i}`}
			class={port.toLowerCase()}
			style={`top: ${28 + i * 18}px`}
		/>
	{/each}
</div>

<style>
	.node {
		/* Fixed rather than shrink-to-fit: a graph whose boxes are all one width
		   reads as a diagram, and the layout can then space them without guessing. */
		width: 184px;
		background: #252525;
		border: 1px solid #3a3a3a;
		border-left-width: 3px;
		border-radius: 4px;
		color: #e0e0e0;
		font-size: 12px;
	}
	/* One colour per family, so the shape of a graph reads before the words do. */
	.node.source { border-left-color: #4a9eff; }
	.node.logic { border-left-color: #a78bfa; }
	.node.timing { border-left-color: #fbbf24; }
	.node.action { border-left-color: #22c55e; }

	/* What makes the graph an instrument rather than a diagram: a signal passing
	   through lights the node it passed through. */
	.node.lit { border-color: #4a9eff; box-shadow: 0 0 0 1px #4a9eff55, 0 0 12px #4a9eff33; }

	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 6px;
		padding: 5px 4px 5px 9px;
		border-bottom: 1px solid #2e2e2e;
	}
	.title { font-weight: 600; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: #999; }
	.node.lit .title { color: #cfe3ff; }
	.remove { background: none; border: none; color: #666; font-size: 15px; line-height: 1; padding: 0 4px; cursor: pointer; }
	.remove:hover { color: #e05555; }
	.body { padding: 7px 9px 8px; display: flex; flex-direction: column; gap: 5px; }

	/* A level handle is round, a pulse handle is square: the two never join, and
	   the shape says so before the drag is refused. */
	.node :global(.svelte-flow__handle) { width: 9px; height: 9px; border: 1px solid #1a1a1a; }
	.node :global(.svelte-flow__handle.level) { background: #a78bfa; border-radius: 50%; }
	.node :global(.svelte-flow__handle.pulse) { background: #fbbf24; border-radius: 1px; }
</style>
