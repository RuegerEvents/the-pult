<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { nodeTag } from '$lib/flow.js';
	import NodeShell from './NodeShell.svelte';
	import { getFlowContext, inputOn } from './context.js';

	const show = getFlowContext();

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const explanation: Record<string, string> = {
		And: 'True only while both are true.',
		Or: 'True while either is true.',
		Not: 'True while the one below is not.',
		Gate: 'Lets the pulse (top) through only while the level (bottom) is true, as it stood before this pulse changed anything.'
	};
	const titles: Record<string, string> = { Gate: 'Only if' };

	/** A gate is open or shut by its level input; the others are true or false. */
	const live = $derived.by(() => {
		if (nodeTag(node.kind) === 'Gate') {
			const open = inputOn(show, node.id, 1);
			return open === null ? null : { text: open ? 'open' : 'shut', on: open };
		}
		return { text: node.active ? 'true' : 'false', on: node.active };
	});
</script>

<NodeShell {node} {live} title={titles[nodeTag(node.kind)] ?? nodeTag(node.kind)}>
	<span class="hint">{explanation[nodeTag(node.kind)] ?? ''}</span>
</NodeShell>

<style>
	.hint { color: #666; font-size: 10px; line-height: 1.3; }
</style>
