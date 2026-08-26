<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { delayMsOf } from '$lib/flow.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const data = getDataContext();
	const ms = $derived(delayMsOf(node.kind) ?? 0);
</script>

<NodeShell {node} title="Wait">
	<label class="row">
		<input
			class="text-input nodrag"
			type="number"
			min="0"
			step="100"
			value={ms}
			onchange={(e) =>
				data.flow_nodes.byId(node.id).kind.set({ Delay: { ms: Number(e.currentTarget.value) } })}
		/>
		<span class="unit">ms</span>
	</label>
	{#if node.active}
		<span class="waiting">waiting…</span>
	{/if}
</NodeShell>

<style>
	.row { display: flex; align-items: center; gap: 5px; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 5px; font: inherit; font-size: 12px; width: 100%; }
	.unit { color: #777; font-size: 11px; }
	.waiting { color: #fbbf24; font-size: 10px; }
</style>
