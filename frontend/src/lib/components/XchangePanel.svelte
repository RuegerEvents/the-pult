<script lang="ts">
	/**
	 * MVR-xchange: who is in the group, what they have, and the two acts.
	 *
	 * Its own panel rather than a sheet on Rig tools, because it is a live list of
	 * other people's software and a strip of buttons is not that. What it draws is
	 * the LOCAL `xchange` path, which every station holds — the one running the
	 * exchange publishes it and the sync layer pushes it to the rest — so this panel
	 * is the same on the booth desk as on the tablet at the back.
	 *
	 * Nothing here works anything out for itself. `idle` says why the exchange is not
	 * running and `running` says whether it is; a second opinion formed in the browser
	 * would be a second answer that drifts, which is the rule the System panel's
	 * `struggling()` already follows.
	 */
	import { onMount } from 'svelte';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import type { XchangeState } from '$lib/generated/XchangeState';
	import type { XchangeSettings } from '$lib/generated/XchangeSettings';
	import type { XchangeIdle } from '$lib/generated/XchangeIdle';

	const client = getClientContext();
	const data = getDataContext();

	const EMPTY: XchangeState = {
		running: false,
		idle: null,
		on_station: '',
		settings: { enabled: false, group: 'Default', mode: 'Tcp', url: '' },
		station_uuid: '',
		station_name: '',
		address: '',
		stations: [],
		commits: [],
		pending_host: null,
	};

	let xchange = $state<XchangeState>(EMPTY);
	let comment = $state('');
	let busy = $state(false);
	let applying = $state<string | null>(null);
	/** A commit waiting on somebody saying yes while a sequence is live. */
	let confirming = $state<{ fileUuid: string; running: string[] } | null>(null);

	/** Why it is not running, in the words a person can act on. */
	function why(idle: XchangeIdle | null): string {
		if (idle === null || idle === undefined) return 'Not running.';
		if (idle === 'NotEnabled') return 'Switched off for this show.';
		if (idle === 'RefusedByStation') return "This station's preferences do not allow it.";
		if (idle === 'AnotherStationIsLeading')
			return `Running on ${xchange.on_station || 'another station'} — you can still use it from here.`;
		if (idle === 'NoShow') return 'No show is open.';
		if (typeof idle === 'object' && 'Failed' in idle) return idle.Failed;
		return 'Not running.';
	}

	async function settings(next: Partial<XchangeSettings>) {
		busy = true;
		try {
			await data.show.mvr_xchange.set({ ...xchange.settings, ...next });
		} catch (e) {
			addToast(`Could not change the exchange: ${e}`);
		} finally {
			busy = false;
		}
	}

	async function commit() {
		if (!comment.trim()) {
			addToast('A commit needs a comment — an empty one is one nobody can tell from the last.', 'warning');
			return;
		}
		busy = true;
		try {
			await client.call('xchange.commit', { comment });
			comment = '';
		} catch (e) {
			addToast(`Commit failed: ${e}`);
		} finally {
			busy = false;
		}
	}

	async function apply(fileUuid: string, confirmed = false) {
		applying = fileUuid;
		try {
			const answer = (await client.call('xchange.apply', { fileUuid, confirm: confirmed })) as {
				needsConfirm?: boolean;
				running?: string[];
			};
			// The station will not decide whether the house is in. It says what is
			// live and hands the decision back to whoever can see the stage.
			if (answer?.needsConfirm) {
				confirming = { fileUuid, running: answer.running ?? [] };
				return;
			}
			confirming = null;
			addToast('Applied. Ctrl-Z takes it back.', 'success');
		} catch (e) {
			addToast(`Could not apply: ${e}`);
		} finally {
			applying = null;
		}
	}

	async function followHost(follow: boolean) {
		busy = true;
		try {
			await client.call('xchange.followHost', { follow });
		} catch (e) {
			addToast(`Could not answer: ${e}`);
		} finally {
			busy = false;
		}
	}

	function size(bytes: number): string {
		if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
		if (bytes >= 1024) return `${Math.round(bytes / 1024)} kB`;
		return `${bytes} B`;
	}

	function when(atMs: number): string {
		if (!atMs) return '';
		return new Date(atMs).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
	}

	onMount(() => {
		const take = (v: unknown) => {
			if (v && typeof v === 'object') xchange = v as XchangeState;
		};
		const unsub = client.subscribe('xchange', take);
		const fetchNow = () => client.get(['xchange']).then(take);
		fetchNow();
		const unsubConnect = client.addConnectListener(fetchNow);
		return () => {
			unsub();
			unsubConnect();
		};
	});
