<script lang="ts">
	/**
	 * The GDTF Share, as a list you can search and import from.
	 *
	 * The Share holds tens of thousands of fixture definitions, so this is a search box
	 * over a list the *station* keeps — fetched once, cached for a day, searched
	 * locally. The browser never sees the whole thing.
	 *
	 * The credential lives on the station, in its preferences, and never in the show:
	 * a showfile travels, and a password in one travels with it. So there is a login
	 * form here rather than in a Settings panel — this is where somebody finds out
	 * they need one.
	 */
	import { backendOrigin } from '$lib/ws/endpoint.js';
	import { userId } from '$lib/stores/user.js';
	import { onMount } from 'svelte';

	type ShareMode = { name: string; dmxfootprint: number };
	type ShareFixture = {
		rid: number;
		fixture: string;
		manufacturer: string;
		revision: string;
		rating: number | null;
		modes: ShareMode[];
		/// `"Manuf."` where the manufacturer published it, `"User"` where somebody else
		/// did. The Share's own answer to which of seven MegaPointes is the real one.
		uploader: string;
	};
	type Status = {
		configured: boolean;
		user: string | null;
		signedIn: boolean;
		listSize: number;
		listAgeSeconds: number | null;
	};

	const backend = () => backendOrigin(window.location);

	let status = $state<Status | null>(null);
	let query = $state('');
	let hits = $state<ShareFixture[]>([]);
	let searching = $state(false);
	let refreshing = $state(false);
	let importingRid = $state<number | null>(null);
	let message = $state<{ ok: boolean; text: string } | null>(null);

	// The login form. The password is write-only: the station answers whether one is
	// set and never what it is, so this box starts empty and an empty box means
	// "leave the one you have".
	let user = $state('');
	let password = $state('');
	let saving = $state(false);

	async function loadStatus() {
		try {
			const answer = await fetch(`${backend()}/api/gdtf-share/status`);
			status = await answer.json();
			user = status?.user ?? '';
		} catch {
			status = null;
		}
	}

	async function saveLogin() {
		saving = true;
		message = null;
		try {
			const answer = await fetch(`${backend()}/api/preferences`, {
				method: 'PUT',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({ gdtfShare: { user, password } })
			});
			if (!answer.ok) {
				message = { ok: false, text: await answer.text() };
				return;
			}
			password = '';
			await loadStatus();
			message = { ok: true, text: 'Login saved on this station.' };
		} finally {
			saving = false;
		}
	}

	async function search(refresh = false) {
		if (refresh) refreshing = true;
		else searching = true;
		message = null;
		try {
			const url = new URL(`${backend()}/api/gdtf-share/search`);
			url.searchParams.set('q', query);
			url.searchParams.set('limit', '60');
			if (refresh) url.searchParams.set('refresh', 'true');
			const answer = await fetch(url);
			if (!answer.ok) {
				hits = [];
				message = { ok: false, text: await answer.text() };
				return;
			}
			hits = (await answer.json()).fixtures ?? [];
			if (hits.length === 0) message = { ok: true, text: 'Nothing matched.' };
			await loadStatus();
		} catch (error) {
			message = { ok: false, text: String(error) };
		} finally {
			searching = false;
			refreshing = false;
		}
	}

	async function importFixture(row: ShareFixture) {
		importingRid = row.rid;
		message = null;
		try {
			const answer = await fetch(`${backend()}/api/gdtf-share/import?rid=${row.rid}`, {
				method: 'POST',
				headers: { 'x-pult-user': $userId }
			});
			if (!answer.ok) {
				message = { ok: false, text: await answer.text() };
				return;
			}
			const body = await answer.json();
			const warnings: string[] = body.warnings ?? [];
			message = {
				ok: true,
				text:
					`${body.replaced ? 'Updated' : 'Imported'} ${row.manufacturer} ${row.fixture}` +
					(warnings.length > 0
						? ` — ${warnings.length} thing${warnings.length === 1 ? '' : 's'} worth knowing.`
						: '.')
			};
		} catch (error) {
			message = { ok: false, text: String(error) };
		} finally {
			importingRid = null;
		}
	}

	/// How old the station's copy of the list is, in words.
	const listAge = $derived.by(() => {
		const seconds = status?.listAgeSeconds;
		if (seconds === null || seconds === undefined) return null;
		if (seconds < 90) return 'just now';
		if (seconds < 3600) return `${Math.round(seconds / 60)} min ago`;
		return `${Math.round(seconds / 3600)} h ago`;
	});

	onMount(loadStatus);
