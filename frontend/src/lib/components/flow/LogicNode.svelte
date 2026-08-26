<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { nodeTag } from '$lib/flow.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const explanation: Record<string, string> = {
		And: 'True only while both are true.',
		Or: 'True while either is true.',
		Not: 'True while the one below is not.'
	};
</script>

<NodeShell {node} title={nodeTag(node.kind)}>
	<span class="hint">{explanation[nodeTag(node.kind)] ?? ''}</span>
</NodeShell>

<style>
	.hint { color: #666; font-size: 10px; line-height: 1.3; }
</style>
