// What this station's output frames cost, read off the station itself.
//
//   node scripts/demo-measure.mjs <port> [--label <name>] [--windows <n>]
//
// No profiler and nothing privileged: the figures come from the `stations` row the
// console publishes about itself every couple of seconds, over the same WebSocket
// API a browser uses. So the number printed here is the number the Stations panel
// shows and the number a peer sees — if it is wrong, it is wrong everywhere, which
// is the property worth having.
//
// Run by `scripts/demo.sh --measure`, which seeds a preset first.
//
// The thing with a deadline is the output *frame*. There is no engine tick behind it
// any more: a pass through playback happens when the show changes, and a fade in
// progress is not a change to the show. So what is printed is per connector, because
// their rates and their costs are their own.
//
// ── Why this takes several windows ────────────────────────────────────────────
//
// It used to take one. It slept a second, slept four more, read `stations` once and
// printed whatever window happened to be sitting in the row. That is a single sample
// of a single two-second window, caught at an arbitrary phase against the station's
// own reporting tick — and depending on where the phase landed, the window it printed
// could still be half full of the cue-taking this script does to get the show moving.
// Two runs at 505 fixtures came out fifty per cent apart, and that is the reason.
//
// So: take several windows, throw the first away, and report the median with the
// spread beside it. An instrument that cannot say how much it disagrees with itself
// cannot be used to tell an optimisation from noise, which is the whole job here.

import { connect, sleep } from './demo-ws.mjs';

const argv = process.argv.slice(2);
let port = '7700';
let label = '';
let build = '';
let windows = 6;
for (let i = 0; i < argv.length; i++) {
	if (argv[i] === '--label') label = argv[++i];
	else if (argv[i] === '--build') build = argv[++i];
	else if (argv[i] === '--windows') windows = Math.max(3, Number(argv[++i]) || 6);
	else if (!argv[i].startsWith('--')) port = argv[i];
}

// `Frames::DMX`'s moving rate, and so the budget a frame has to come in under.
const FRAME_BUDGET_MS = 25;
// `REPORT_INTERVAL` in infra/stations.rs. A window is one of these.
const REPORT_INTERVAL_MS = 2000;

// How far the windows may disagree before the median stops meaning anything. Read as
// a fraction of the median: a spread wider than this is printed with a warning rather
// than quietly, because a reader comparing it against another run would otherwise be
// comparing two clouds and calling the difference a result.
const SPREAD_REFUSED = 0.25;

const station = connect(port, { timeoutMs: 30_000 });
const { get, call } = station;

await station.open.catch((error) => {
	console.error(`  ${error.message}`);
	process.exit(1);
});

