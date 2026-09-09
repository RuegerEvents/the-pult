<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { PultWsClient } from '$lib/ws/client.js';
	import { backendOrigin, wsUrl } from '$lib/ws/endpoint.js';
	import { createRootProxy } from '$lib/ws/data.js';
	import { setClientContext, setDataContext, setStationContext } from '$lib/ws/context.js';
	import { watchStation } from '$lib/stores/station.js';
	import ShowMenu from '$lib/components/ShowMenu.svelte';
	import ConnectionStatus from '$lib/components/ConnectionStatus.svelte';
	import ConnectingOverlay from '$lib/components/ConnectingOverlay.svelte';
	import Toasts from '$lib/components/Toasts.svelte';
	import DeletePrompt from '$lib/components/stage/DeletePrompt.svelte';
	import { answerDelete, deleteAsk } from '$lib/stores/editor.js';
	import { addToast } from '$lib/toasts.js';
	import { initShowStores } from '$lib/stores/show.js';
	import { identifyOnConnect } from '$lib/stores/user.js';
	import { isTextField, redo, shortcutFor, undo } from '$lib/stores/undo.js';
	import { focusConsole } from '$lib/stores/plugins.js';
	import { restoreLayout } from '$lib/stores/layout.js';
	import { reportBrowserErrors } from '$lib/errors.js';
	import { reportBrowserStats } from '$lib/stats.js';
	import { frameMeter } from '$lib/stores/output.js';
	import { beginSwitch, endSwitch, switching } from '$lib/stores/switching.js';
	import { switchFromClose } from '$lib/switching.js';
	import LayoutBar from '$lib/components/layout/LayoutBar.svelte';
	import Setup from '$lib/components/setup/Setup.svelte';
	import { closeSetup, setupSection, toggleSetup } from '$lib/stores/setup.js';
	import Verbs from '$lib/components/programmer/Verbs.svelte';
	import { focusedCueSheet } from '$lib/stores/cues.js';
	import { clear } from '$lib/stores/programmer.js';
	import { clearSelection } from '$lib/stores/selection.js';
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
	// What the station says it is, asked again on every reconnect — because opening a
	// show is this station stopping and another starting in its place, and the
	// reconnect is the first moment there is anybody to ask. A tab that comes back to
	// a different show reloads: see `$lib/shows.ts`.
	// And a fresh answer that keeps this tab where it is — the same show, or the
	// reloaded page hearing the new one — is the moment a switch is over.
	const station = watchStation(
		client,
		browser ? backendOrigin(window.location) : 'http://localhost:7700',
		undefined,
		endSwitch
	);
	setStationContext(station);
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
	// And this browser's own faults into the station's log, where the System Log
	// panel — and a peer console, and the run's file — can see them. A panel that
	// throws is otherwise invisible to everything but this tab's devtools.
	reportBrowserErrors(client);
	// And what it is costing itself, every couple of seconds, whether or not anybody
	// has the System panel open. A browser worth knowing about is precisely the one
	// with nobody in front of it, so this cannot be something a panel opts into.
	reportBrowserStats(client, frameMeter);

	/** The verbs in the top bar, so the keymap can press the same buttons. */
	let verbs = $state<ReturnType<typeof Verbs> | null>(null);
	/**
	 * When Escape was last pressed, for the second one.
	 *
	 * Esc gives the rig back to playback; Esc Esc also drops the selection. Two acts
	 * rather than one because they are separate on purpose — the spec keeps the buffer
	 * and the selection apart, so a look can be parked and reached again from a
	 * different selection — and because dropping a hard-won selection by reflex is a
	 * minute of clicking to get back.
	 */
	let lastEscape = 0;
	const ESCAPE_AGAIN_MS = 600;

	/**
	 * Ctrl-Z anywhere that is not a text field.
	 *
	 * On the window rather than on a panel, because undo is not any one panel's: an
	 * operator who has just deleted a fixture in Patch and moved to the Plan still
	 * means that fixture.
	 */
	function onKey(event: KeyboardEvent) {
		// Ctrl/Cmd+K lands in a command line, wherever one is open. Allowed even
		// from a text field: the browsers' own use of the key is a search bar,
		// and stealing it from an input is exactly what an operator mid-rename
		// pressing it means.
		if (event.key.toLowerCase() === 'k' && (event.ctrlKey || event.metaKey)) {
			if (focusConsole()) event.preventDefault();
			return;
		}
		// Ctrl/Cmd+S is Save, which on this console means "take a version" — there is
		// nothing unsaved to flush. Taken from a text field too: an operator mid-rename
		// pressing it means the show, and the browser's own Save-page dialog is never
		// what they wanted.
		if (event.key.toLowerCase() === 's' && (event.ctrlKey || event.metaKey)) {
			event.preventDefault();
			takeAVersion();
			return;
		}
		// Everything below is a console verb rather than a browser one, and none of it
		// may fire while somebody is typing: a cue being renamed contains spaces, and
		// Space is Go.
		const typing = isTextField(event.target);

		// The three verbs, and the cue sheet's Go. Ctrl/Cmd+Enter and Ctrl/Cmd+U are
		// what every other desk puts under a thumb; Space is Go, on the cue sheet
		// touched last.
		if (!typing && $station?.show) {
			if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
				event.preventDefault();
				void verbs?.openStore();
				return;
			}
			if (event.key.toLowerCase() === 'u' && (event.ctrlKey || event.metaKey)) {
				event.preventDefault();
				void verbs?.update();
				return;
			}
			if (event.key === 'Escape') {
				event.preventDefault();
				const now = Date.now();
				if (now - lastEscape < ESCAPE_AGAIN_MS) clearSelection();
				lastEscape = now;
				void clear({ keepLocked: true });
				return;
			}
			if (event.key === ' ') {
				event.preventDefault();
				void goOnFocusedSheet();
				return;
			}
		}

		const action = shortcutFor(event, typing);
		if (!action) return;
		event.preventDefault();
		if (action === 'undo') undo();
		else redo();
	}

	/**
	 * Space: Go on the cue sheet touched last.
	 *
	 * With none focused there is **no Go and a toast**, never a guess. A console with
	 * three cue sheets open has to answer which one Space means, and answering it by
	 * picking one is a look on stage nobody asked for.
	 */
	async function goOnFocusedSheet() {
		const sequenceId = $focusedCueSheet;
		if (!sequenceId) {
			addToast('Space is Go on a cue sheet — click one first');
			return;
		}
		try {
			await data.sequences.byId(sequenceId).goNext({ at: Date.now() });
		} catch (e) {
			addToast(`${e}`);
		}
	}

	/** Save: a checkpoint, with no name. The Show panel is where one is given a name. */
	async function takeAVersion() {
		if (!$station?.show) return;
		try {
			await data.versions.checkpoint({});
			addToast('Version saved', 'success');
		} catch (e) {
			addToast(`Could not save a version: ${e}`);
		}
	}

	// A show closing takes Setup with it: every section of it is about the open show,
	// and a dialog that survived the close would come back over the next one.
	$effect(() => {
		if (!$station?.show) closeSetup();
	});

	let connected = $state(false);
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
		// A switch covers at once and stays covered through the reconnect and the
		// reload: the operator asked for it and the screen says what it is doing.
		if ($switching) {
			covering = true;
			return;
		}
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
		client.onConnect = () => { connected = true; everConnected = true; };
		client.onDisconnect = () => { connected = false; };
		// A station making way for another says so in its close frame, which is how
		// the tablet on somebody else's socket draws "Opening Festival…" rather than
		// "the console stopped answering".
		client.onClose = (code, reason) => {
			const asked = switchFromClose(code, reason, Date.now());
			if (asked) beginSwitch(asked.doing);
		};
		client.onError = (msg) => addToast(msg);
		client.connect();
		return () => client.disconnect();
	});
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
	<header class="topbar">
		<span class="brand" title={$station ? `station ${$station.nodeId}` : address}>
			the-pult{#if $station}<span class="version">{$station.version}</span>{/if}
		</span>
		{#if $station?.show}
			<ShowMenu show={$station.show} />
			<LayoutBar />
			<!-- Setup is a mode, not a tile. See `components/setup/Setup.svelte`. -->
			<button class="setup-btn" class:on={$setupSection !== null} onclick={toggleSetup}>
				Setup<span class="caret">▾</span>
			</button>
			<!-- The three verbs, in the chrome rather than in a panel: an operator
			     programming in the rig, in the plan or from the command line means the
			     same three things. -->
			<Verbs bind:this={verbs} />
		{/if}
		<span class="spacer"></span>
		<UserBar />
		<ConnectionStatus {connected} />
	</header>
	<main>
		{@render children()}
	</main>
</div>

{#if covering}
	<ConnectingOverlay
		{everConnected}
		{address}
		switching={$switching}
		onretry={() => {
			// Trying again by hand means the switch is no longer being waited on as
			// one: whatever the console is now, the ordinary screens take over.
			endSwitch();
			client.retryNow();
		}}
	/>
{/if}

<Toasts />

<!-- Everything that is done once, over the workspace rather than inside it. At the
     root because it is a mode of the whole window, and because the menu that opens
     it lives in the top bar. -->
{#if $setupSection !== null}
	<Setup />
{/if}

<!-- "This bar has six lights on it." Mounted here rather than in a tile because a
     modal belongs to the window: the verb can be reached from Rig tools, from the
     Objects list and from the rig's own Delete key, and a prompt owned by one of
     them would be missing from the other two. -->
{#if $deleteAsk}
	<DeletePrompt
		name={$deleteAsk.name}
		objects={$deleteAsk.objects}
		fixtures={$deleteAsk.fixtures}
		onanswer={answerDelete}
	/>
{/if}

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

	.setup-btn {
		background: none;
		border: none;
		color: var(--text-dim, #888);
		font: inherit;
		font-size: var(--font-sm, 12px);
		font-weight: 600;
		cursor: pointer;
		padding: 4px 2px;
		white-space: nowrap;
	}
	.setup-btn:hover,
	.setup-btn.on {
		color: var(--text-bright, #fff);
	}
	.caret {
		margin-left: 5px;
		color: var(--text-faint, #555);
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