</script>

<div class="share">
	{#if status && !status.configured}
		<!-- The empty state is the useful one: somebody who has never used the Share
		     arrives here and needs to be told what is missing and where it goes. -->
		<p class="explain">
			The GDTF Share needs an account — a user name, not an email address. It is free,
			from
			<a href="https://gdtf-share.com" target="_blank" rel="noreferrer">gdtf-share.com</a>.
			The login is kept on <em>this station</em> and never in the show — a showfile
			travels, and a password in one would travel with it.
		</p>
	{/if}

	{#if status && (!status.configured || password || user !== (status.user ?? ''))}
		<form class="login" onsubmit={(e) => { e.preventDefault(); saveLogin(); }}>
			<!-- Text, not email: a Share account is a username, and `type="email"` makes a
			     browser refuse a perfectly good one. -->
			<input
				class="input"
				type="text"
				autocomplete="username"
				placeholder="Share user name"
				bind:value={user}
			/>
			<input
				class="input"
				type="password"
				autocomplete="current-password"
				placeholder={status.configured ? 'password (unchanged)' : 'password'}
				bind:value={password}
			/>
			<button class="btn btn-primary" type="submit" disabled={saving || !user}>
				{saving ? 'Saving…' : 'Save login'}
			</button>
		</form>
	{/if}

	{#if status?.configured}
		<form class="find" onsubmit={(e) => { e.preventDefault(); search(); }}>
			<input class="input" placeholder="Fixture or manufacturer" bind:value={query} />
			<button class="btn btn-primary" type="submit" disabled={searching}>
				{searching ? 'Searching…' : 'Search'}
			</button>
			<button
				class="btn btn-ghost"
				type="button"
				title="Fetch the Share's list again"
				disabled={refreshing}
				onclick={() => search(true)}
			>{refreshing ? 'Fetching…' : 'Refresh list'}</button>
			{#if status.listSize > 0}
				<span class="hint">{status.listSize.toLocaleString()} known · {listAge}</span>
			{/if}
		</form>
	{/if}

	{#if message}
		<p class="message" class:bad={!message.ok}>{message.text}</p>
	{/if}

	{#if hits.length > 0}
		<table class="hits">
			<thead>
				<tr><th>Manufacturer</th><th>Fixture</th><th>Revision</th><th>Modes</th><th></th></tr>
			</thead>
			<tbody>
				{#each hits as row (row.rid)}
					<tr>
						<td>
							{row.manufacturer}
							{#if row.uploader.startsWith('Manuf')}
								<!-- Worth a badge: a popular fixture has half a dozen files on the
								     Share and only one of them is the manufacturer's. -->
								<span class="badge" title="Published by the manufacturer">manuf.</span>
							{/if}
						</td>
						<td>{row.fixture}</td>
						<td>
							{row.revision}
							{#if row.rating !== null}<span class="hint">★ {row.rating.toFixed(1)}</span>{/if}
						</td>
						<td class="modes">
							<!-- The footprints, because two revisions of one fixture are often
							     told apart by nothing else. -->
							{row.modes.map((m) => `${m.name} (${m.dmxfootprint})`).join(', ') || '—'}
						</td>
						<td>
							<button
								class="btn btn-ghost"
								disabled={importingRid !== null}
								onclick={() => importFixture(row)}
							>{importingRid === row.rid ? 'Importing…' : 'Import'}</button>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</div>

<style>
	.share { display: flex; flex-direction: column; gap: 10px; padding: 4px 0; }
	.explain { margin: 0; color: #999; font-size: 12px; line-height: 1.5; }
	.explain a { color: inherit; }
	.login,
	.find { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
	.login .input,
	.find .input { flex: 1 1 160px; min-width: 120px; }
	.hint { color: #999; font-size: 11px; }
	.message { margin: 0; font-size: 12px; color: #999; }
	.message.bad { color: #e5534b; }
	.hits { width: 100%; border-collapse: collapse; font-size: 12px; }
	.hits th,
	.hits td { text-align: left; padding: 4px 8px 4px 0; border-bottom: 1px solid #2e2e2e; }
	.hits th { font-weight: 500; color: #999; }
	.hits .modes { color: #999; max-width: 280px; }
	.badge {
		display: inline-block;
		margin-left: 4px;
		padding: 0 4px;
		border-radius: 3px;
		background: #2e3a2e;
		color: #8fbf8f;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.03em;
	}
</style>