</script>

<div class="panel">
	<div class="panel-header">
		<span class="panel-title">MVR-xchange</span>
		{#if xchange.running}
			<span class="badge badge--green">● In {xchange.settings.group}</span>
		{:else if xchange.idle === 'AnotherStationIsLeading'}
			<span class="badge badge--blue">● Elsewhere</span>
		{:else}
			<span class="badge badge--dim">○ Off</span>
		{/if}
	</div>

	{#if !xchange.running}
		<p class="info-text">{why(xchange.idle)}</p>
	{/if}

	<!-- ── The group ── -->
	<div class="group-row">
		<label class="field">
			<span class="field-label">Group</span>
			<input
				class="field-input"
				value={xchange.settings.group}
				disabled={busy}
				onchange={(e) => settings({ group: (e.currentTarget as HTMLInputElement).value })}
			/>
		</label>
		<label class="field field--narrow">
			<span class="field-label">Mode</span>
			<select
				class="field-input"
				value={xchange.settings.mode}
				disabled={busy}
				onchange={(e) =>
					settings({ mode: (e.currentTarget as HTMLSelectElement).value as XchangeSettings['mode'] })}
			>
				<option value="Tcp">Local network</option>
				<option value="WebSocket">Join a host</option>
				<option value="WebSocketHost">Host here</option>
			</select>
		</label>
	</div>

	{#if xchange.settings.mode === 'WebSocket'}
		<label class="field">
			<span class="field-label">Host</span>
			<input
				class="field-input"
				placeholder="ws://previz.local:8080/mvrxchange"
				value={xchange.settings.url}
				disabled={busy}
				onchange={(e) => settings({ url: (e.currentTarget as HTMLInputElement).value })}
			/>
		</label>
	{/if}

	<button
		class="action-btn"
		class:leave-btn={xchange.settings.enabled}
		disabled={busy}
		onclick={() => settings({ enabled: !xchange.settings.enabled })}
	>
		{xchange.settings.enabled ? 'Switch off' : 'Switch on'}
	</button>

	{#if xchange.running}
		<p class="hint">
			On the network as <span class="mono">{xchange.station_name}</span>
			{#if xchange.address}<span class="dim"> · {xchange.address}</span>{/if}
		</p>
	{/if}

	<!-- ── Somebody wants this group to move ── -->
	{#if xchange.pending_host}
		<div class="pending">
			<p class="pending-text">
				<strong>{xchange.pending_host.from_name || 'A station'}</strong> wants this group to move to
				<span class="mono">{xchange.pending_host.service_url || xchange.pending_host.service_name}</span>.
			</p>
			<p class="hint">Nothing has moved. This came from the network, not from a person you know.</p>
			<div class="pending-actions">
				<button class="join-btn" disabled={busy} onclick={() => followHost(true)}>Follow</button>
				<button class="action-btn leave-btn" disabled={busy} onclick={() => followHost(false)}>
					Stay
				</button>
			</div>
		</div>
	{/if}

	<!-- ── Who is here ── -->
	{#if xchange.stations.length > 0}
		<p class="sub-label">In the group</p>
		<div class="rows">
			{#each xchange.stations as station (station.station_uuid)}
				<div class="row">
					<div class="row-info">
						<span class="row-name">
							{station.station_name}
							{#if station.is_us}<span class="dim"> (this console)</span>{/if}
						</span>
						<span class="dim mono small">
							{station.provider}{station.provider && station.address ? ' · ' : ''}{station.address}
						</span>
					</div>
					{#if !station.joined && !station.is_us}
						<span class="badge badge--dim">seen</span>
					{/if}
				</div>
			{/each}
		</div>
	{:else if xchange.running}
		<p class="empty-hint">Nobody else in this group yet.</p>
	{/if}

	<!-- ── Committing ── -->
	{#if xchange.running}
		<p class="sub-label">Commit this rig</p>
		<div class="commit-row">
			<input
				class="field-input"
				placeholder="What changed?"
				bind:value={comment}
				disabled={busy}
				onkeydown={(e) => {
					if (e.key === 'Enter') commit();
				}}
			/>
			<button class="join-btn" disabled={busy || !comment.trim()} onclick={commit}>Commit</button>
		</div>
		<p class="hint">The whole rig, to everyone in the group. Nothing is sent until somebody asks.</p>
	{/if}

	<!-- ── What there is ── -->
	{#if xchange.commits.length > 0}
		<p class="sub-label">Commits</p>
		<div class="rows">
			{#each xchange.commits as commit (commit.file_uuid)}
				<div class="row">
					<div class="row-info">
						<span class="row-name">{commit.comment || '(no comment)'}</span>
						<span class="dim mono small">
							{commit.ours ? 'this console' : commit.station_name || 'a station that has gone'}
							· {size(Number(commit.file_size))}
							{#if when(Number(commit.at_ms))} · {when(Number(commit.at_ms))}{/if}
							{#if commit.here} · here{/if}
						</span>
					</div>
					{#if !commit.ours}
						<button
							class="join-btn"
							disabled={applying !== null || !xchange.running}
							onclick={() => apply(commit.file_uuid)}
						>
							{applying === commit.file_uuid ? '…' : 'Apply'}
						</button>
					{/if}
				</div>

				{#if confirming?.fileUuid === commit.file_uuid}
					<div class="pending">
						<p class="pending-text">
							{confirming.running.join(', ')}
							{confirming.running.length === 1 ? 'is' : 'are'} running. Applying this repatches the rig.
						</p>
						<div class="pending-actions">
							<button class="join-btn" onclick={() => apply(commit.file_uuid, true)}>
								Apply anyway
							</button>
							<button class="action-btn leave-btn" onclick={() => (confirming = null)}>Cancel</button>
						</div>
					</div>
				{/if}
			{/each}
		</div>
	{:else if xchange.running}
		<p class="empty-hint">Nothing has been committed to this group.</p>
	{/if}
</div>

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
	.badge--green {
		background: #14532d44;
		color: #4ade80;
		border: 1px solid #14532d;
	}
	.badge--blue {
		background: #1e3a5f44;
		color: #60a5fa;
		border: 1px solid #1e3a5f;
	}
	.badge--dim {
		background: #2a2a2a;
		color: #777;
		border: 1px solid #333;
	}

	.info-text {
		font-size: 0.8rem;
		color: #bbb;
		margin: 0 0 10px;
	}

	.hint {
		font-size: 0.7rem;
		color: #777;
		margin: 6px 0 10px;
	}

	.empty-hint {
		font-size: 0.75rem;
		color: #666;
		margin: 8px 0;
	}

	.sub-label {
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: #666;
		margin: 14px 0 6px;
	}

	.group-row {
		display: flex;
		gap: 8px;
	}

	.field {
		display: block;
		flex: 1;
		margin-bottom: 8px;
	}
	.field--narrow {
		flex: 0 0 40%;
	}

	.field-label {
		display: block;
		font-size: 0.65rem;
		color: #777;
		margin-bottom: 3px;
	}

	.field-input {
		width: 100%;
		box-sizing: border-box;
		background: #1c1c1c;
		border: 1px solid #383838;
		border-radius: 4px;
		color: #ddd;
		font-size: 0.78rem;
		padding: 5px 7px;
	}
	.field-input:disabled {
		opacity: 0.5;
	}

	.action-btn {
		width: 100%;
		background: #2f6b3f;
		border: 1px solid #3d8a52;
		border-radius: 4px;
		color: #e8f5ec;
		cursor: pointer;
		font-size: 0.78rem;
		padding: 6px 10px;
	}
	.action-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.leave-btn {
		background: #3a2a2a;
		border-color: #5a3a3a;
		color: #e5c5c5;
	}

	.join-btn {
		background: #2a3a55;
		border: 1px solid #3c5580;
		border-radius: 4px;
		color: #cfe0ff;
		cursor: pointer;
		font-size: 0.72rem;
		padding: 4px 10px;
		white-space: nowrap;
	}
	.join-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}

	.rows {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		background: #1e1e1e;
		border: 1px solid #303030;
		border-radius: 4px;
		padding: 6px 8px;
	}

	.row-info {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}

	.row-name {
		font-size: 0.78rem;
		color: #ddd;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.commit-row {
		display: flex;
		gap: 6px;
		align-items: center;
	}

	.pending {
		background: #2e2a1c;
		border: 1px solid #5a4a20;
		border-radius: 4px;
		margin: 8px 0;
		padding: 8px 10px;
	}

	.pending-text {
		font-size: 0.78rem;
		color: #e8dcc0;
		margin: 0 0 4px;
	}

	.pending-actions {
		display: flex;
		gap: 6px;
	}

	.small {
		font-size: 0.68rem;
	}
	.dim {
		color: #777;
	}
	.mono {
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
	}
</style>
