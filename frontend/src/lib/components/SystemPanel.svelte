<script lang="ts">
	/**
	 * What the console is costing, everywhere it is running.
	 *
	 * The other half of the pair the Stations panel is: that one is *who is here* —
	 * leader, addresses, the links between them — and this one is *what it costs*.
	 * Latency appears in both deliberately, because it is the one figure that belongs
	 * to each question.
	 *
	 * Three kinds of number, and they must not be confused with one another:
	 *
	 * - **What the process costs**: CPU, memory, uptime. Replicated on the `stations`
	 *   row, so every console sees every station's.
	 * - **What a frame costs**, per output connector, from `Station::frame_costs`.
	 *   One line per connector rather than one figure per station: Art-Net drawing at
	 *   40 Hz beside an OpenHaunt node that was told about a fade once are not two
	 *   samples of one number. A connector that emitted nothing carries no entry at
	 *   all, and absent is rendered as absent — zero would read as "instant".
	 * - **What a browser costs**, from the LOCAL `clients` map. A console *is* a
	 *   browser evaluating a rig in wasm at frame rate, and the machine struggling in
	 *   a room where every station is comfortable can be the tablet at the back of it.
	 *
	 * The sparklines are this tile's own memory. Nothing on the wire carries a series —
	 * every report is one closed window — so the panel keeps the last {@link TRACE_LENGTH}
	 * readings it *witnessed* and draws those. Which means a line starts empty when the
	 * tile is opened and covers only what this tile saw, and the panel says so rather
	 * than implying a record it does not have.
	 *
	 * The browsers listed are the ones connected to *this* station, because the map is
	 * LOCAL. That is the deliberate answer to the question the roadmap left open: a
	 * fault is occasional and a frame rate is every second, so what crosses to the
	 * other consoles is the exception — a struggling page writes a `warn`, which the
	 * System Log carries everywhere — rather than a row per browser per report for
	 * ever on the same network as the Art-Net.
	 */

	import { onMount } from 'svelte';
	import Help from './Help.svelte';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import type { ClientStats, FrameCost, PeerLink, Station } from '$lib/generated/index.js';
	import { SLOW_FPS, STALL_MS, fpsOf, struggling, thisBrowser } from '$lib/stats.js';
	import { TRACE_LENGTH, Traces, sparkline } from '$lib/trace.js';
	import { drivingReport } from '$lib/stores/timelines.js';

	const client = getClientContext();
	const data = getDataContext();

	/// Matches REPORT_INTERVAL in infra::stations, and the browser's own in stats.ts.
	const REPORT_INTERVAL_S = 2;

	let stations = $state<Station[]>([]);
	let links = $state<Record<string, PeerLink>>({});
	let clients = $state<Record<string, ClientStats>>({});
	let thisStation = $state<string | null>(null);
	/// Ticks so "3s ago" keeps moving between reports.
	let now = $state(Date.now());

	const isSelf = (station: Station) => station.id === thisStation;
	const stale = (atMs: number) => (now - atMs) / 1000 > REPORT_INTERVAL_S * 3;

	/**
	 * The nodes with a loudspeaker, from the station driving them — the only one that
	 * streams to them and hears their `audio/status` — so every console reads the same.
	 */
	const speakerNodes = $derived(
		Object.entries($drivingReport?.nodes ?? {})
			.flatMap(([serial, n]) => (n ? [{ serial, ...n }] : []))
			.sort((a, b) => a.name.localeCompare(b.name))
	);
	/** How long ago a show-clock moment was, said the way a log line is. */
	function since(atMs: number): string {
		const seconds = Math.max(0, Math.round((now - atMs) / 1000));
		if (seconds < 60) return `${seconds}s ago`;
		if (seconds < 3600) return `${Math.round(seconds / 60)}m ago`;
		return `${(seconds / 3600).toFixed(1)}h ago`;
	}

	function ago(atMs: number): string {
		const seconds = Math.max(0, Math.round((now - atMs) / 1000));
		if (seconds < REPORT_INTERVAL_S * 2) return 'now';
		if (seconds < 60) return `${seconds}s ago`;
		return `${Math.round(seconds / 60)}m ago`;
	}

	const megabytes = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

	/// Bigger quantities read better in their own unit; a disk is not megabytes.
	function size(bytes: number): string {
		if (bytes >= 1_000_000_000_000) return `${(bytes / 1_000_000_000_000).toFixed(1)} TB`;
		if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
		return megabytes(bytes);
	}

	const percent = (part: number, whole: number) => (whole === 0 ? 0 : (part / whole) * 100);

	/**
	 * Is the machine, rather than the console, the thing in trouble?
	 *
	 * The pair is the point of having both: a station at 4% on a machine at 96% is not
	 * a comfortable station, it is one about to be starved by something nobody is
	 * looking at. Memory and disk are here too because both end a show rather than
	 * slowing it — a full disk is a show that cannot be saved.
	 */
	/// The console's CPU as a share of the whole machine, so it can be read against
	/// `machine.cpu_percent` — which is what the pair is for. A process percentage is
	/// of one core; dividing by the count puts both on the machine's scale.
	const consoleShareOfMachine = (station: Station) =>
		station.machine.cores === 0 ? 0 : station.cpu_percent / station.machine.cores;

	function machineStrained(station: Station): boolean {
		const m = station.machine;
		return (
			m.cpu_percent > 90 ||
			percent(m.mem_used, station.mem_total) > 90 ||
			(m.disk_total > 0 && m.disk_free / m.disk_total < 0.05)
		);
	}

	const memPercent = (station: Station) =>
		station.mem_total === 0 ? 0 : (station.mem_used / station.mem_total) * 100;

	function uptime(seconds: number): string {
		if (seconds < 60) return `${seconds}s`;
		if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
		return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
	}

	/// Latency is measured from here, so there is only a number for other stations.
	function latency(station: Station): string {
		if (isSelf(station)) return '—';
		const link = links[station.id];
		if (!link || link.rtt_ms === null) return 'not connected';
		return `${link.rtt_ms.toFixed(1)} ms`;
	}

	/// How often a connector emitted, read off the window rather than stored.
	const rateOf = (cost: FrameCost) =>
		cost.window_ms === 0 ? 0 : (cost.frames * 1000) / cost.window_ms;

	/**
	 * A throughput, in the unit that reads: bytes on a quiet link, kilobytes on a
	 * working one, megabytes on a rig at full tilt.
	 *
	 * Decimal rather than binary, because a network is sold and specified in decimal:
	 * a 100 Mbit line carries 12.5 MB/s by this reckoning and by the one on its box.
	 */
	function perSecond(bytes: number): string {
		if (bytes < 1) return '—';
		if (bytes < 1000) return `${Math.round(bytes)} B/s`;
		if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB/s`;
		return `${(bytes / 1_000_000).toFixed(2)} MB/s`;
	}

	const bytesPerSecondOf = (window_ms: number, bytes: number) =>
		window_ms === 0 ? 0 : (bytes * 1000) / window_ms;

	/// What a connector put on the wire, per second, over the window it reported.
	const wireOf = (cost: FrameCost) => bytesPerSecondOf(cost.window_ms, cost.bytes);

	/**
	 * Everything this station's connectors put on the wire, per second.
	 *
	 * A sum over connectors, which is meaningful in a way a sum of their frame *times*
	 * would not be: two connectors' frames overlap in time and cannot be added, but
	 * their bytes go down the same cable and can.
	 */
	const outputWire = (station: Station) =>
		station.frame_costs.reduce((total, cost) => total + wireOf(cost), 0);

	/// What the peer links from here carried, per second, both ways.
	const syncWire = (station: Station) => {
		const link = links[station.id];
		return link ? bytesPerSecondOf(link.window_ms, link.sent_bytes + link.received_bytes) : 0;
	};

	/// And what the machine's own interfaces carried — everything, this console or not.
	const machineWire = (station: Station) =>
		bytesPerSecondOf(station.net_window_ms, station.net_received + station.net_sent);

	/**
	 * The share of a frame spent on the wire rather than working out what the rig is
	 * doing. Two halves that scale differently is the whole reason both are published.
	 */
	const sendingOf = (cost: FrameCost) => Math.max(0, cost.mean_ms - cost.evaluating_mean_ms);

	const browsers = $derived(
		Object.values(clients).sort((a, b) => a.session.localeCompare(b.session))
	);

	/**
	 * What this station is sending its browsers, per second, all of them together.
	 *
	 * Only ever this station's: the `clients` map is LOCAL, so a peer's browsers are
	 * not here to be summed and the column says so rather than showing a nought.
	 */
	const browserWire = $derived(
		browsers.reduce(
			(total, row) => total + bytesPerSecondOf(row.sent_window_ms, row.sent_bytes),
			0
		)
	);

	// ── What this tile has watched ────────────────────────────────────────────
	//
	// The traces are plain objects mutated in place rather than `$state`: they hold a
	// few hundred numbers and making the arrays themselves reactive would have Svelte
	// re-proxy every one of them on every report. What *is* reactive is the finished
	// SVG path per line, written out by the effect below.
	const frameTraces = new Traces();
	const browserTraces = new Traces();

	/// Sixty frames a second is the rate a page is trying to hit, so it is the scale
	/// every browser's line is drawn against — which is what makes two comparable.
	const FPS_CEILING = 60;
	const SPARK_W = 64;
	const SPARK_H = 14;

	let framePaths = $state<Record<string, string | null>>({});
	let browserPaths = $state<Record<string, string | null>>({});

	/**
	 * One reading per connector and per browser, per report, and the paths that draw
	 * them — keyed so a connector removed from the show or a tab that closed leaves no
	 * line behind.
	 *
	 * This effect reads `stations` and `browsers` and writes only the two path maps.
	 * It must stay that way: an earlier version ticked a counter it also read, which
	 * is `effect_update_depth_exceeded` and takes the whole page down rather than the
	 * panel — every tile in the workspace went blank.
	 */
	$effect(() => {
		const frames: Record<string, string | null> = {};
		const frameKeys: string[] = [];
		for (const station of stations) {
			const stamp = Date.parse(station.last_seen);
			for (const cost of station.frame_costs) {
				const key = `${station.id}/${cost.output}`;
				frameKeys.push(key);
				frameTraces.push(key, cost.mean_ms, stamp);
				frames[key] = sparkline(frameTraces.points(key), SPARK_W, SPARK_H);
			}
		}
		frameTraces.keep(frameKeys);
		framePaths = frames;

		const rates: Record<string, string | null> = {};
		for (const row of browsers) {
			// A window that drew nothing is not a reading of zero; it is the absence of
			// one, and the line simply does not advance.
			if (row.frames) browserTraces.push(row.session, fpsOf(row.frames), row.at_ms);
			rates[row.session] = sparkline(
				browserTraces.points(row.session),
				SPARK_W,
				SPARK_H,
				FPS_CEILING
			);
		}
		browserTraces.keep(browsers.map((b) => b.session));
		browserPaths = rates;
	});

	/**
	 * Did this browser's last window write a warning into the log?
	 *
	 * The browser's own rule, called rather than restated — including its guard that a
	 * window has to have enough frames in it to be judged, which a page that has just
	 * woken up does not. A second copy of the rule here drifted immediately: the banner
	 * claimed a line was in the log for a window the browser had declined to complain
	 * about, which is a panel lying about a log anybody can go and read.
	 */
	const troubled = (row: ClientStats) => struggling(row.frames) !== null;

	onMount(() => {
		const stop = data.stations.subscribeDeep((v) => { stations = v; });

		const applyLinks = (v: unknown) => {
			if (v && typeof v === 'object') links = v as Record<string, PeerLink>;
		};
		const applyClients = (v: unknown) => {
			if (v && typeof v === 'object') clients = v as Record<string, ClientStats>;
		};
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') thisStation = (v as { node_id: string | null }).node_id;
		};
		const stopLinks = client.subscribe('peers', applyLinks);
		const stopClients = client.subscribe('clients', applyClients);
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => {
			client.get(['peers']).then(applyLinks);
			client.get(['clients']).then(applyClients);
			client.get(['session']).then(applySession);
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);
		const ticking = setInterval(() => { now = Date.now(); }, 1000);

		return () => {
			stop();
			stopLinks();
			stopClients();
			stopSession();
			stopConnect();
			clearInterval(ticking);
		};
	});
</script>

<div class="system">
	<section class="block">
		<header class="block-head">
			<h2>
				Stations
				<Help label="About these figures">
					<p>
						<b>Console</b> is what the pult process itself is costing; <b>Machine</b> is
						everything on the box. Read them against each other, never summed: a console at 4%
						on a machine at 96% is about to be starved by something else.
					</p>
					<p>
						A process CPU figure is of <em>one core</em> — a busy console can pass 100% — and
						the machine's is of all its cores together, so each says which.
					</p>
				</Help>
			</h2>
			<span class="count">{stations.length} {stations.length === 1 ? 'station' : 'stations'}</span>
		</header>

		{#if stations.length === 0}
			<p class="empty">Nothing has reported yet.</p>
		{:else}
			<div class="cards">
				{#each stations as station (station.id)}
					{@const quiet = stale(Date.parse(station.last_seen))}
					<article class="card" class:stale={quiet} class:self={isSelf(station)}>
						<header class="card-head">
							<span class="name">{station.hostname}</span>
							{#if isSelf(station)}<span class="tag">this one</span>{/if}
							<span class="spacer"></span>
							{#if machineStrained(station)}
								<span class="badge-warn" title="The machine is short of processor, memory or disk — which the console will feel whether or not it is the cause">
									machine under strain
								</span>
							{/if}
							<span class="dim small" title="How long the console has been running, and the link to it from here">
								up {uptime(station.uptime_s)}{isSelf(station) ? '' : ` · ${latency(station)}`}
							</span>
							<span class="dim small">· {ago(Date.parse(station.last_seen))}</span>
						</header>

						<!-- What the console costs beside what the machine is carrying, row by
						     row: the comparison is the point, and two separate lists made it a
						     reading exercise. -->
						<table class="cmp">
							<thead><tr><th></th><th>Console</th><th>Machine</th></tr></thead>
							<tbody>
								<tr>
									<th>CPU</th>
									<td
										class="num"
										title="{station.machine.cores > 0
											? (station.cpu_percent / station.machine.cores).toFixed(1)
											: '?'}% of the whole machine"
									>
										{station.cpu_percent.toFixed(1)}% <span class="dim">of a core</span>
									</td>
									<td class="num" class:bad={station.machine.cpu_percent > 90}>
										{station.machine.cpu_percent.toFixed(1)}% <span class="dim">of {station.machine.cores} cores</span>
									</td>
								</tr>
								<tr>
									<th>Memory</th>
									<td class="num" title="{megabytes(station.mem_used)} of {megabytes(station.mem_total)}">
										{megabytes(station.mem_used)} <span class="dim">({memPercent(station).toFixed(0)}%)</span>
									</td>
									<td
										class="num"
										class:bad={percent(station.machine.mem_used, station.mem_total) > 90}
										title="{size(station.machine.mem_used)} of {size(station.mem_total)}"
									>
										{percent(station.machine.mem_used, station.mem_total).toFixed(0)}% <span class="dim">of {size(station.mem_total)}</span>
									</td>
								</tr>
							</tbody>
						</table>

						<!-- The rest of the box, small: these are looked up, not watched. -->
						<dl class="kv">
							<div>
								<dt title="One minute, and per core: 1.0 is as much work queued as there are cores to do it. Windows has no load average and reports nothing.">Load</dt>
								<dd class="num">
									{#if station.machine.load_1 === 0 && station.machine.load_5 === 0}
										<span class="dim">—</span>
									{:else}
										{station.machine.load_1.toFixed(2)}<span class="dim"> / {station.machine.load_5.toFixed(2)} / {station.machine.load_15.toFixed(2)}</span>
									{/if}
								</dd>
							</div>
							<div>
								<dt>Swap</dt>
								<dd class="num">
									{#if station.machine.swap_total === 0}
										<span class="dim">none</span>
									{:else}
										{size(station.machine.swap_used)} <span class="dim">of {size(station.machine.swap_total)}</span>
									{/if}
								</dd>
							</div>
							<div>
								<dt title="The volume the showfile is written to. A show that cannot be saved is what this is here to see coming.">Disk free</dt>
								<dd class="num">
									{#if station.machine.disk_total === 0}
										<span class="dim">unknown</span>
									{:else}
										<span class:bad={station.machine.disk_free / station.machine.disk_total < 0.05}>{size(station.machine.disk_free)}</span>
										<span class="dim">of {size(station.machine.disk_total)}</span>
									{/if}
								</dd>
							</div>
							<div>
								<dt title="The warmest sensor the machine exposes. Most virtual machines expose none.">Temperature</dt>
								<dd class="num">
									{#if station.machine.cpu_temperature_c === null}
										<span class="dim">no sensor</span>
									{:else}
										<span class:bad={station.machine.cpu_temperature_c > 85}>{station.machine.cpu_temperature_c.toFixed(0)}°C</span>
									{/if}
								</dd>
							</div>
							<div>
								<dt title="How long the machine has been up. A console younger than its machine has been restarted.">Booted</dt>
								<dd class="num">{uptime(station.machine.uptime_s)} ago</dd>
							</div>
						</dl>

						<h3 class="sub">
							Network
							<Help label="About the network figures">
								<p>
									The first three are what this console is responsible for; <b>The machine</b> is
									what its interfaces carried, including everything else the box is doing. A
									station whose own traffic is a fraction of its machine's has a network problem
									that is not the console's.
								</p>
								<p>
									Loopback is left out, or a demo talking to itself would count every byte twice.
									Node commands over MQTT are not in the output figure — small by construction,
									since a three-second fade is one message to a node that can run it.
								</p>
							</Help>
						</h3>
						<!-- Three of these are what the console is responsible for and the
						     fourth is what the cable is carrying. They must not be read as one
						     number: a machine whose interfaces are busy and whose console is
						     quiet has a network problem that is not the console's. -->
						<dl class="figures figures--wide">
							<div>
								<dt title="What this station's output connectors put on the wire">Output</dt>
								<dd class="num">{perSecond(outputWire(station))}</dd>
							</div>
							<div>
								<dt title="What the peer link from this console carried, both ways">To peers</dt>
								<dd class="num">{isSelf(station) ? '—' : perSecond(syncWire(station))}</dd>
							</div>
							<div>
								<dt title="What this station sent the browsers it is serving">To browsers</dt>
								<dd class="num">
									{isSelf(station) ? perSecond(browserWire) : '—'}
								</dd>
							</div>
							<div>
								<dt title="Every interface on the machine, this console's traffic and everything else's. Loopback excluded.">
									The machine
								</dt>
								<dd class="num">
									{#if station.net_window_ms === 0}
										<span class="dim">not read yet</span>
									{:else}
										{perSecond(machineWire(station))}
									{/if}
								</dd>
							</div>
						</dl>

						<h3 class="sub">Output frames</h3>
						{#if station.frame_costs.length === 0}
							<!-- Absent rather than zero: a connector that emitted nothing in the
							     window carries no entry at all, because zero would read as
							     "instant" when the truth is that nothing happened. -->
							<p class="quiet-note">None in the last window: no output, or everything settled.</p>
						{:else}
							<table class="frames">
								<thead>
									<tr>
										<th>Output</th><th>Rate</th><th>Mean</th><th title="The mean frame, over the windows this tile has seen">Trend</th>
										<th>Worst</th><th>Evaluating</th><th>Sending</th>
										<th title="What actually left, after the dedup skipped every universe that had not changed">On the wire</th>
									</tr>
								</thead>
								<tbody>
									{#each station.frame_costs as cost (cost.output)}
										<tr>
											<td>
												<span class="name">{cost.output}</span>
												<span class="dim small">{cost.kind}</span>
											</td>
											<td class="num">{rateOf(cost).toFixed(0)} Hz</td>
											<td class="num">{cost.mean_ms.toFixed(2)} ms</td>
											<td class="spark">
												{#if framePaths[`${station.id}/${cost.output}`]}
													<svg width={SPARK_W} height={SPARK_H} aria-hidden="true">
														<path d={framePaths[`${station.id}/${cost.output}`]} />
													</svg>
												{/if}
											</td>
											<td class="num">{cost.max_ms.toFixed(2)} ms</td>
											<td class="num" title="Working out what the patch is doing; worst {cost.evaluating_max_ms.toFixed(2)} ms">
												{cost.evaluating_mean_ms.toFixed(2)} ms
											</td>
											<td class="num dim">{sendingOf(cost).toFixed(2)} ms</td>
											<!-- What left, not what a universe count suggests: the DMX
											     family skips a universe whose image has not changed and
											     is not yet due a refresh, so a settled rig sends a
											     fraction of what a moving one does. -->
											<td class="num" title="{cost.packets} packets in the window">
												{perSecond(wireOf(cost))}
											</td>
										</tr>
									{/each}
								</tbody>
							</table>
						{/if}
					</article>
				{/each}
			</div>
		{/if}


	</section>

	<section class="block">
		<header class="block-head">
			<h2>
				Node audio
				<Help label="About the node audio figures">
					<p>
						What the station driving the nodes knows about each loudspeaker's stream, since that
						stream started. A dropout is one of three things and reads differently here:
					</p>
					<p>
						<b>Missing</b> is sound the node had to play without its packet, in milliseconds; a
						5 ms packet is 5 ms. <b>Never arrived</b> counts packets that did not reach the
						node's receiver at all, and <b>longest wait</b> is the longest it went without one:
						over 30 ms, its six-packet socket queue overflows and the node itself drops the next
						ones. Never arrived with short waits is the network. <b>Late</b> packets came after
						their time. <b>Behind</b> and <b>skips</b> are the sender on the station not keeping
						up; <b>lowest lead</b> is the least time a packet had between leaving and being heard,
						and near the delay is healthy. <b>Underruns</b> on their own are the node running dry
						with everything arriving.
					</p>
					<p>
						Every one of these is also a warning in the System Log, with the buffer depth and the
						path at that moment.
					</p>
				</Help>
			</h2>
			<span class="count">
				{#if $drivingReport}{speakerNodes.length} with a loudspeaker{:else}no station is driving the nodes{/if}
			</span>
		</header>

		{#if speakerNodes.length === 0}
			<p class="empty">No adopted node has a loudspeaker.</p>
		{:else}
			<table class="rack">
				<thead>
					<tr>
						<th>Node</th><th>Path</th><th>Streaming</th>
						<th title="Sound the node had to play with nothing in its buffer, since the stream started: each 5 ms is one packet">Missing</th>
						<th title="Packets that never reached the node's receiver, by sequence number (newer firmware)">Never arrived</th>
						<th title="The longest the node's receiver went without a packet. Over 30 ms its six-packet socket queue overflows.">Longest wait</th>
						<th title="Packets that arrived after they should have been heard">Late</th>
						<th title="The node's buffer ran dry">Underruns</th>
						<th title="The node gave up on the stream's timing and started over">Resyncs</th>
						<th title="The sender fell behind by more than half the delay / skipped ahead">Behind</th>
						<th title="The least time a packet had between leaving and being heard, of the delay">Lowest lead</th>
						<th>Last trouble</th>
					</tr>
				</thead>
				<tbody>
					{#each speakerNodes as node (node.serial)}
						{@const health = node.stream}
						<tr class:stale={!node.online}>
							<td>
								{node.name}<span class="addr mono">{node.serial}</span>
								{#if !node.online}<span class="tag">offline</span>{:else if !node.on_broker}<span class="tag">not on the broker</span>{/if}
							</td>
							<td>
								{#if health && health.paths.length}
									{#each health.paths as path (path.ip)}
										<span class:bad={path.transport === 'wifi'}>{path.transport}</span><span class="addr">{path.ip}</span>
									{/each}
								{:else}
									<span class="dim">—</span>
								{/if}
							</td>
							{#if health}
								<td>
									{since(health.since_ms)}
									{#if health.sessions > 1}<span class="tag" title="Each restart is a gap the length of the node's buffer">{health.sessions} sessions</span>{/if}
								</td>
								<td class="num" class:bad={health.missing > 0}>
									{health.missing > 0 ? `${Math.ceil((health.missing * 1000) / (health.node_rate || 48000))} ms` : '0'}
								</td>
								<td class="num" class:bad={health.seq_lost > 0}>{health.seq_lost}</td>
								<td class="num" class:bad={health.rx_gap_max_ms > 30}>{health.rx_gap_max_ms ? `${health.rx_gap_max_ms} ms` : '—'}</td>
								<td class="num" class:bad={health.late > 0}>{health.late}</td>
								<td class="num" class:bad={health.underruns > 0}>{health.underruns}</td>
								<td class="num" class:bad={health.resyncs > 0}>{health.resyncs}</td>
								<td class="num" class:bad={health.stalls + health.skips + health.send_errors > 0}>
									{health.stalls}{#if health.skips}<span class="dim">{` · ${health.skips} skips`}</span>{/if}{#if health.send_errors}<span class="dim">{` · ${health.send_errors} not sent`}</span>{/if}
								</td>
								<td class="num" class:bad={health.lowest_lead_ms !== null && health.lowest_lead_ms < health.delay_ms / 2}>
									{health.lowest_lead_ms === null ? '—' : `${health.lowest_lead_ms} ms`}<span class="dim">{` of ${health.delay_ms} ms`}</span>
								</td>
								<td>
									{#if health.last_incident && health.last_incident_ms}
										<span class="bad">{health.last_incident}</span><span class="addr">{since(health.last_incident_ms)}</span>
									{:else}
										<span class="dim">none</span>
									{/if}
								</td>
							{:else}
								<td class="dim" colspan="10">
									no stream{#if node.summary && node.summary.underruns > 0} · {node.summary.underruns} underruns since it started{/if}
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	</section>

	<section class="block">
		<header class="block-head">
			<h2>
				Browsers
				<Help label="About the browser figures">
					<p>
						These are the browsers connected to <em>this</em> station. What reaches the others is a
						struggling browser's warning in the System Log rather than a figure every second.
					</p>
					<p>
						<b>Received</b> is measured by the station rather than claimed by the page: no browser
						can see how many bytes arrived on its own socket.
					</p>
					<p>
						A trend is what this tile has watched since it was opened — nothing on the network
						carries a history.
					</p>
				</Help>
			</h2>
			<span class="count">{browsers.length} on this station</span>
		</header>

		{#if browsers.length === 0}
			<p class="empty">No browser has described itself yet.</p>
		{:else}
			<table class="rack">
				<thead>
					<tr>
						<th>Browser</th><th>Frame rate</th>
						<th title="Frame rate against 60 fps, over the windows this tile has seen">Trend</th>
						<th>Worst frame</th><th>Evaluating</th>
						<th>Parameters</th><th>Memory</th>
						<th title="What this station is sending down that socket. Measured by the station: a page cannot see its own socket.">Received</th>
						<th>Clock</th><th>Heard</th>
					</tr>
				</thead>
				<tbody>
					{#each browsers as row (row.session)}
						<tr class:stale={stale(row.at_ms)} class:self={row.session === $thisBrowser}>
							<td>
								<span class="mono">{row.session}</span>
								{#if row.session === $thisBrowser}<span class="tag">this one</span>{/if}
								<span class="addr">{row.label}</span>
							</td>
							{#if row.frames}
								{@const frames = row.frames}
								<td class="num" class:bad={fpsOf(frames) < SLOW_FPS}>
									{fpsOf(frames).toFixed(0)} fps
								</td>
								<td class="spark">
									{#if browserPaths[row.session]}
										<svg width={SPARK_W} height={SPARK_H} aria-hidden="true">
											<!-- Drawn against 60 fps rather than against its own best, so
											     two browsers' lines mean the same thing. -->
											<path d={browserPaths[row.session]} class:bad={fpsOf(frames) < SLOW_FPS} />
										</svg>
									{/if}
								</td>
								<td class="num" class:bad={frames.max_ms > STALL_MS}>
									{frames.max_ms.toFixed(0)} ms
								</td>
								<td class="num" title="Worst {frames.evaluating_max_ms.toFixed(2)} ms">
									{frames.evaluating_mean_ms.toFixed(2)} ms
								</td>
								<td class="num">{frames.parameters}</td>
							{:else}
								<!-- A page with no light on it, or a tab the browser has stopped
								     serving frames to. Neither is a fault, and a frame rate of
								     zero would read as one. -->
								<td class="num dim" colspan="5">drawing nothing</td>
							{/if}
							<td class="num">
								{#if row.heap_used === null}
									<span class="dim">not offered</span>
								{:else}
									{megabytes(row.heap_used)}
								{/if}
							</td>
							<td class="num">
								{#if row.sent_window_ms === 0}
									<!-- A page's first report has no previous one to measure against,
									     so there is a byte count and not yet a rate. -->
									<span class="dim">first report</span>
								{:else}
									{perSecond(bytesPerSecondOf(row.sent_window_ms, row.sent_bytes))}
								{/if}
							</td>
							<td class="num">
								{#if row.clock_offset_ms === null}
									<!-- The figure that says whether anything else this page shows can
									     be trusted: without an offset it is drawing gaps, not values. -->
									<span class="bad">not placed</span>
								{:else}
									{row.clock_offset_ms >= 0 ? '+' : ''}{row.clock_offset_ms.toFixed(0)} ms
									<span class="dim">±{((row.clock_rtt_ms ?? 0) / 2).toFixed(0)}</span>
								{/if}
							</td>
							<td class="num dim">{ago(row.at_ms)}</td>
						</tr>
					{/each}
				</tbody>
			</table>

			{#if browsers.some(troubled)}
				<p class="warn">
					A browser below is under {SLOW_FPS} fps or has stalled for more than {STALL_MS} ms
					in a frame. It has said so in the log as well, which every console can read.
				</p>
			{/if}
		{/if}
	</section>
</div>

<style>
	.system { padding: 16px 20px; }
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: baseline; justify-content: space-between; margin-bottom: 8px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; display: inline-flex; align-items: center; gap: 6px; }
	/* Console beside machine: the comparison the panel is for. */
	.cmp { width: 100%; border-collapse: collapse; font-size: 13px; margin: 4px 0 10px; }
	.cmp thead th { text-align: left; font-size: 11px; font-weight: 500; text-transform: uppercase; letter-spacing: 0.05em; color: #777; padding: 0 0 4px; }
	.cmp tbody th { text-align: left; font-weight: 400; color: #888; padding: 3px 12px 3px 0; width: 5.5rem; }
	.cmp td { padding: 3px 12px 3px 0; white-space: nowrap; }
	.kv { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 6px 16px; margin: 0 0 12px; font-size: 12px; }
	.kv div { display: flex; flex-direction: column; gap: 1px; }
	.kv dt { color: #777; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; }
	.kv dd { margin: 0; white-space: nowrap; }
	.count { color: #777; font-size: 12px; }

	.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 12px; }
	.card { border: 1px solid #2a2a2a; border-radius: 6px; padding: 10px 12px; background: #1d1d1d; }
	.card.self { background: #1c2128; border-color: #2f3a44; }
	.card.stale { opacity: 0.45; }
	.card-head { display: flex; align-items: center; gap: 6px; margin-bottom: 8px; }
	.spacer { flex: 1; }

	.figures { display: grid; grid-template-columns: repeat(2, 1fr); gap: 4px 16px; margin-bottom: 10px; }
	.figures--wide { margin-bottom: 12px; }
	.figures div { display: flex; justify-content: space-between; align-items: baseline; gap: 8px; }
	dt { color: #777; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; }
	dd { font-size: 13px; }

	.badge-warn { background: #3a2a1a; border: 1px solid #5a3a1a; border-radius: 9px; color: #e0a055; font-size: 10px; padding: 1px 6px; }
	.sub { font-size: 11px; font-weight: 500; text-transform: uppercase; letter-spacing: 0.05em; color: #777; margin-bottom: 4px; margin-top: 4px; display: flex; gap: 6px; align-items: center; }
	.frames { width: 100%; border-collapse: collapse; font-size: 12px; }
	.frames th { text-align: left; color: #666; font-weight: 500; font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; padding: 0 6px 3px 0; }
	.frames td { padding: 3px 6px 3px 0; border-top: 1px solid #262626; }

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
	.small { font-size: 11px; }
	.mono { font-family: monospace; }
	.bad { color: #e0774f; }
	.spark { width: 72px; padding-right: 8px; }
	.spark svg { display: block; overflow: visible; }
	.spark path { fill: none; stroke: #5a8fbd; stroke-width: 1.25; stroke-linejoin: round; }
	.spark path.bad { stroke: #e0774f; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.quiet-note { color: #666; font-size: 12px; font-style: italic; }
	.warn { color: #e08a55; font-size: 12px; margin-top: 10px; }
</style>
