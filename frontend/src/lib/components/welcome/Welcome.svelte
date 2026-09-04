<script lang="ts">
	/**
	 * What a console with no show open looks like.
	 *
	 * Which is a real state and the one a console started with no arguments comes up
	 * in: the engine runs, the socket is open, the station is on the network. There
	 * is simply no show, and this is what to do about that.
	 *
	 * Everything here goes through `show.*`, which are station RPCs rather than
	 * commands — opening a show is nobody's to undo and must not be told to a peer.
	 * Each of them answers and *then* the station stops, so this page's socket closes
	 * and reconnects onto the new console, which `stores/station.ts` turns into a
	 * reload. That is why nothing here waits for a result beyond the acknowledgement.
	 */

	import { onMount } from 'svelte';

	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { backendOrigin } from '$lib/ws/endpoint.js';
	import { baseName, folderName, links, orderRecent, type ShowList } from '$lib/shows.js';
	import { focusOnMount, selectOnMount } from '$lib/actions.js';
	import { beginSwitch, endSwitch } from '$lib/stores/switching.js';
	import DemoCard from './DemoCard.svelte';
	import ShowCard from './ShowCard.svelte';

	let {
		version,
		nodeId,
		repository
	}: { version: string; nodeId: string; repository: string } = $props();

	const client = getClientContext();

	let list = $state<ShowList | null>(null);
	let busy = $state(false);
	let naming = $state(false);
	let draft = $state('');
	let openingPath = $state('');
	let importing = $state(false);
	/**
	 * What is on the network, as the LOCAL `session` state reports it.
	 *
	 * Subscribed here rather than read from a store, because there is no store for
	 * it — `SessionPanel` does the same, and this is the second reader rather than
	 * the reason to build one.
	 */
	type Discovered = { session_id: string; show_name: string; sync_addr: string };
	let sessions = $state<Discovered[]>([]);

	const recent = $derived(orderRecent(list?.recent ?? []));
	const elsewhere = $derived(
		(list?.inDir ?? []).filter((show) => !recent.some((seen) => seen.path === show.path))
	);

	async function refresh() {
		try {
			list = (await client.call('show.list')) as ShowList;
		} catch (e) {
			addToast(`Could not list shows: ${e}`);
		}
	}

	/**
	 * Ask for a show. The station acknowledges and then stops, so this never sees the
	 * new console — the page reconnects onto it and reloads. The switching screen goes
	 * up first, in the operator's own words, so the whole of that is one cover.
	 */
	async function ask(method: string, args: Record<string, unknown>, doing: string) {
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

	async function makeOne() {
		const name = draft.trim();
		if (!name) return;
		naming = false;
		await ask('show.new', { name }, `making ${name}`);
	}

	/**
	 * Join a session that is already on the network.
	 *
	 * A station has to *have* a show before it can be handed one, so this makes a
	 * bundle named after the session's show and tells the new station to join once it
	 * is up. Which also sidesteps the trap in the sync layer's `Hello`: a station
	 * joining with a different show open has that show overwritten.
	 */
	async function join(sessionId: string, showName: string) {
		await ask('show.new', { name: showName, thenJoin: sessionId }, `joining ${showName}`);
	}

	async function importPultz(event: Event) {
		const input = event.target as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		importing = true;
		try {
			const response = await fetch(new URL('/api/shows/import', backendOrigin(window.location)), {
				method: 'POST',
				body: file
			});
			const body = await response.text();
			if (!response.ok) throw new Error(body);
			// Imported, not opened: somebody taking four shows off a stick has not
			// asked to be moved into the last one. The list picks it up instead.
			addToast(`Imported ${(JSON.parse(body) as { name: string }).name}`, 'success');
			await refresh();
		} catch (e) {
			addToast(`Import failed: ${e}`);
		} finally {
			importing = false;
		}
	}

	onMount(() => {
		void refresh();
		const take = (value: unknown) => {
			if (value && typeof value === 'object') {
				sessions = ((value as { discovered?: Discovered[] }).discovered ?? []).filter(
					(found) => found.session_id
				);
			}
		};
		const stop = client.subscribe('session', take);
		const askWhoIsHere = () => void client.get(['session']).then(take);
		askWhoIsHere();
		const stopConnect = client.addConnectListener(askWhoIsHere);
		return () => {
			stop();
			stopConnect();
		};
	});
</script>

<div class="welcome">
	<header>
		<h1>the-pult</h1>
		<p class="sub">
			No show open. <span class="mono">{version}</span> ·
			<span class="mono" title="this station">{nodeId.slice(0, 8)}</span>
		</p>
		<nav>
			{#each links(repository) as link (link.href)}
				<a href={link.href} target="_blank" rel="noreferrer">{link.label}</a>
			{/each}
		</nav>
	</header>

	<section>
		<h2>Start a show</h2>
		<div class="row">
			{#if naming}
				<form
					class="naming"
					onsubmit={(e) => {
						e.preventDefault();
						makeOne();
					}}
				>
					<input
						bind:value={draft}
						placeholder="Show name…"
						use:focusOnMount
						use:selectOnMount
						onkeydown={(e) => e.key === 'Escape' && (naming = false)}
					/>
					<span class="hint mono">{folderName(draft || 'Untitled Show')}</span>
					<button class="primary" type="submit" disabled={busy || !draft.trim()}>Create</button>
					<button type="button" onclick={() => (naming = false)}>Cancel</button>
				</form>
			{:else}
				<button
					class="primary"
					disabled={busy}
					onclick={() => {
						draft = '';
						naming = true;
					}}>New show…</button
				>
				<label class="file">
					<input type="file" accept=".pultz" onchange={importPultz} disabled={importing} />
					<span>{importing ? 'Importing…' : 'Import .pultz…'}</span>
				</label>
				<form
					class="open-path"
					onsubmit={(e) => {
						e.preventDefault();
						if (openingPath.trim())
							ask('show.open', { path: openingPath.trim() }, `opening ${baseName(openingPath.trim())}`);
					}}
				>
					<input bind:value={openingPath} placeholder="Open a path…" spellcheck="false" />
					<button type="submit" disabled={busy || !openingPath.trim()}>Open</button>
				</form>
			{/if}
		</div>
		{#if list?.showsDir}
			<p class="where mono">New shows go in {list.showsDir}</p>
		{:else if list}
			<p class="where warn">This console has nowhere to keep shows.</p>
		{/if}
	</section>

	{#if recent.length}
		<section>
			<h2>Recent</h2>
			<div class="cards">
				{#each recent as show (show.path)}
					<ShowCard {show} onopen={(path) => ask('show.open', { path }, `opening ${show.name}`)} />
				{/each}
			</div>
		</section>
	{/if}

	{#if elsewhere.length}
		<section>
			<h2>In the shows folder</h2>
			<div class="cards">
				{#each elsewhere as show (show.path)}
					<ShowCard {show} onopen={(path) => ask('show.open', { path }, `opening ${show.name}`)} />
				{/each}
			</div>
		</section>
	{/if}

	{#if sessions.length}
		<section>
			<h2>On the network</h2>
			<p class="lead">
				Another console is running a show here. Joining makes a copy of it on this
				station and follows along.
			</p>
			<div class="cards">
				{#each sessions as found (found.session_id)}
					<button
						class="joinable"
						disabled={busy}
						onclick={() => join(found.session_id, found.show_name)}
					>
						<span class="name">{found.show_name}</span>
						<span class="what">at {found.sync_addr}</span>
					</button>
				{/each}
			</div>
		</section>
	{/if}

	{#if list?.demos?.length}
		<section>
			<h2>Or open a demo</h2>
			<p class="lead">
				Four shows this console makes for itself. Nothing to download, and yours to
				break — they land in the shows folder like any other.
			</p>
			<div class="cards demos">
				{#each list.demos as demo (demo.id)}
					<DemoCard
						{demo}
						{busy}
						onopen={(id) => ask('show.new', { name: demo.title, demo: id }, `making ${demo.title}`)}
					/>
				{/each}
			</div>
		</section>
	{/if}
</div>

<style>
	.welcome {
		height: 100%;
		overflow-y: auto;
		padding: 40px 32px 64px;
		display: flex;
		flex-direction: column;
		gap: 32px;
		max-width: 1100px;
		margin: 0 auto;
	}

	header h1 {
		font-size: 1.8rem;
		font-weight: 600;
		letter-spacing: 0.04em;
		color: var(--text-bright);
	}
	.sub {
		margin-top: 4px;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	nav {
		display: flex;
		gap: 14px;
		margin-top: 10px;
	}
	nav a {
		color: var(--accent);
		font-size: var(--font-sm);
		text-decoration: none;
	}
	nav a:hover {
		text-decoration: underline;
	}

	h2 {
		font-size: 0.7rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.09em;
		color: var(--text-dim);
		margin-bottom: 10px;
	}

	.lead {
		font-size: var(--font-sm);
		color: var(--text-dim);
		margin: -4px 0 12px;
		max-width: 62ch;
		line-height: 1.5;
	}

	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 10px;
		align-items: center;
	}

	.cards {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 10px;
	}
	.cards.demos {
		grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
	}

	button,
	.file span {
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font-size: var(--font-sm);
		padding: 8px 14px;
		cursor: pointer;
	}
	button:hover:not(:disabled),
	.file:hover span {
		border-color: var(--accent);
	}
	button:disabled {
		opacity: 0.5;
		cursor: default;
	}
	button.primary {
		background: var(--accent-solid);
		border-color: var(--accent-solid);
		color: var(--text-bright);
	}

	.file input {
		display: none;
	}
	.file span {
		display: inline-block;
	}

	.naming,
	.open-path {
		display: flex;
		gap: 8px;
		align-items: center;
		flex-wrap: wrap;
	}
	.naming input,
	.open-path input {
		background: var(--bg-sunken);
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		color: var(--text);
		font-size: var(--font-sm);
		padding: 8px 10px;
		min-width: 220px;
	}

	.hint {
		font-size: var(--font-xs);
		color: var(--text-faint);
	}

	.where {
		margin-top: 10px;
		font-size: var(--font-xs);
		color: var(--text-faint);
	}
	.where.warn {
		color: var(--live);
	}

	.joinable {
		display: flex;
		flex-direction: column;
		gap: 3px;
		align-items: flex-start;
		text-align: left;
		padding: 12px 14px;
	}
	.joinable .name {
		font-size: 0.95rem;
		font-weight: 600;
		color: var(--text-bright);
	}
	.joinable .what {
		font-size: var(--font-xs);
		color: var(--text-faint);
	}

	.mono {
		font-family: monospace;
	}
</style>
