// What this station's tick costs, read off the station itself.
//
//   node scripts/demo-measure.mjs <port> [--label <name>]
//
// No profiler and nothing privileged: the figures come from the `stations` row the
// console publishes about itself every couple of seconds, over the same WebSocket
// API a browser uses. So the number printed here is the number the Stations panel
// shows and the number a peer sees — if it is wrong, it is wrong everywhere, which
// is the property worth having.
//
// Run by `scripts/demo.sh --measure`, which seeds a preset first.

import { connect, sleep } from './demo-ws.mjs';

const argv = process.argv.slice(2);
let port = '7700';
let label = '';
let build = '';
for (let i = 0; i < argv.length; i++) {
	if (argv[i] === '--label') label = argv[++i];
	else if (argv[i] === '--build') build = argv[++i];
	else if (!argv[i].startsWith('--')) port = argv[i];
}

// `Playback::tick`'s interval, and so the budget a tick has to come in under.
const TICK_BUDGET_MS = 25;
// `REPORT_INTERVAL` in infra/stations.rs. A window is one of these.
const REPORT_INTERVAL_MS = 2000;

const station = connect(port, { timeoutMs: 30_000 });
const { get, call } = station;

await station.open.catch((error) => {
	console.error(`  ${error.message}`);
	process.exit(1);
});

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

	// Two windows: the first covers the writes above as well as the ticks, the
	// second is the show running on its own, which is what is being asked about.
	await sleep(REPORT_INTERVAL_MS * 2.5);

	const rows = await get(['stations']);
	if (!rows?.length) {
		console.error('  no station has published a row yet');
		process.exit(1);
	}

	const ms = (n) => `${n.toFixed(n < 10 ? 2 : 1)} ms`;
	const pad = (s, n) => String(s).padEnd(n);
	const rpad = (s, n) => String(s).padStart(n);

	console.log('');
	if (build) {
		// A tick figure without the build it came from is worse than none: a debug
		// build is the best part of an order of magnitude slower, and these numbers
		// are going to end up compared against ones taken in release.
		console.log(`  ${build} build`);
	}
	console.log(
		`  ${pad('show', 12)}${rpad('fixtures', 9)}${rpad('cues', 6)}` +
			`${rpad('tick', 10)}${rpad('worst', 10)}${rpad('playback', 10)}${rpad('CPU', 8)}`
	);
	console.log(`  ${'─'.repeat(65)}`);

	// One station reads as the show it is running; several have to say which is which.
	const named = rows.length > 1 || !label;

	for (const row of rows) {
		const cost = row.tick_cost;
		const host = row.hostname.split('.')[0];
		console.log(
			`  ${pad(label || host, 12)}${rpad(fixtures.length, 9)}${rpad(cues.length, 6)}` +
				`${rpad(cost ? ms(cost.mean_ms) : '—', 10)}` +
				`${rpad(cost ? ms(cost.max_ms) : '—', 10)}` +
				`${rpad(cost ? ms(cost.playback_mean_ms) : '—', 10)}` +
				`${rpad(`${row.cpu_percent.toFixed(0)}%`, 8)}`
		);

		if (named) console.log(`    on ${host}`);

		if (!cost) {
			// Absent is not zero. A station that did no work in the window has nothing
			// to say about what a tick costs, and says so.
			console.log('    nothing ran in that window — this station is settled, not instant');
			continue;
		}

		const share = ((cost.mean_ms / TICK_BUDGET_MS) * 100).toFixed(0);
		const applying = cost.mean_ms - cost.playback_mean_ms;
		console.log(
			`    ${share}% of the ${TICK_BUDGET_MS} ms budget over ${cost.ticks} ticks; ` +
				`computing ${ms(cost.playback_mean_ms)}, applying ${ms(applying)}`
		);
		if (cost.max_ms > TICK_BUDGET_MS) {
			console.log(
				`    over budget: worst tick ${ms(cost.max_ms)}. A late tick loses smoothness, ` +
					'not correctness — a value comes from the wall clock, not from the last one.'
			);
		}
	}

	// What this figure is not, said where it is read, because it is the way it will
	// be misread: flows and the output-config push are outside it, so it is what
	// playback costs and not what the process costs. That is the CPU column.
	console.log('');
	console.log('  tick = playback only (reading the show, computing it, applying it).');
	console.log('  Flows and output config are outside it; the whole process is the CPU column.');
	console.log('');

	process.exit(0);
} catch (error) {
	console.error(`  measuring failed: ${error.message}`);
	process.exit(1);
}
