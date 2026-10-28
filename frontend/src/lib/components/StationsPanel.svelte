<script lang="ts">
	/**
	 * Who is in the session, who could be, and how this console reaches them.
	 *
	 * One table: the stations in the session, and below them in another color every
	 * other station on the network, each with Invite (bring it here) and Join (go
	 * there). There is no Advertise: every station announces itself whether or not it
	 * is in a session, so the act that used to make a session findable is not one an
	 * operator has to remember any more. Joining by address — for a network mDNS does
	 * not cross — is behind the ⋯.
	 *
	 * Half of a pair. This one is the *network*: which machines are here, which is
	 * leading, where each is reached, and what the link to it measures. What every
	 * machine is *costing* — CPU, memory, uptime, and what its output frames took —
	 * is the System panel, because those are the figures you read against a browser's
	 * and against each other, not against a sync address.
	 *
	 * Latency is in both, deliberately: it is the one figure that answers both
	 * questions, and it is measured from here either way.
	 */

	import { stationNames } from '$lib/stores/zones.js';
	import { onMount } from 'svelte';
	import Ellipsis from '@lucide/svelte/icons/ellipsis';
	import Dialog from './Dialog.svelte';
	import Help from './Help.svelte';
	import { show } from '$lib/stores/show.js';
	import { addToast } from '$lib/toasts.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import type { NearbyStation, PeerLink, SessionState, Station } from '$lib/generated/index.js';

	const client = getClientContext();
	const data = getDataContext();

	/// Matches REPORT_INTERVAL in infra::stations.
	const REPORT_INTERVAL_S = 2;

	let stations = $state<Station[]>([]);
	/** Known to the show and not here: see `stationNames`. Not one this panel lists as nearby. */
	const away = $derived(
		$stationNames.filter((s) => !s.online && !stations.some((t) => t.id === s.id) && !others.some((o) => o.node_id === s.id))
	);
	let links = $state<Record<string, PeerLink>>({});
	let thisStation = $state<string | null>(null);
	let session = $state<SessionState | null>(null);

	/** Every station on the network that is not already in this session. */
	const others = $derived(
		(session?.nearby ?? []).filter((n) => !stations.some((s) => s.id === n.node_id))
	);

	/**
	 * An invitation or a join waiting to be confirmed, since either replaces a show.
	 * `station` is `null` when it was asked for by hand, and then the dialog asks for
	 * the address: the other station's page for an invitation, its sync port for a join.
	 */
	type Asking = { kind: 'invite' | 'join'; station: NearbyStation | null };
	let asking = $state<Asking | null>(null);
	let sending = $state(false);
	/** The ⋯ menu: inviting and joining by hand, for a network mDNS does not cross. */
	let more = $state(false);
	let address = $state('');

	function ask(kind: 'invite' | 'join', station: NearbyStation | null) {
		more = false;
		address = '';
		asking = { kind, station };
	}

	/** The host part of an `ip:port`, for the IP column; the port is in the title. */
	const ipOf = (addr: string) => addr.replace(/:\d+$/, '');

	async function answer() {
		const ask = asking;
		if (!ask) return;
		const at = address.trim();
		if (!ask.station && !at) return;
		const name = ask.station?.hostname ?? at;
		sending = true;
		try {
			if (ask.kind === 'invite') {
				await client.call(
					'session.invite',
					ask.station ? { nodeId: ask.station.node_id } : { address: at }
				);
				addToast(`${name} joined`, 'success');
			} else {
				await client.call('session.joinAt', { address: ask.station?.sync_addr ?? at });
				addToast(`Joined ${name}`, 'success');
			}
			asking = null;
		} catch (e) {
			addToast(`${ask.kind === 'invite' ? 'Invite' : 'Join'} failed: ${e}`);
		} finally {
			sending = false;
		}
	}

	async function leave() {
		try {
			await client.call('session.leave');
		} catch (e) {
			addToast(`Leave failed: ${e}`);
		}
	}
	/// Ticks so "3s ago" keeps moving between reports.
	let now = $state(Date.now());

	const isSelf = (station: Station) => station.id === thisStation;

	/// A station is stale when it has missed a few of its own reports.
	function stale(station: Station): boolean {
		return (now - Date.parse(station.last_seen)) / 1000 > REPORT_INTERVAL_S * 3;
	}

	function heardAgo(station: Station): string {
		const seconds = Math.max(0, Math.round((now - Date.parse(station.last_seen)) / 1000));
		if (seconds < REPORT_INTERVAL_S * 2) return 'now';
		if (seconds < 60) return `${seconds}s ago`;
		return `${Math.round(seconds / 60)}m ago`;
	}

	/// Latency is measured from here, so there is only a number for other stations.
	function latency(station: Station): string {
		if (isSelf(station)) return '—';
		const link = links[station.id];
		if (!link || link.rtt_ms === null) return 'not connected';
		const rtt = `${link.rtt_ms.toFixed(1)} ms`;
		return link.unanswered > 0 ? `${rtt} · ${link.unanswered} missed` : rtt;
	}

	/**
	 * What a station says about the show clock.
	 *
	 * Three states rather than a figure, because two of them are zero and only one is
	 * right to be: a station that *is* the reference and one that has not managed to
	 * measure anything are both adding nothing. Printing "0.0 ms" for both is the
	 * plausible-wrong-number this whole mechanism exists to remove.
	 */
	function clock(station: Station): string {
		const c = station.clock;
		const said =
			c.state === 'Reference'
				? 'reference'
				: c.state === 'Uncorrected'
					? 'no offset yet'
					: `${c.offset_ms >= 0 ? '+' : ''}${c.offset_ms.toFixed(1)} ms` +
						(c.converging ? ' · converging' : '');
		// What that station says about itself, and what this one measures of it. The
		// second is the figure that answers "do these two consoles agree about the
		// time", and it is measured over the link from here — so there is one only for
		// somebody else, the way latency is.
		const link = isSelf(station) ? null : links[station.id];
		if (!link || link.offset_ms === null || link.offset_ms === undefined) return said;
		return `${said} · ${Math.abs(link.offset_ms).toFixed(1)} ms apart`;
	}

	/// A station driving a rig against a clock nobody has placed. It goes on driving
	/// it — a lamp cannot show a gap the way a panel can — which is exactly why the
	/// row has to say so.
	const uncorrected = $derived(stations.filter((s) => s.clock.state === 'Uncorrected'));

	/// Every station computes every fixture today, so this is all-or-nothing until
	/// parameter computation is partitioned.
	const fixtureShare = (station: Station) =>
		station.total_fixtures === 0
			? '—'
			: `${station.computes_fixtures} / ${station.total_fixtures}`;


	onMount(() => {
		const stop = data.stations.subscribeDeep((v) => { stations = v; });

		const applyLinks = (v: unknown) => {
			if (v && typeof v === 'object') links = v as Record<string, PeerLink>;
		};
		const applySession = (v: unknown) => {
			if (!v || typeof v !== 'object') return;
			const said = v as Partial<SessionState>;
			// `nearby` is new: a station on an older build answers without it.
			session = { ...(said as SessionState), nearby: said.nearby ?? [], discovered: said.discovered ?? [] };
			thisStation = session.node_id;
		};
		const stopLinks = client.subscribe('peers', applyLinks);
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => {
			client.get(['peers']).then(applyLinks);
			client.get(['session']).then(applySession);
			// The other stations on the network are looked for only while somebody is
			// looking: asked again on every connect, since a new socket is a new page.
			void client.call('session.watchNearby', { on: true }).catch(() => {});
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);
		const ticking = setInterval(() => { now = Date.now(); }, 1000);

		return () => {
			stop();
			stopLinks();
			stopSession();
			stopConnect();
			clearInterval(ticking);
			void client.call('session.watchNearby', { on: false }).catch(() => {});
		};
	});
