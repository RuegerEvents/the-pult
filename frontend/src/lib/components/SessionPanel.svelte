<script lang="ts">
	import { onMount } from 'svelte';
	import type { NearbyStation, SessionState } from '$lib/generated/index.js';
	import { collection } from '$lib/stores/show.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import Dialog from './Dialog.svelte';

	const client = getClientContext();
	const data = getDataContext();
	const stations = collection('stations');

	let session = $state<SessionState>({
		node_id: null,
		is_advertising: false,
		is_follower: false,
		session_id: null,
		discovered: [],
		nearby: []
	});
	/**
	 * Who could be invited: every station on the network that is not already in this
	 * session. Read against the `stations` rows, which are exactly this session's
	 * members — the session id a station advertises is its own name for the session
	 * and need not match this one's.
	 */
	const invitable = $derived(session.nearby.filter((s) => !$stations.some((m) => m.id === s.node_id)));
	/** The station an invitation is being confirmed for. */
	let inviting = $state<NearbyStation | null>(null);
	let sending = $state(false);

	async function invite() {
		const guest = inviting;
		if (!guest) return;
		sending = true;
		try {
			await client.call('session.invite', { nodeId: guest.node_id });
			addToast(`${guest.hostname} joined`, 'success');
			inviting = null;
		} catch (e) {
			addToast(`Invite failed: ${e}`);
		} finally {
			sending = false;
		}
	}
	let joining = $state<string | null>(null);
	let busy = $state(false);
	// A station's sync address, typed in: the way in where mDNS does not reach — a
	// venue switch that drops multicast, or a Mac never given Local Network access.
	let address = $state('');

	async function advertise() {
		const show = await data.show.get();
		if (!show) { addToast('Initialize a show first before advertising.', 'warning'); return; }
		busy = true;
		try {
			await client.call('session.create', { showName: show.name, showId: show.id });
		} catch (e) {
			addToast(`Advertise failed: ${e}`);
		} finally {
			busy = false;
		}
	}

	async function join(sessionId: string) {
		joining = sessionId;
		try {
			await client.call('session.join', { sessionId });
		} catch (e) {
			addToast(`Join failed: ${e}`);
		} finally {
			joining = null;
		}
	}

	async function joinAt() {
		const at = address.trim();
		if (!at) return;
		joining = at;
		try {
			await client.call('session.joinAt', { address: at });
			address = '';
		} catch (e) {
			addToast(`Join failed: ${e}`);
		} finally {
			joining = null;
		}
	}

	async function leave() {
		busy = true;
		try {
			await client.call('session.leave');
		} catch (e) {
			addToast(`Leave failed: ${e}`);
		} finally {
			busy = false;
		}
	}

	onMount(() => {
		// session is LOCAL state not in ShowState — subscribe via client directly
		const unsub = client.subscribe('session', v => { if (v && typeof v === 'object') session = { ...session, ...(v as SessionState) }; });
		const doFetch = () => client.get(['session']).then(v => { if (v && typeof v === 'object') session = { ...session, ...(v as SessionState) }; });
		doFetch();
		const unsubConnect = client.addConnectListener(doFetch);
		return () => { unsub(); unsubConnect(); };
	});
</script>

