// What this station's output frames cost, read off the station itself.
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
//
// The thing with a deadline is the output *frame*. There is no engine tick behind it
// any more: a pass through playback happens when the show changes, and a fade in
// progress is not a change to the show. So what is printed is per connector, because
// their rates and their costs are their own.

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

// `Frames::DMX`'s moving rate, and so the budget a frame has to come in under.
const FRAME_BUDGET_MS = 25;
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
	await sleep(REPORT_INTERVAL_MS * 0.5);
	const countedFrom = Date.now();
	pushed = 0;

	// Two windows: the first covers the writes above as well as the frames, the
	// second is the show running on its own, which is what is being asked about.
	await sleep(REPORT_INTERVAL_MS * 2);
	const countedFor = (Date.now() - countedFrom) / 1000;

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
		`  ${pad('station', 12)}${rpad('fixtures', 9)}${rpad('cues', 6)}${rpad('CPU', 8)}`
	);
	console.log(`  ${'─'.repeat(35)}`);
	for (const row of rows) {
		const host = row.hostname.split('.')[0];
		console.log(
			`  ${pad(label || host, 12)}${rpad(fixtures.length, 9)}${rpad(cues.length, 6)}` +
				`${rpad(`${row.cpu_percent.toFixed(0)}%`, 8)}`
		);
	}

	console.log('');
	console.log(
		`  ${pad('connector', 26)}${pad('protocol', 10)}` +
			`${rpad('frame', 10)}${rpad('worst', 10)}${rpad('evaluating', 12)}${rpad('frames', 8)}`
	);
	console.log(`  ${'─'.repeat(76)}`);

	let anyMeasured = false;
	for (const row of rows) {
		const host = row.hostname.split('.')[0];
		// One station reads as the show it is running; several have to say which.
		const where = rows.length > 1 ? ` (${host})` : '';

		if (!row.frame_costs?.length) {
			// Absent is not zero. A station whose connectors emitted nothing in the
			// window has nothing to say about what a frame costs, and says so.
			console.log(`  ${pad(`nothing sending${where}`, 26)}`);
			console.log('    no frames in that window — this station is settled, not instant');
			continue;
		}

		for (const cost of row.frame_costs) {
			anyMeasured = true;
			console.log(
				`  ${pad(cost.output + where, 26)}${pad(cost.kind, 10)}` +
					`${rpad(ms(cost.mean_ms), 10)}${rpad(ms(cost.max_ms), 10)}` +
					`${rpad(ms(cost.evaluating_mean_ms), 12)}${rpad(cost.frames, 8)}`
			);
			const share = ((cost.mean_ms / FRAME_BUDGET_MS) * 100).toFixed(0);
			const emitting = cost.mean_ms - cost.evaluating_mean_ms;
			const rate = cost.window_ms ? ((cost.frames * 1000) / cost.window_ms).toFixed(0) : '?';
			console.log(
				`    ${share}% of the ${FRAME_BUDGET_MS} ms budget at ${rate} Hz ` +
					`over ${(cost.window_ms / 1000).toFixed(1)} s; ` +
					`evaluating ${ms(cost.evaluating_mean_ms)}, emitting ${ms(emitting)}`
			);
			if (cost.max_ms > FRAME_BUDGET_MS) {
				console.log(
					`    over budget: worst frame ${ms(cost.max_ms)}. A late frame loses smoothness, ` +
						'not correctness — a value comes from the clock, not from the last frame.'
				);
			}
		}
	}

	// What these figures are not, said where they are read, because it is the way they
	// will be misread.
	console.log('');
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
	console.log('  The engine has no tick behind it: a pass happens when the show changes,');
	console.log('  and a fade in progress is not a change to the show. The whole process is');
	console.log('  the CPU column.');
	console.log('');

	process.exit(0);
} catch (error) {
	console.error(`  measuring failed: ${error.message}`);
	process.exit(1);
}
