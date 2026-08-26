<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode, TriggerCondition } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { conditionFrom, conditionOf, conditionTag, thresholdOf } from '$lib/flow.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const data = getDataContext();
	const condition = $derived(conditionOf(node.kind) ?? 'RisingEdge');
	const tag = $derived(conditionTag(condition));

	const set = (next: TriggerCondition) => data.flow_nodes.byId(node.id).kind.set({ Condition: next });
</script>

<NodeShell {node} title="When it">
	<select
		class="text-input nodrag"
		value={tag}
		onchange={(e) => set(conditionFrom(e.currentTarget.value, thresholdOf(condition)))}
	>
		<option value="RisingEdge">closes / rises</option>
		<option value="FallingEdge">opens / falls</option>
		<option value="AnyChange">changes at all</option>
		<option value="Above">rises above</option>
		<option value="Below">falls below</option>
	</select>
	{#if tag === 'Above' || tag === 'Below'}
		<input
			class="text-input nodrag"
			type="number"
			step="0.1"
			value={thresholdOf(condition)}
			onchange={(e) => set(conditionFrom(tag, Number(e.currentTarget.value)))}
		/>
	{/if}
	<span class="hint">Fires on the change, not on the level.</span>
</NodeShell>

<style>
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 5px; font: inherit; font-size: 12px; width: 100%; }
	.hint { color: #666; font-size: 10px; line-height: 1.3; }
</style>
