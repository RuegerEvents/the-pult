// What a *browser* costs on this show, read off the station the browser reports to.
//
//   node scripts/demo-measure-browser.mjs <port> [--label <name>] [--windows <n>]
//
// Run by `scripts/demo.sh --measure-browser`, which seeds a preset first.
//
// ── Why this is not part of --measure ─────────────────────────────────────────
//
// `--measure` starts no sims and no dev server, because both would be taking the CPU
// it is holding still to measure. A browser drawing five thousand fixtures is a far
// larger competitor for that CPU than either. So the station figures printed by
// `--measure` and the page figures printed here are *not* two halves of one reading,
// and this script says so in as many words rather than leaving somebody to put them
// side by side and subtract.
//
// ── Where the figures come from ───────────────────────────────────────────────
//
// Nowhere new. Task 49 already had the page reporting its own frame time, evaluator
// time, parameter count and clock offset to the station over `client.report`, and
// the station keeping those in the LOCAL `clients` map. This opens a page, waits for
// it to start reporting, and reads that map over the same WebSocket API the System
// panel uses. There is no second instrument and no profiler.
//
// The page is pointed at a workspace holding only the rig, by writing the layout into
// `localStorage` before the app boots. That is the panel whose cost is in doubt, and
// a page showing a cue list would report a frame rate that says nothing about it.

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

// 60 Hz is what a page is served if it is keeping up, so this is the budget a frame
// has. `struggling()` in the frontend calls a window under 20 fps or with a frame
// over 100 ms a fault, and those are the same thresholds printed below.
const FRAME_BUDGET_MS = 1000 / 60;
const STRUGGLING_FPS = 20;
const STRUGGLING_FRAME_MS = 100;

// Only the rig, because it is the panel whose cost is being asked about.
const RIG_ONLY = { type: 'Tabs', panels: ['rig'], active: 0 };

let chromium;
try {
	({ chromium } = await import('playwright'));
} catch {
	console.error('  this needs playwright, which is not installed here.');
	console.error('');
	console.error('    npm --prefix frontend install -D playwright');
	console.error('    npx --prefix frontend playwright install chromium');
	console.error('');
	console.error('  Optional on purpose: a plain checkout should not have to pull a');
	console.error('  browser down to build the console.');
	process.exit(2);
}

const median = (values) => {
	const sorted = [...values].sort((a, b) => a - b);
	const middle = Math.floor(sorted.length / 2);
	return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};

const spread = (values) => {
	const mid = median(values);
	if (!mid) return 0;
	return (Math.max(...values) - Math.min(...values)) / mid;
};

const url = `http://localhost:${port}`;
const station = connect(port, { timeoutMs: 30_000 });
const { get } = station;

await station.open.catch((error) => {
	console.error(`  ${error.message}`);
	process.exit(1);
});

