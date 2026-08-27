<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { PultWsClient } from '$lib/ws/client.js';
	import { createRootProxy } from '$lib/ws/data.js';
	import { setClientContext, setDataContext } from '$lib/ws/context.js';
	import ConnectionStatus from '$lib/components/ConnectionStatus.svelte';
	import Toasts from '$lib/components/Toasts.svelte';
	import { addToast } from '$lib/toasts.js';
	import { initShowStores } from '$lib/stores/show.js';
	import { restoreLayout } from '$lib/stores/layout.js';
	import LayoutBar from '$lib/components/layout/LayoutBar.svelte';
	import '$lib/styles/tokens.css';

	let { children } = $props();

	const wsPort = browser
		? (new URLSearchParams(window.location.search).get('port') ?? '7700')
		: '7700';
	const client = new PultWsClient(`ws://localhost:${wsPort}/ws`);
	const data = createRootProxy(client);

	setClientContext(client);
	setDataContext(data);
	// One store per collection, shared by every panel: a tiled workspace can have the
	// same fixtures on screen four times, and four deep subscriptions to them is
	// four copies of every update forty times a second.
	initShowStores(data);
	// Which tiles this browser had up last time. The layouts themselves are the
	// show's; which one is on screen is this operator's.
	restoreLayout();

	let connected = $state(false);

	onMount(() => {
		client.onConnect = () => { connected = true; };
		client.onDisconnect = () => { connected = false; };
		client.onError = (msg) => addToast(msg);
		client.connect();
		return () => client.disconnect();
	});
</script>

<div class="shell">
	<header class="topbar">
		<span class="brand">the-pult</span>
		<LayoutBar />
		<span class="spacer"></span>
		<ConnectionStatus {connected} />
	</header>
	<main>
		{@render children()}
	</main>
</div>

<Toasts />

<style>
	:global(*, *::before, *::after) {
		box-sizing: border-box;
		margin: 0;
		padding: 0;
	}

	:global(body) {
		background: #1a1a1a;
		color: #e0e0e0;
		font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
		font-size: 14px;
	}

	.shell {
		display: flex;
		flex-direction: column;
		height: 100dvh;
	}

	.topbar {
		display: flex;
		align-items: center;
		gap: 16px;
		padding: 0 16px;
		height: 40px;
		background: #111;
		border-bottom: 1px solid #333;
		flex-shrink: 0;
	}

	.spacer {
		flex: 1;
	}

	.brand {
		font-weight: 600;
		letter-spacing: 0.05em;
		color: #fff;
	}

	main {
		flex: 1;
		min-height: 0;
		overflow: hidden;
	}
</style>