</script>

<div class="stations">
	<section class="block">
		<header class="block-head">
			<h2>
				Stations
				<Help label="About this table">
					<p>
						Everything in white is in this session. Below it is every other station on the
						network: blue on its own, gray in a session of its own. <b>Invite</b> brings one here and it takes this show; <b>Join</b> takes
						this station there, and this one takes that show. Either replaces a show, so both
						ask first.
					</p>
					<p>
						Latency is measured from this station, so each console shows its own view of the
						network. What these machines are <em>costing</em> is the System panel.
					</p>
					<p>
						The clock column is what each station adds to its own to get show time. The leader
						is the reference and adds nothing; a follower measures the difference over the sync
						link and works it off gradually, because a clock that steps steps every running fade
						with it.
					</p>
					<p>
						Every station computes every fixture, which is what makes output the same on all of
						them without extra messages.
					</p>
				</Help>
			</h2>
			<span class="head-right">
				<span class="count">{stations.length} in the session</span>
				<span class="menu-anchor">
					<button class="more hit" aria-label="More" aria-haspopup="menu" aria-expanded={more} onclick={() => (more = !more)}>
						<Ellipsis size={16} />
					</button>
					{#if more}
						<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
						<div class="catch" onclick={() => (more = false)}></div>
						<div class="menu" role="menu">
							<button role="menuitem" onclick={() => ask('invite', null)}>Invite manually…</button>
							<button role="menuitem" disabled={!!session?.is_follower} onclick={() => ask('join', null)}>Join manually…</button>
						</div>
					{/if}
				</span>
			</span>
		</header>


		{#if stations.length === 0 && others.length === 0}
			<p class="empty">Nothing has reported yet.</p>
		{:else}
			<table class="rack">
				<thead>
					<tr>
						<th>Station</th><th>Show</th><th>IP</th><th>Role</th><th>Latency</th><th>Clock</th><th>Outputs</th>
						<th>Fixtures</th><th>Heard</th><th></th>
					</tr>
				</thead>
				<tbody>
					{#each stations as station (station.id)}
						<tr class:stale={stale(station)} class:self={isSelf(station)}>
							<td>
								<span class="name">{station.hostname}</span>
								{#if isSelf(station)}<span class="tag">this one</span>{/if}
							</td>
							<td>{$show?.name ?? '—'}</td>
							<td class="mono" title={`sync ${station.sync_addr}`}>{ipOf(station.sync_addr)}</td>
							<td>
								{#if station.is_leader}
									<span class="badge badge--green">Leader</span>
								{:else}
									<span class="badge badge--dim">Follower</span>
								{/if}
							</td>
							<td class="num">{latency(station)}</td>
							<td
								class="num"
								class:dim={station.clock.state === 'Reference'}
								class:warnt={station.clock.state === 'Uncorrected'}
							>{clock(station)}</td>
							<td>
								{#if station.output_plugins.length === 0}
									<span class="dim">none</span>
								{:else}
									{station.output_plugins.join(', ')}
								{/if}
							</td>
							<td class="num">{fixtureShare(station)}</td>
							<td class="num dim">{heardAgo(station)}</td>
							<td class="acts">
								{#if isSelf(station) && session?.is_follower}
									<button class="btn btn-small" onclick={leave}>Leave</button>
								{/if}
							</td>
						</tr>
					{/each}
					<!-- Stations the show has seen and remembers, that are not here now: listed
					     so a spare that is off is still in the picture. Mapped in Setup → Audio,
					     and forgotten there. -->
					{#each away as gone (gone.id)}
						<tr class="other elsewhere">
							<td><span class="name">{gone.name}</span></td>
							<td>{$show?.name ?? '—'}</td>
							<td class="mono dim">—</td>
							<td><span class="badge badge--dim">Offline</span></td>
							{#each { length: 5 } as _, i (i)}<td class="num dim">—</td>{/each}
							<td class="acts"></td>
						</tr>
					{/each}
					{#each others as other (other.node_id)}
						<tr class="other" class:elsewhere={other.follows || !!other.session_id}>
							<td><span class="name">{other.hostname}</span></td>
							<td>{other.show_name || '—'}</td>
							<td class="mono" title={`page ${other.http_addr}${other.sync_addr ? ` · sync ${other.sync_addr}` : ''}`}>{ipOf(other.http_addr)}</td>
							<td>
								<span class="badge" class:badge--blue={!other.follows && !other.session_id} class:badge--dim={other.follows || !!other.session_id}>
									{other.follows ? 'Follows another' : other.session_id ? 'Leads another' : 'On its own'}
								</span>
							</td>
							<!-- Latency, clock and the rest are measured over a session's link, and
							     there is none to a station outside it. -->
							{#each { length: 5 } as _, i (i)}<td class="num dim">—</td>{/each}
							<td class="acts">
								<button class="btn btn-small" onclick={() => ask('invite', other)}>Invite</button>
								<button
									class="btn btn-small"
									disabled={other.follows || !other.sync_addr || !!session?.is_follower}
									title={other.follows
										? 'It follows another station: join that one'
										: session?.is_follower
											? 'Leave this session first'
											: undefined}
									onclick={() => ask('join', other)}
								>Join</button>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>

			{#if uncorrected.length > 0}
				<p class="warn">
					{uncorrected.length === 1
						? `${uncorrected[0].hostname} has`
						: `${uncorrected.length} stations have`} no offset to the leader's clock yet, and
					{uncorrected.length === 1 ? 'is' : 'are'} running on
					{uncorrected.length === 1 ? 'its' : 'their'} own. Every fade is anchored in an absolute
					millisecond, so until that is measured this console and that one disagree about where a
					running cue has got to — by however far apart their clocks are.
				</p>
			{/if}
		{/if}
	</section>
</div>

{#if asking}
	<Dialog title={asking.kind === 'invite' ? 'Invite a station' : 'Join a station'} onclose={() => (asking = null)}>
		<div class="ask">
			{#if !asking.station}
				<form onsubmit={(e) => { e.preventDefault(); answer(); }}>
					<input
						class="mono"
						placeholder={asking.kind === 'invite' ? 'host:port of its page, e.g. 10.0.0.2:7700' : 'host:sync-port, e.g. 10.0.0.2:7701'}
						aria-label="The other station's address"
						bind:value={address}
					/>
				</form>
				<p class="dim small">
					{asking.kind === 'invite'
						? 'The address its page is served from: port 7700 unless it was started with another.'
						: 'Its sync port: 7701 unless it was started with another.'}
					Only needed where a station is not listed, as on a switch that drops multicast.
				</p>
			{/if}
			<p>
				{#if asking.kind === 'invite'}
					<b>{asking.station?.hostname ?? 'That station'}</b> joins this session and takes this show.
					{#if asking.station?.show_name}
						The show it has open, <b>{asking.station.show_name}</b>, is replaced, and nothing can
						take that back.
					{:else if !asking.station}
						Whatever show it has open is replaced, and nothing can take that back.
					{/if}
					{#if asking.station?.session_id}It leaves the session it is in.{/if}
				{:else}
					This station joins <b>{asking.station?.hostname ?? 'that station'}</b> and takes its
					show{asking.station?.show_name ? `, ${asking.station.show_name}` : ''}.
					{#if $show}
						The show open here, <b>{$show.name}</b>, is replaced, and nothing can take that back.
					{/if}
				{/if}
			</p>
		</div>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (asking = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={answer} disabled={sending || (!asking?.station && !address.trim())}>
				{sending ? '…' : asking?.kind === 'invite' ? 'Invite' : 'Join'}
			</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.stations { padding: 16px 20px; }
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.count { color: #777; font-size: 12px; }
	.rack { width: 100%; border-collapse: collapse; font-size: 13px; }
	.rack th { text-align: left; color: #777; font-weight: 500; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; padding: 0 8px 6px 0; }
	.rack td { padding: 5px 8px 5px 0; vertical-align: middle; border-top: 1px solid #262626; }
	.rack tr.stale td { opacity: 0.45; }
	.rack tr.self td { background: #1c2128; }
	.name { font-weight: 500; }
	.tag { background: #2a2a2a; border: 1px solid #3a3a3a; border-radius: 9px; color: #999; font-size: 10px; padding: 1px 6px; margin-left: 6px; }
	.num { font-variant-numeric: tabular-nums; }
	.dim { color: #666; }
	.warnt { color: #fbbf24; }
	.mono { font-family: monospace; }
	.badge { font-size: 0.68rem; font-weight: 500; padding: 2px 7px; border-radius: 10px; }
	.badge--green { background: #14532d44; color: #4ade80; border: 1px solid #14532d; }
	.badge--dim { background: #2a2a2a; color: #777; border: 1px solid #333; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	h2 { display: inline-flex; align-items: center; gap: 6px; }
	.head-right { display: inline-flex; align-items: center; gap: 10px; }
	.more { display: inline-flex; padding: 2px; border: none; background: none; color: #888; cursor: pointer; }
	.more:hover, .more[aria-expanded='true'] { color: #ddd; }
	.menu-anchor { position: relative; display: inline-flex; }
	.catch { position: fixed; inset: 0; z-index: 19; }
	.menu { position: absolute; top: calc(100% + 4px); right: 0; z-index: 20; min-width: 180px; padding: 4px; background: #222; border: 1px solid #3a3a3a; border-radius: 6px; box-shadow: 0 6px 20px rgb(0 0 0 / 0.4); display: flex; flex-direction: column; }
	.menu button { text-align: left; padding: 7px 10px; border: none; border-radius: 4px; background: none; color: #ddd; font-size: 13px; cursor: pointer; }
	.menu button:hover:not(:disabled) { background: #2e2e2e; }
	.menu button:disabled { color: #555; cursor: default; }
	.ask input { width: 100%; font-size: 13px; padding: 6px 8px; background: #1e1e1e; border: 1px solid #333; border-radius: 4px; color: #ddd; margin-bottom: 6px; }
	.ask p + p { margin-top: 10px; }
	.small { font-size: 12px; }
	.rack tr.elsewhere td { color: #888; }
	.rack tr.other td { color: #7fb3ff; border-top-style: dashed; }
	.badge--blue { background: #1e3a5f44; color: #60a5fa; border: 1px solid #1e3a5f; }
	.acts { text-align: right; white-space: nowrap; }
	.acts .btn + .btn { margin-left: 4px; }
	.btn-small { min-height: 24px; padding: 0 10px; font-size: 12px; }
	.ask { margin: 0; font-size: 0.85rem; line-height: 1.5; }
	.ask p { margin: 0; }
	.warn { color: #e08a55; font-size: 12px; margin-top: 10px; }
</style>