let browser;
try {
	const fixtures = await get(['fixtures']);

	// A real window rather than a headless one would be a different workload — a
	// browser throttles what it is not compositing — but headless Chromium still
	// serves animation frames, which is the thing being measured.
	browser = await chromium.launch({ headless: true });
	const context = await browser.newContext({ viewport: { width: 1600, height: 900 } });

	// The layout, before the app boots. `addInitScript` runs in every document of the
	// context before any page script does, which is the one moment `localStorage` can
	// be written and be seen by the store that reads it on load.
	await context.addInitScript(
		([key, tree]) => {
			try {
				window.localStorage.setItem(
					key,
					JSON.stringify({ active: { kind: 'preset', key: 'rig-only' }, tree, dirty: true })
				);
			} catch {
				// A page that cannot store still opens; it opens on the default layout,
				// and the caller is told what it actually drew below.
			}
		},
		['pult.layout', RIG_ONLY]
	);

	const page = await context.newPage();
	// Errors from the page reach the station through `log.report` anyway, but a run
	// that drew nothing because the bundle failed should say so here rather than
	// being read as a browser that is very fast.
	const problems = [];
	page.on('pageerror', (error) => problems.push(String(error.message ?? error)));

	await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 });

	console.log(`  opened ${url} on a rig of ${fixtures.length} fixtures`);
	console.log('  waiting for the page to start reporting');

	// ── Collecting windows ──────────────────────────────────────────────────────
	//
	// Deduped by `at_ms`, which is the station's stamp on the report and the same
	// field `trace.ts` dedupes a sparkline by. A page that has gone quiet is still in
	// the map with its last figures, and counting those again would draw a flat line
	// that reads as steady work.
	const collected = [];
	const seen = new Set();
	const wanted = windows + 1;
	const deadline = Date.now() + wanted * 3000 + 30_000;

	while (collected.length < wanted && Date.now() < deadline) {
		await sleep(250);
		const clients = await get(['clients']).catch(() => null);
		const rows = clients ? Object.values(clients) : [];
		for (const row of rows) {
			if (seen.has(row.at_ms)) continue;
			seen.add(row.at_ms);
			collected.push(row);
		}
	}

	if (collected.length < 3) {
		console.error('');
		console.error(`  only ${collected.length} report${collected.length === 1 ? '' : 's'} arrived.`);
		if (problems.length) {
			console.error('  The page reported errors, which is the likely reason:');
			for (const problem of problems.slice(0, 3)) console.error(`    ${problem}`);
		} else {
			console.error('  A debug build serves frontend/build off the disk — if that directory');
			console.error('  is missing or stale, run: npm --prefix frontend run build');
		}
		process.exit(1);
	}

	// The first is thrown away for the reason the station's is: it covers the page
	// loading, the socket opening and the clock estimate being taken, none of which
	// is what a browser costs while somebody is using it.
	const kept = collected.slice(1);
	const drawing = kept.filter((row) => row.frames);

	const ms = (n) => `${n.toFixed(n < 10 ? 2 : 1)} ms`;
	const pad = (s, n) => String(s).padEnd(n);
	const rpad = (s, n) => String(s).padStart(n);

	console.log('');
	if (build) console.log(`  ${build} build, headless chromium`);
	console.log(`  median of ${kept.length} windows, first discarded`);
	console.log('');

	if (drawing.length === 0) {
		// A page drawing nothing measures nothing and says so, which is the same rule
		// an idle connector follows by carrying no `FrameCost` at all.
		console.log('  The page reported, but drew nothing in any window.');
		console.log('  `frames` was absent throughout, which means no animation frames were');
		console.log('  served — a backgrounded tab, or a workspace with no drawing panel in it.');
		process.exit(1);
	}

	const frameMs = drawing.map((row) => row.frames.mean_ms);
	const worst = Math.max(...drawing.map((row) => row.frames.max_ms));
	const evaluating = drawing.map((row) => row.frames.evaluating_mean_ms);
	const fps = drawing.map((row) =>
		row.frames.window_ms ? (row.frames.frames * 1000) / row.frames.window_ms : 0
	);
	const parameters = median(drawing.map((row) => row.frames.parameters));
	const variation = spread(frameMs);

	console.log(
		`  ${pad('page', 14)}${rpad('frame', 10)}${rpad('±', 8)}${rpad('worst', 10)}` +
			`${rpad('evaluating', 12)}${rpad('fps', 7)}${rpad('parameters', 12)}`
	);
	console.log(`  ${'─'.repeat(73)}`);
	console.log(
		`  ${pad(label || 'rig', 14)}${rpad(ms(median(frameMs)), 10)}` +
			`${rpad(`${(variation * 100).toFixed(0)}%`, 8)}${rpad(ms(worst), 10)}` +
			`${rpad(ms(median(evaluating)), 12)}${rpad(median(fps).toFixed(0), 7)}` +
			`${rpad(parameters.toLocaleString(), 12)}`
	);

	const share = ((median(frameMs) / FRAME_BUDGET_MS) * 100).toFixed(0);
	const crossing = ((median(evaluating) / median(frameMs)) * 100).toFixed(0);
	console.log('');
	console.log(
		`  ${share}% of a ${FRAME_BUDGET_MS.toFixed(1)} ms frame at 60 Hz; the evaluator crossing ` +
			`is ${crossing}% of it`
	);

	// The evaluator crossing is called out on its own because it is the figure that
	// survives whatever the viewer is rewritten into. What the beam drawing costs
	// today is the disposable half.
	const offsets = kept.map((row) => row.clock_offset_ms).filter((v) => v != null);
	if (offsets.length) {
		console.log(`  clock offset ${median(offsets).toFixed(1)} ms against the station`);
	} else {
		console.log('  the page never placed itself on the station clock — it was showing gaps');
	}

	const struggled = drawing.filter(
		(row) =>
			(row.frames.window_ms ? (row.frames.frames * 1000) / row.frames.window_ms : 0) <
				STRUGGLING_FPS || row.frames.max_ms > STRUGGLING_FRAME_MS
	);
	if (struggled.length) {
		console.log('');
		console.log(
			`  ⚠ ${struggled.length} of ${drawing.length} windows were struggling ` +
				`(under ${STRUGGLING_FPS} fps, or a frame over ${STRUGGLING_FRAME_MS} ms).`
		);
		console.log('  Each of those also became a warn line in the system log, by the same rule.');
	}

	console.log('');
	console.log('  These are NOT comparable with `--measure`. A page drawing this rig competes');
	console.log('  for the CPU that run holds still, so the station figures taken beside these');
	console.log('  would be worse than the ones that run prints, and neither set is wrong.');
	console.log('');
	console.log('  frame = the gap between animation frames, not the work inside one: a page');
	console.log('  served a frame every 200 ms is stuttering however cheap its own work was.');
	console.log('');

	process.exit(0);
} catch (error) {
	console.error(`  measuring the browser failed: ${error.message}`);
	process.exit(1);
} finally {
	await browser?.close().catch(() => {});
}
