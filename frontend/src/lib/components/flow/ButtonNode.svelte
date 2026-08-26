<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const data = getDataContext();
</script>

<NodeShell {node} title="Button">
	<button class="press nodrag" onclick={() => data.flow_nodes.byId(node.id).press()}>Press</button>
	<span class="hint">A press fires whatever this is wired to, from any console.</span>
</NodeShell>

<style>
	.press { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; font-size: 12px; cursor: pointer; }
	.press:active { background: #4a9eff; }
	.hint { color: #666; font-size: 10px; line-height: 1.3; }
</style>
