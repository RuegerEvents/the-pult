<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { PultWsClient } from '$lib/ws/client.js';
	import { backendOrigin, fetchConfig, wsUrl, type BackendConfig } from '$lib/ws/endpoint.js';
	import { createRootProxy } from '$lib/ws/data.js';
	import { setClientContext, setDataContext } from '$lib/ws/context.js';
	import ConnectionStatus from '$lib/components/ConnectionStatus.svelte';
	import ConnectingOverlay from '$lib/components/ConnectingOverlay.svelte';
	import Toasts from '$lib/components/Toasts.svelte';
	import { addToast } from '$lib/toasts.js';
	import { initShowStores } from '$lib/stores/show.js';
	import { identifyOnConnect } from '$lib/stores/user.js';
	import { isTextField, redo, shortcutFor, undo } from '$lib/stores/undo.js';
	import { restoreLayout } from '$lib/stores/layout.js';
	import LayoutBar from '$lib/components/layout/LayoutBar.svelte';
	import UserBar from '$lib/components/UserBar.svelte';
	import '$lib/styles/tokens.css';
	import '$lib/styles/controls.css';

	let { children } = $props();

	// The backend serves this page, so where it is is where we came from. `?port=`
	// still names a second station on the same host, which is what demo.sh --two
	// prints and what a dev server pointed at one console needs to reach another.
	const client = new PultWsClient(
		browser ? wsUrl(window.location) : 'ws://localhost:7700/ws'
	);
	const data = createRootProxy(client);

	setClientContext(client);
	setDataContext(data);
	// One store per collection, shared by every panel: a tiled workspace can have the
	// same fixtures on screen four times, and four deep subscriptions to them is
	// four copies of every update forty times a second.
	initShowStores(data, client);
	// Which tiles this browser had up last time. The layouts themselves are the
	// show's; which one is on screen is this operator's.
	restoreLayout();
	// And who this browser is, so its writes can be taken back. Said again on every
	// reconnect by the client itself — a socket that came back anonymous would keep
	// working and quietly stop being undoable.
	identifyOnConnect();

	/**
	 * Ctrl-Z anywhere that is not a text field.
	 *
	 * On the window rather than on a panel, because undo is not any one panel's: an
	 * operator who has just deleted a fixture in Patch and moved to the Plan still
	 * means that fixture.
	 */
	function onKey(event: KeyboardEvent) {
		const action = shortcutFor(event, isTextField(event.target));
		if (!action) return;
		event.preventDefault();
		if (action === 'undo') undo();
		else redo();
	}

	let connected = $state(false);
	/// What the station says it is. Nothing waits on it — it is the version in the
	/// corner, not the socket — so a console still opens if it never arrives.
	let station = $state<BackendConfig | null>(null);
	/// Whether this browser has ever had the console, which decides what the cover
	/// says: a first connection is being made, a later one has been lost.
	let everConnected = $state(false);
	/// Whether to cover the workspace. Starts covered, because until the socket opens
	/// there is nothing behind it to look at — and it is not simply "disconnected"
	/// afterwards, since a reconnect takes a moment and flashing a full-screen panel
	/// over a blip is its own kind of confusion.
	let covering = $state(true);

	const address = $derived.by(() => {
		try {
			return new URL(client.url).host;
		} catch {
			return client.url;
		}
	});

	$effect(() => {
		if (connected) {
			covering = false;
			return;
		}
		if (!everConnected) {
			covering = true;
			return;
		}
		const settle = setTimeout(() => (covering = true), 600);
		return () => clearTimeout(settle);
	});

	onMount(() => {
		fetchConfig(backendOrigin(window.location)).then((config) => (station = config));
		client.onConnect = () => { connected = true; everConnected = true; };
		client.onDisconnect = () => { connected = false; };
		client.onError = (msg) => addToast(msg);
		client.connect();
		return () => client.disconnect();
	});
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
	<header class="topbar">
		<span class="brand" title={station ? `station ${station.nodeId}` : address}>
			the-pult{#if station}<span class="version">{station.version}</span>{/if}
		</span>
		<LayoutBar />
		<span class="spacer"></span>
		<UserBar />
		<ConnectionStatus {connected} />
	</header>
	<main>
		{@render children()}
	</main>
</div>

{#if covering}
	<ConnectingOverlay {everConnected} {address} onretry={() => client.retryNow()} />
{/if}

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
		white-space: nowrap;
	}

	.version {
		margin-left: 6px;
		font-weight: 400;
		font-size: 0.7rem;
		letter-spacing: 0;
		color: #666;
	}

	main {
		flex: 1;
		min-height: 0;
		overflow: hidden;
	}
</style>
