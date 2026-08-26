<script lang="ts">
	import { onMount } from 'svelte';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import type { PeerLink, Station } from '$lib/generated/index.js';

	const client = getClientContext();
	const data = getDataContext();

	/// Matches REPORT_INTERVAL in infra::stations.
	const REPORT_INTERVAL_S = 2;

	let stations = $state<Station[]>([]);
	let links = $state<Record<string, PeerLink>>({});
	let thisStation = $state<string | null>(null);
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

	const memPercent = (station: Station) =>
		station.mem_total === 0n ? 0 : (Number(station.mem_used) / Number(station.mem_total)) * 100;

	const megabytes = (bytes: bigint) => `${Math.round(Number(bytes) / 1_000_000)} MB`;

	function uptime(station: Station): string {
		const s = Number(station.uptime_s);
		if (s < 60) return `${s}s`;
		if (s < 3600) return `${Math.floor(s / 60)}m`;
		return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
	}

	/// Every station computes every fixture today, so this is all-or-nothing until
	/// parameter computation is partitioned.
	const fixtureShare = (station: Station) =>
		station.total_fixtures === 0
			? '—'
			: `${station.computes_fixtures} / ${station.total_fixtures}`;

	const partitioned = $derived(
		stations.some((s) => s.computes_fixtures !== s.total_fixtures)
	);

	onMount(() => {
		const stop = data.stations.subscribeDeep((v) => { stations = v; });

		const applyLinks = (v: unknown) => {
			if (v && typeof v === 'object') links = v as Record<string, PeerLink>;
		};
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') thisStation = (v as { node_id: string | null }).node_id;
		};
		const stopLinks = client.subscribe('peers', applyLinks);
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => {
			client.get(['peers']).then(applyLinks);
			client.get(['session']).then(applySession);
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);
		const ticking = setInterval(() => { now = Date.now(); }, 1000);

		return () => { stop(); stopLinks(); stopSession(); stopConnect(); clearInterval(ticking); };
	});
</script>

<div class="stations">
	<section class="block">
		<header class="block-head">
			<h2>Stations</h2>
			<span class="count">{stations.length} in the session</span>
		</header>

		{#if stations.length === 0}
			<p class="empty">Nothing has reported yet.</p>
		{:else}
			<table class="rack">
				<thead>
					<tr>
						<th>Station</th><th>Role</th><th>Latency</th><th>CPU</th><th>Memory</th>
						<th>Up</th><th>Outputs</th><th>Fixtures</th><th>Heard</th>
					</tr>
				</thead>
				<tbody>
					{#each stations as station (station.id)}
						<tr class:stale={stale(station)} class:self={isSelf(station)}>
							<td>
								<span class="name">{station.hostname}</span>
								{#if isSelf(station)}<span class="tag">this one</span>{/if}
								<span class="addr mono">{station.sync_addr}</span>
							</td>
							<td>
								{#if station.is_leader}
									<span class="badge badge--green">Leader</span>
								{:else}
									<span class="badge badge--dim">Follower</span>
								{/if}
							</td>
							<td class="num">{latency(station)}</td>
							<td class="num">{station.cpu_percent.toFixed(1)}%</td>
							<td class="num" title="{megabytes(station.mem_used)} of {megabytes(station.mem_total)}">
								{memPercent(station).toFixed(1)}%
							</td>
							<td class="num">{uptime(station)}</td>
							<td>
								{#if station.output_plugins.length === 0}
									<span class="dim">none</span>
								{:else}
									{station.output_plugins.join(', ')}
								{/if}
							</td>
							<td class="num">{fixtureShare(station)}</td>
							<td class="num dim">{heardAgo(station)}</td>
						</tr>
					{/each}
				</tbody>
			</table>

			{#if stations.some(stale)}
				<p class="warn">
					A greyed station has missed several of its own reports. Its row is kept until it has
					been quiet for half a minute.
				</p>
			{/if}
			{#if !partitioned}
				<p class="note">
					Every station computes every fixture — playback runs everywhere, which is what makes
					output the same on all of them without extra messages. The fixture column becomes
					interesting when that is partitioned.
				</p>
			{/if}
			<p class="note">
				Latency is measured from this station, so each console shows its own view of the
				network rather than a shared one.
			</p>
		{/if}
	</section>
</div>

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
	.addr { color: #666; font-size: 11px; margin-left: 8px; }
	.tag { background: #2a2a2a; border: 1px solid #3a3a3a; border-radius: 9px; color: #999; font-size: 10px; padding: 1px 6px; margin-left: 6px; }
	.num { font-variant-numeric: tabular-nums; }
	.dim { color: #666; }
	.mono { font-family: monospace; }
	.badge { font-size: 0.68rem; font-weight: 500; padding: 2px 7px; border-radius: 10px; }
	.badge--green { background: #14532d44; color: #4ade80; border: 1px solid #14532d; }
	.badge--dim { background: #2a2a2a; color: #777; border: 1px solid #333; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.warn { color: #e08a55; font-size: 12px; margin-top: 10px; }
	.note { color: #666; font-size: 12px; margin-top: 8px; font-style: italic; }
</style>
