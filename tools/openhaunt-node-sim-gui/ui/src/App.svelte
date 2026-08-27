<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';
	import Config from './lib/Config.svelte';
	import Header from './lib/Header.svelte';
	import Inputs from './lib/Inputs.svelte';
	import Outputs from './lib/Outputs.svelte';
	import Sacn from './lib/Sacn.svelte';
	import type { Frame, Snapshot } from './lib/node.js';

	let node = $state<Snapshot | null>(null);
	let frames = $state<Frame[]>([]);

	$effect(() => {
		// Asked for once as well as listened for: a node nothing has happened to yet
		// still has a state worth drawing, and it may have reached it before this
		// window existed.
		invoke<Snapshot>('snapshot').then((snapshot) => (node ??= snapshot));

		const stopped = [
			listen<Snapshot>('sim://state', (event) => (node = event.payload)),
			listen<Frame>('sim://sacn', (event) => {
				const rest = frames.filter((f) => f.universe !== event.payload.universe);
				frames = [...rest, event.payload].sort((a, b) => a.universe - b.universe);
			})
		];
		return () => stopped.forEach((s) => s.then((stop) => stop()));
	});
</script>

{#if node}
	<Header {node} />
	<Config {node} />
	<Inputs
		{node}
		oncontact={(port, state) => invoke('contact', { port, state })}
		onreading={(port, value) => invoke('reading', { port, value })}
	/>
	<Outputs {node} />
	{#if node.sacnAddr}
		<Sacn {frames} />
	{/if}
{:else}
	<p class="starting">Starting the node…</p>
{/if}

<style>
	.starting {
		padding: 20px;
		color: var(--dim);
	}
</style>
