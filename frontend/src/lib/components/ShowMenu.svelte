<script lang="ts">
	/**
	 * Which show is open, and what to do with it.
	 *
	 * In the top bar beside the layout menu, because a show is the thing this whole
	 * window is about: an operator should not have to find a panel to save one.
	 *
	 * Everything here that changes *which* show is open goes through `show.*` — a
	 * station RPC, not a command, because it is nobody's to undo and must not be told
	 * to a peer. Each answers and then the station stops; this page's socket closes,
	 * reconnects onto the new console, and reloads.
	 */

	import { dropdown } from '$lib/actions.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { show as openShow } from '$lib/stores/show.js';
	import { openSetup } from '$lib/stores/setup.js';
	import { addToast } from '$lib/toasts.js';
	import { backendOrigin, type OpenShow } from '$lib/ws/endpoint.js';
	import NameDialog from './NameDialog.svelte';
	import { beginSwitch, endSwitch } from '$lib/stores/switching.js';
	import { collection } from '$lib/stores/show.js';
	import Drama from '@lucide/svelte/icons/drama';

	let { show }: { show: OpenShow } = $props();

	const client = getClientContext();
	const data = getDataContext();

	let open = $state(false);
	let naming = $state<'version' | 'copy' | null>(null);
	let draft = $state('');
	let busy = $state(false);

	/**
	 * When the last version was taken, said beside the name rather than as a Save
	 * button: every edit is already on the disk, so the thing worth knowing is how far
	 * back the last point to return to is, not whether anything is unsaved.
	 */
	const versions = collection('versions');
	let now = $state(Date.now());
	$effect(() => {
		const tick = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(tick);
	});
	const lastVersion = $derived.by(() => {
		let latest = 0;
		for (const v of $versions) latest = Math.max(latest, Date.parse(v.created_at));
		if (!latest) return 'no versions yet';
		const minutes = Math.round((now - latest) / 60_000);
		if (minutes < 1) return 'version just now';
		if (minutes < 60) return `version ${minutes} min ago`;
		const hours = Math.round(minutes / 60);
		if (hours < 24) return `version ${hours} h ago`;
		return `version ${Math.round(hours / 24)} d ago`;
	});

	// The show's own name where it has one — the row is what an operator renamed —
	// falling back to what the folder is called, which is all `/api/config` knows.
	const name = $derived($openShow?.name ?? show.name);

	function start(what: 'version' | 'copy') {
		draft = what === 'copy' ? `${name} copy` : '';
		naming = what;
		open = false;
	}

	async function commit(typed: string) {
		const what = naming;
		naming = null;
		if (what === 'version') return save(typed || undefined);
		if (what === 'copy' && typed) return ask('show.saveAs', { name: typed }, `saving a copy as ${typed}`);
	}

	/** A version, named or not. Not a switch: the console goes on running. */
	async function save(withName?: string) {
		busy = true;
		try {
			await data.versions.checkpoint(withName ? { name: withName } : {});
			addToast(withName ? `Saved “${withName}”` : 'Version saved', 'success');
		} catch (e) {
			addToast(`Could not save a version: ${e}`);
		} finally {
			busy = false;
		}
	}

	/**
	 * A switch. Answers, and then the station stops — so nothing here waits after it.
	 *
	 * The switching screen goes up *before* the station is asked, with the words the
	 * operator chose: the socket is about to close, and the cover has to be there
	 * before the connection handling notices.
	 */
	async function ask(method: string, args: Record<string, unknown>, doing: string) {
		open = false;
		if (busy) return;
		busy = true;
		beginSwitch(doing);
		try {
			await client.call(method, args);
		} catch (e) {
			endSwitch();
			addToast(`${e}`);
			busy = false;
		}
	}

	/**
	 * Export. A plain link rather than a fetch: the browser's own download is what
	 * turns a response into a file on somebody's disk, and it carries the name the
	 * station puts in the Content-Disposition.
	 */
	function exportUrl(versions: boolean): string {
		const url = new URL('/api/shows/export', backendOrigin(window.location));
		if (versions) url.searchParams.set('versions', '1');
		return url.toString();
	}
</script>

<div class="bar">

		<button class="tb-menu" aria-expanded={open} onclick={() => (open = !open)} title={show.path}>
			<Drama size={16} /><span class="name">{name}</span><span class="tb-sub">· {lastVersion}</span><span class="tb-caret">▾</span>
		</button>

	{#if open}
		<div class="menu" use:dropdown={{ close: () => (open = false) }}>
			<span class="heading">This show</span>
			<button
				disabled={busy}
				onclick={() => {
					open = false;
					save();
				}}
			><span>Save version</span><kbd>⌘S</kbd></button>
			<button onclick={() => start('version')}>Save a named version…</button>
			<button
				onclick={() => {
					open = false;
					openSetup('show');
				}}>Versions…</button
			>
			<button onclick={() => start('copy')}>Save a copy…</button>
			<a class="row" href={exportUrl(false)} download onclick={() => (open = false)}>
				Export .pultz
			</a>
			<a class="row dim" href={exportUrl(true)} download onclick={() => (open = false)}>
				Export with versions
			</a>
			<span class="heading">Another show</span>
			<button onclick={() => ask('show.close', {}, 'closing the show')}>Close — back to the start</button>
			<span class="path">{show.path}</span>
		</div>
	{/if}
</div>

{#if naming}
	<NameDialog
		title={naming === 'copy' ? 'Save a copy of the show' : 'Save a named version'}
		placeholder={naming === 'copy' ? 'Copy name…' : 'Version name…'}
		initial={draft}
		confirm={naming === 'copy' ? 'Save copy' : 'Save version'}
		hint={naming === 'copy'
			? 'A new show folder beside this one; the console opens it.'
			: 'A point in this show to come back to, listed under Versions.'}
		onsubmit={commit}
		onclose={() => (naming = null)}
	/>
{/if}

<style>
	.bar {
		position: relative;
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.name {
		max-width: 220px;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.menu button {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
	}
	.menu kbd {
		font: inherit;
		font-size: 10px;
		color: var(--text-faint, #555);
	}


	.menu {
		position: absolute;
		top: 100%;
		left: 0;
		z-index: 20;
		min-width: 230px;
		display: flex;
		flex-direction: column;
		padding: 5px;
		background: var(--bg-panel, #222);
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: var(--radius, 4px);
		box-shadow: 0 8px 24px rgb(0 0 0 / 45%);
	}
	.menu button,
	.menu .row {
		background: none;
		border: none;
		border-radius: 3px;
		color: var(--text, #e0e0e0);
		font-size: var(--font-sm, 12px);
		text-align: left;
		text-decoration: none;
		padding: 7px 8px;
		cursor: pointer;
	}
	.menu button:hover,
	.menu .row:hover {
		background: var(--bg-hover, #2a2a2a);
	}
	.menu .row.dim {
		color: var(--text-dim, #888);
	}

	.heading {
		font-size: var(--font-xs, 11px);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint, #555);
		padding: 8px 8px 4px;
	}

	.path {
		font-family: monospace;
		font-size: 10px;
		color: var(--text-faint, #555);
		padding: 6px 8px 3px;
		border-top: 1px solid var(--line, #2a2a2a);
		margin-top: 4px;
		overflow: hidden;
		text-overflow: ellipsis;
		direction: rtl;
	}

</style>