/** The middle value, which is what a handful of noisy windows should be read by. */
function median(values) {
	const sorted = [...values].sort((a, b) => a - b);
	const middle = Math.floor(sorted.length / 2);
	return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

/**
 * How far the windows disagreed, as a fraction of the middle one.
 *
 * Full range rather than a standard deviation, deliberately: with six samples the
 * question being asked is "could this number have come out much different", and the
 * worst pair answers that more honestly than a statistic that assumes a shape.
 */
function spread(values) {
	const mid = median(values);
	if (!mid) return 0;
	return (Math.max(...values) - Math.min(...values)) / mid;
}

try {
	const sequences = await get(['sequences']);
	const cues = await get(['cues']);
	const fixtures = await get(['fixtures']);
	const byId = new Map(cues.map((cue) => [cue.id, cue]));

	/**
	 * Put the show somewhere worth measuring.
	 *
	 * A station with nothing moving settles and stops ticking — correctly, and it is
	 * what makes an absent figure mean something — but a settled station is not what
	 * anybody ran this to measure. So each sequence is advanced until its live cue
	 * carries an effect, which is the state that keeps a console ticking for as long
	 * as it is up. Bounded by the number of cues, so a stack with no effect in it
	 * ends up somewhere rather than looping forever.
	 */
	const hasEffect = (cue) => cue?.captures?.some((capture) => capture.effect);
	const live = async (sequenceId) =>
		(await get(['sequences'])).find((s) => s.id === sequenceId);

	for (const sequence of sequences) {
		for (let step = 0; step < sequence.cue_ids.length; step++) {
			const now = await live(sequence.id);
			const active =
				now?.active_cue_index != null ? byId.get(now.cue_ids[now.active_cue_index]) : null;
			if (hasEffect(active)) break;
			await call('sequences.goNext', { sequenceId: sequence.id });
		}
	}

	// What a browser is sent while the show runs.
	//
	// The other half of the claim, and the one an operator feels: a fade used to be a
	// write and a broadcast per fixture per tick, so a connected console spent a cue on
	// the receiving end of a few thousand messages a second. Nothing stores a value any
	// more, so the only thing a frontend hears about is the show changing — and this
	// counts what actually arrives rather than asserting that.
	let pushed = 0;
	station.subscribe('fixtures/**', () => {
		pushed += 1;
	});

	// ── Letting the cue-taking drain ────────────────────────────────────────────
	//
	// Taking a cue writes `live_fades` per captured fixture, and those broadcasts are
	// still arriving well after the last `goNext` has returned — three hundred of them
	// on a 505-fixture rig. Start measuring the instant the call returns and every one
	// of them lands inside the first windows: the CPU reads high, the frame times read
	// wide, and the browser-update count reads as though a running show were pushing
	// values, which is the one claim this script exists to check.
	//
	// So wait for it to go quiet rather than sleeping a guessed constant. Quiet is the
	// real end of the burst on any rig size, where a constant is only ever right for
	// the one it was measured on.
	const seen = new Set();
	const QUIET_MS = 1500;
	const SETTLE_LIMIT_MS = 20_000;
	let lastSeen = Date.now();
	let before = pushed;
	const settleBy = Date.now() + SETTLE_LIMIT_MS;
	while (Date.now() < settleBy) {
		await sleep(250);
		if (pushed !== before) {
			before = pushed;
			lastSeen = Date.now();
		} else if (Date.now() - lastSeen >= QUIET_MS) {
			break;
		}
	}

	// ── Collecting windows ──────────────────────────────────────────────────────
	//
	// Poll faster than the station reports and keep a window the first time it is
	// seen, identified by the row's `last_seen` the way `trace.ts` dedupes a
	// sparkline. Polling rather than sleeping exactly one interval keeps this from
	// drifting into lockstep with the reporter and sampling the same phase forever.
	const collected = new Map(); // station hostname → array of windows
	let rows = [];

	const wanted = windows + 1; // one to throw away
	const deadline = Date.now() + wanted * REPORT_INTERVAL_MS * 2 + 10_000;
	const countedFrom = Date.now();
	// Zeroed here, after the burst has gone quiet, so what this counts is a *running*
	// show pushing nothing rather than the tail of the cues this script itself took.
	pushed = 0;
	// And the windows the station is holding right now still cover that burst, so the
	// first one collected is discarded on top of this wait, not instead of it.
	seen.clear();

	while (seen.size < wanted && Date.now() < deadline) {
		await sleep(250);
		rows = await get(['stations']);
		if (!rows?.length) continue;
		for (const row of rows) {
			const stamp = `${row.hostname}@${row.last_seen}`;
			if (seen.has(stamp)) continue;
			seen.add(stamp);
			const host = row.hostname.split('.')[0];
			if (!collected.has(host)) collected.set(host, []);
			collected.get(host).push({ cpu_percent: row.cpu_percent, frame_costs: row.frame_costs ?? [] });
		}
	}
	const countedFor = (Date.now() - countedFrom) / 1000;

	if (seen.size < 3) {
		console.error(
			`  only ${seen.size} report${seen.size === 1 ? '' : 's'} arrived in ` +
				`${countedFor.toFixed(1)} s — the station may not be publishing its row`
		);
		process.exit(1);
	}

	// The first window is thrown away in every case. It overlaps the cue-taking above
	// and the socket setup, and it is exactly the window that made the old single
	// sample untrustworthy.
	for (const [host, list] of collected) collected.set(host, list.slice(1));

	const ms = (n) => `${n.toFixed(n < 10 ? 2 : 1)} ms`;
	const pad = (s, n) => String(s).padEnd(n);
	const rpad = (s, n) => String(s).padStart(n);

	console.log('');
	if (build) {
		// A frame figure without the build it came from is worse than none: a debug
		// build is the best part of an order of magnitude slower, and these numbers
		// are going to end up compared against ones taken in release.
		console.log(`  ${build} build`);
	}
	console.log(
		`  median of ${Math.min(...[...collected.values()].map((l) => l.length))} windows, ` +
			`first discarded`
	);
	console.log('');
	console.log(
		`  ${pad('station', 12)}${rpad('fixtures', 9)}${rpad('cues', 6)}${rpad('CPU', 8)}${rpad('±', 8)}`
	);
	console.log(`  ${'─'.repeat(43)}`);
	for (const [host, list] of collected) {
		const cpus = list.map((w) => w.cpu_percent);
		console.log(
			`  ${pad(label || host, 12)}${rpad(fixtures.length, 9)}${rpad(cues.length, 6)}` +
				`${rpad(`${median(cpus).toFixed(0)}%`, 8)}${rpad(`${(spread(cpus) * 100).toFixed(0)}%`, 8)}`
		);
	}

	console.log('');
	console.log(
		`  ${pad('connector', 24)}${pad('protocol', 9)}` +
			`${rpad('frame', 9)}${rpad('±', 6)}${rpad('worst', 9)}` +
			`${rpad('evaluating', 11)}${rpad('assembling', 11)}${rpad('socket', 9)}${rpad('Hz', 5)}`
	);
	console.log(`  ${'─'.repeat(94)}`);

	let anyMeasured = false;
	let widest = 0;
	for (const [host, list] of collected) {
		// One station reads as the show it is running; several have to say which.
		const where = collected.size > 1 ? ` (${host})` : '';

		// Every connector that showed up in any window. A connector absent from one
		// window sent nothing in it, which is a real fact about that window rather
		// than a zero to average in.
		const names = new Map();
		for (const window of list) {
			for (const cost of window.frame_costs) {
				if (!names.has(cost.output)) names.set(cost.output, []);
				names.get(cost.output).push(cost);
			}
		}

		if (names.size === 0) {
			// Absent is not zero. A station whose connectors emitted nothing in the
			// window has nothing to say about what a frame costs, and says so.
			console.log(`  ${pad(`nothing sending${where}`, 26)}`);
			console.log('    no frames in those windows — this station is settled, not instant');
			continue;
		}

		for (const [name, costs] of names) {
			anyMeasured = true;
			const frame = costs.map((c) => c.mean_ms);
			const evaluating = costs.map((c) => c.evaluating_mean_ms);
			// Three parts now, not two. Assembling and the socket are both per
			// universe and a rig that grows past a couple of dozen of them spends
			// its frame in one of the two — but they do not answer to the same fix,
			// so a single "emitting" figure could not say which.
			const assembling = costs.map((c) => c.assembling_mean_ms ?? 0);
			const socket = costs.map(
				(c) => c.mean_ms - c.evaluating_mean_ms - (c.assembling_mean_ms ?? 0)
			);
			const rates = costs.map((c) => (c.window_ms ? (c.frames * 1000) / c.window_ms : 0));
			const variation = spread(frame);
			widest = Math.max(widest, variation);

			console.log(
				`  ${pad(name + where, 24)}${pad(costs[0].kind, 9)}` +
					`${rpad(ms(median(frame)), 9)}${rpad(`${(variation * 100).toFixed(0)}%`, 6)}` +
					`${rpad(ms(Math.max(...costs.map((c) => c.max_ms))), 9)}` +
					`${rpad(ms(median(evaluating)), 11)}${rpad(ms(median(assembling)), 11)}` +
					`${rpad(ms(median(socket)), 9)}${rpad(median(rates).toFixed(0), 5)}`
			);
			const share = ((median(frame) / FRAME_BUDGET_MS) * 100).toFixed(0);
			const whole = median(frame) || 1;
			console.log(
				`    ${share}% of the ${FRAME_BUDGET_MS} ms budget over ${costs.length} windows; ` +
					`${((median(evaluating) / whole) * 100).toFixed(0)}% evaluating, ` +
					`${((median(assembling) / whole) * 100).toFixed(0)}% assembling, ` +
					`${((median(socket) / whole) * 100).toFixed(0)}% socket`
			);
			// The worst frame is a maximum across every window rather than a median of
			// maxima: what is being asked is whether this connector ever missed, and
			// one miss in six windows is still a miss.
			if (Math.max(...costs.map((c) => c.max_ms)) > FRAME_BUDGET_MS) {
				console.log(
					`    over budget: worst frame ${ms(Math.max(...costs.map((c) => c.max_ms)))}. ` +
						'A late frame loses smoothness, not correctness — a value comes from the ' +
						'clock, not from the last frame.'
				);
			}
		}
	}

	// What these figures are not, said where they are read, because it is the way they
	// will be misread.
	console.log('');
	if (widest > SPREAD_REFUSED) {
		console.log(
			`  ⚠ these windows disagreed by ${(widest * 100).toFixed(0)}%, which is past the ` +
				`${SPREAD_REFUSED * 100}% this instrument trusts.`
		);
		console.log('  Something else on this machine is competing for the CPU, or the run was');
		console.log('  too short. Do not compare this against another run: take it again.');
		console.log('');
	}
	if (!anyMeasured) {
		console.log('  No output is configured, so there is no frame to measure.');
		console.log('  Seeding an Art-Net or sACN output is what gives this something to say.');
	}
	console.log(
		`  A connected browser was sent ${pushed} update${pushed === 1 ? '' : 's'} about the rig ` +
			`in ${countedFor.toFixed(1)} s`
	);
	console.log('  — the show changing, never a value moving. Motion is drawn in the page.');
	console.log('');
	console.log('  frame = one connector gathering, evaluating and emitting one frame.');
	console.log('  Evaluating is linear in the rig; assembling and the socket are per');
	console.log('  universe, which is why they are apart — a rig of 5000 six-channel heads');
	console.log('  is about 59 universes against 24, and only those two grow with that.');
	console.log('  ± is the full range across windows as a fraction of the median: how much');
	console.log('  this instrument disagrees with itself, and the floor on any difference');
	console.log('  between two runs that can honestly be called a result.');
	console.log('  The engine has no tick behind it: a pass happens when the show changes,');
	console.log('  and a fade in progress is not a change to the show. The whole process is');
	console.log('  the CPU column.');
	console.log('');

	process.exit(widest > SPREAD_REFUSED ? 3 : 0);
} catch (error) {
	console.error(`  measuring failed: ${error.message}`);
	process.exit(1);
}
