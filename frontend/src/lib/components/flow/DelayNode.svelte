<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { delayMsOf } from '$lib/flow.js';
	import { consoleNow } from '$lib/ws/clock.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const data = getDataContext();
	const ms = $derived(delayMsOf(node.kind) ?? 0);

	/**
	 * What is left, in ms, redrawn per animation frame while it counts and not at all
	 * otherwise. Worked out from the deadline the leader published against this page's
	 * station clock; `null` — and no figure — until the page has one, or when the
	 * node is not counting.
	 */
	let left = $state<number | null>(null);
	$effect(() => {
		const due = node.due_at ?? null;
		if (due === null) {
			left = null;
			return;
		}
		let frame = 0;
		const draw = () => {
			const now = consoleNow();
			left = now === null ? null : Math.max(0, due - now);
			frame = requestAnimationFrame(draw);
		};
		draw();
		return () => cancelAnimationFrame(frame);
	});
	const fraction = $derived(left === null || ms <= 0 ? 0 : Math.min(1, left / ms));
	const seconds = (ms: number) => (ms >= 10_000 ? `${Math.ceil(ms / 1000)} s` : `${(ms / 1000).toFixed(1)} s`);
</script>

<NodeShell {node} title="Wait" live={node.active ? { text: 'counting', on: true } : null}>
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
	{#if left !== null}
		<div class="left">
			<div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax={ms} aria-valuenow={left}>
				<div class="fill" style:width="{fraction * 100}%"></div>
			</div>
			<span class="time">{seconds(left)} left</span>
		</div>
	{:else if node.active}
		<span class="waiting">waiting…</span>
	{/if}
	<!-- The two inputs, top to bottom: start (again from the top if it is already
	     counting), and reset, which stops it with nothing coming out. -->
	<span class="ports">in: start, then reset</span>
</NodeShell>

<style>
	.row { display: flex; align-items: center; gap: 5px; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 5px; font: inherit; font-size: 12px; width: 100%; }
	.unit { color: #777; font-size: 11px; }
	.waiting { color: #fbbf24; font-size: 10px; }
	.left { display: flex; align-items: center; gap: 6px; }
	.bar { flex: 1; height: 6px; border-radius: 3px; background: #171717; border: 1px solid #3a3a3a; overflow: hidden; }
	.fill { height: 100%; background: #fbbf24; }
	.time { color: #fbbf24; font-size: 10px; font-variant-numeric: tabular-nums; white-space: nowrap; }
	.ports { display: block; color: #777; font-size: 10px; }
</style>