<div class="panel">
	<div class="panel-header">
		<span class="panel-title">Sync</span>
		{#if session.is_advertising}
			<span class="badge badge--green">● Advertising</span>
		{:else if session.is_follower}
			<span class="badge badge--blue">● In session</span>
		{:else}
			<span class="badge badge--dim">○ Idle</span>
		{/if}
	</div>

	{#if session.is_follower}
		<!-- ── Follower view ── -->
		<p class="info-text">
			Connected to session<br />
			<span class="mono dim">{session.session_id?.slice(0, 8)}…</span>
		</p>
		<p class="hint">This node cannot advertise while joined.</p>
		<button class="action-btn leave-btn" onclick={leave} disabled={busy}>
			{busy ? '…' : 'Leave session'}
		</button>

	{:else if session.is_advertising}
		<!-- ── Advertising view ── -->
		<p class="info-text">
			Advertising session<br />
			<span class="mono dim">{session.session_id?.slice(0, 8)}…</span>
		</p>
		<p class="hint">Peers on the network can discover and join this session.</p>

		{#if session.discovered.length > 0}
			<p class="sub-label">Discovered peers</p>
			<div class="session-list">
				{#each session.discovered as s (s.session_id)}
					<div class="session-row">
						<div class="session-info">
							<span class="session-name">{s.show_name}</span>
							<span class="session-addr dim mono">{s.sync_addr}</span>
						</div>
					</div>
				{/each}
			</div>
		{/if}

		<button class="action-btn leave-btn" onclick={leave} disabled={busy}>
			{busy ? '…' : 'Stop advertising'}
		</button>

	{:else}
		<!-- ── Idle view ── -->
		{#if session.discovered.length === 0}
			<p class="empty-hint">No sessions found on the network.</p>
		{:else}
			<div class="session-list">
				{#each session.discovered as s (s.session_id)}
					<div class="session-row">
						<div class="session-info">
							<span class="session-name">{s.show_name}</span>
							<span class="session-addr dim mono">{s.sync_addr}</span>
						</div>
						<button
							class="join-btn"
							disabled={joining === s.session_id}
							onclick={() => join(s.session_id)}
						>
							{joining === s.session_id ? '…' : 'Join'}
						</button>
					</div>
				{/each}
			</div>
		{/if}

		<form class="join-at" onsubmit={(e) => { e.preventDefault(); joinAt(); }}>
			<input
				class="mono"
				placeholder="host:sync-port, e.g. 10.0.0.2:7701"
				aria-label="Join a station by its sync address"
				bind:value={address}
			/>
			<button class="join-btn" type="submit" disabled={!address.trim() || joining === address.trim()}>
				{joining === address.trim() && address.trim() ? '…' : 'Join'}
			</button>
		</form>
		<p class="hint">Not listed? Join by the other console's IP and its sync port (7701 unless it was started with another).</p>

		<button class="action-btn advertise-btn" onclick={advertise} disabled={busy}>
			{busy ? 'Advertising…' : 'Advertise this show'}
		</button>
	{/if}

	{#if !session.is_follower && invitable.length > 0}
		<p class="sub-label">Stations on the network</p>
		<div class="session-list">
			{#each invitable as s (s.node_id)}
				<div class="session-row">
					<div class="session-info">
						<span class="session-name">{s.hostname}</span>
						<span class="session-addr dim mono">
							{s.http_addr} · {s.session_id ? 'in another session' : 'on its own'}{s.show_name ? ` · ${s.show_name}` : ''}
						</span>
					</div>
					<button class="join-btn" onclick={() => (inviting = s)}>Invite</button>
				</div>
			{/each}
		</div>
	{/if}
</div>

{#if inviting}
	<Dialog title="Invite a station" onclose={() => (inviting = null)}>
		<p class="ask">
			<b>{inviting.hostname}</b> joins this session and takes this show.
			{#if inviting.show_name}
				The show it has open, <b>{inviting.show_name}</b>, is replaced by this one, and nothing
				can take that back.
			{/if}
			{#if inviting.session_id}
				It is in another session now, and leaves it.
			{/if}
		</p>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (inviting = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={invite} disabled={sending}>{sending ? 'Inviting…' : 'Invite'}</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.panel {
		background: #252525;
		border: 1px solid #333;
		border-radius: 6px;
		padding: 12px 14px;
	}

	.panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 10px;
	}

	.panel-title {
		font-size: 0.68rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #777;
	}

	.badge {
		font-size: 0.68rem;
		font-weight: 500;
		padding: 2px 6px;
		border-radius: 10px;
	}
	.badge--green { background: #14532d44; color: #4ade80; border: 1px solid #14532d; }
	.badge--blue  { background: #1e3a5f44; color: #60a5fa; border: 1px solid #1e3a5f; }
	.badge--dim   { background: #2a2a2a;   color: #555;    border: 1px solid #333; }

	.info-text {
		font-size: 0.82rem;
		color: #ccc;
		margin-bottom: 6px;
		line-height: 1.5;
	}

	.hint {
		font-size: 0.72rem;
		color: #555;
		font-style: italic;
		margin-bottom: 10px;
	}

	.empty-hint {
		font-size: 0.78rem;
		color: #555;
		font-style: italic;
		margin-bottom: 10px;
	}

	.sub-label {
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: #555;
		margin-bottom: 4px;
	}

	.session-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin-bottom: 10px;
	}

	.session-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		padding: 6px 8px;
		background: #1e1e1e;
		border: 1px solid #2e2e2e;
		border-radius: 4px;
	}

	.session-info {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.session-name {
		font-size: 0.82rem;
		color: #e0e0e0;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.session-addr { font-size: 0.68rem; }
	.dim   { color: #555; }
	.mono  { font-family: monospace; }

	.join-btn {
		font-size: 0.72rem;
		padding: 3px 10px;
		border-radius: 3px;
		border: 1px solid #4a9eff;
		background: transparent;
		color: #4a9eff;
		cursor: pointer;
		white-space: nowrap;
		flex-shrink: 0;
	}
	.join-btn:hover:not(:disabled) { background: #4a9eff22; }
	.join-btn:disabled { border-color: #444; color: #444; cursor: not-allowed; }

	.action-btn {
		font-size: 0.72rem;
		padding: 5px 8px;
		border-radius: 3px;
		border: 1px solid #444;
		background: transparent;
		color: #888;
		cursor: pointer;
		transition: all 0.15s;
	}
	.action-btn:hover:not(:disabled) { border-color: #888; color: #ccc; }
	.action-btn:disabled { cursor: not-allowed; opacity: 0.5; }

	.advertise-btn {
		border-color: #22c55e44;
		color: #22c55e;
	}
	.advertise-btn:hover:not(:disabled) {
		background: #22c55e11;
		border-color: #22c55e;
	}

	.leave-btn:hover:not(:disabled) { border-color: #ef4444; color: #ef4444; }

	.ask {
		font-size: 0.85rem;
		line-height: 1.5;
	}

	.join-at {
		display: flex;
		gap: 6px;
		margin-bottom: 4px;
	}
	.join-at input {
		flex: 1;
		min-width: 0;
		font-size: 0.72rem;
		padding: 3px 6px;
		background: #1e1e1e;
		border: 1px solid #333;
		border-radius: 3px;
		color: #ddd;
	}
</style>
