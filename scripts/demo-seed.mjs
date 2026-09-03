// Put a rig worth measuring in a fresh demo show, over the ordinary WebSocket API.
//
// Nothing here is privileged — it is the same protocol the frontend speaks, so if
// this drifts out of date it will fail loudly rather than quietly doing the wrong
// thing. Run by scripts/demo.sh.
//
//   node scripts/demo-seed.mjs <port> [--size big|huge|<n>] [--cues <n>] [--slice <f>]
//
// This is the *instrument*, and it goes over the public API on purpose: what it
// measures is then what an operator would feel. The shows meant to be looked at are
// the console's own — `--demo haunt|theatre|club|festival`, seeded in Rust at open
// time, which is what a card on the welcome screen presses. `small` used to be here
// and is `--demo haunt` now.
//
// `big` and `huge` are hundreds or thousands of fixtures, a stack of cues over
// several sequences, and effects left running so the station is actually ticking.
//
// `--size <n>` is any rig size, because the shape of the curve is the answer and not
// one point on it. The cue stack and the slice are separate axes — `--cues` and
// `--slice` — so a run can grow the rig while holding what the stack costs, or the
// other way about. Which matters: three hundred cues over five thousand fixtures at
// a tenth each is a hundred and fifty thousand captures, and past some point that
// measures JSON and SQLite rather than lighting.
//
// The default keeps the stack's cost flat as the rig grows. `--size <n>` alone holds
// captures per cue at `huge`'s absolute number rather than its fraction, so the
// frame cost is what changes between two runs and not the size of the show.

import zlib from 'node:zlib';

import { connect, inWindow } from './demo-ws.mjs';

// ── Which show ────────────────────────────────────────────────────────────────

/**
 * A placement: where a thing is, how it is turned, and its size.
 *
 * Rotations are XYZ Euler degrees, so a rest direction is a rotation rather than a
 * vector. The two this seed uses are written out with the direction they mean, since
 * nobody can read `{ x: 143.1301, y: 0, z: 180 }` and see a light pointing downstage
 * and down. `crates/pult-schema/src/types/scene.rs` is where the conversion lives;
 * this file cannot import it, so it carries the two answers instead of a fourth copy
 * of the arithmetic.
 */
const placed = (at, rotation = { x: 0, y: 0, z: 0 }) => ({
	position: at,
	rotation,
	scale: { x: 1, y: 1, z: 1 }
});

/** Facing (0, -0.8, 0.6): downstage and down, which is how the demo heads hang. */
const DOWNSTAGE_AND_DOWN = { x: 143.1301, y: 0, z: 180 };

/** Facing (0, -0.9138, 0.4061): the same idea, a little steeper, for the big rig. */
const STEEPER = { x: 156.0375, y: 0, z: 180 };

const argv = process.argv.slice(2);
let port = '7700';
let size = 'big';
let askedCues = null;
let askedSlice = null;
for (let i = 0; i < argv.length; i++) {
	if (argv[i] === '--size') size = argv[++i];
	else if (argv[i] === '--cues') askedCues = Number(argv[++i]);
	else if (argv[i] === '--slice') askedSlice = Number(argv[++i]);
	else if (!argv[i].startsWith('--')) port = argv[i];
}

/**
 * What each preset adds on top of the hand-made show.
 *
 * The fixture counts are the ones task 29 measured at — 500 is a console working
 * hard, 2000 is one that has stopped keeping 40 Hz — and the rest is shaped to go
 * with them. A cue captures a *slice* of the rig rather than all of it: three
 * hundred cues times two thousand fixtures would be six hundred thousand captures,
 * which measures JSON rather than lighting, and a real cue stack does not touch
 * everything in every cue either.
 */
const PRESETS = {
	big: { heads: 500, cues: 60, sequences: 4, sliceShare: 0.15, plans: 0 },
	huge: { heads: 2000, cues: 300, sequences: 12, sliceShare: 0.1, plans: 3 }
};

/**
 * How many fixtures each cue captures in the `huge` preset: 0.1 of 2000.
 *
 * This is the number held constant when a size is given as a count, rather than the
 * fraction it came from. A fraction held constant makes the cue stack grow with the
 * rig, and then a run at 5000 is measuring a show four times the size of the one at
 * 1250 in two ways at once — which is exactly the confusion `--size <n>` exists to
 * avoid.
 */
const CAPTURES_PER_CUE = 200;

/** A rig of `n`, shaped like the presets but at whatever size was asked for. */
function sized(n) {
	return {
		heads: n,
		// Held rather than scaled. A stack is a stack: an evening has a few hundred
		// cues in it whether the rig is fifty lamps or five thousand.
		cues: 300,
		sequences: 12,
		sliceShare: Math.min(1, CAPTURES_PER_CUE / n),
		// Three plans is what `huge` draws, and drawing is not what a sized run is
		// measuring. Kept so the two are comparable.
		plans: 3
	};
}

let preset;
if (size in PRESETS) {
	preset = PRESETS[size];
} else if (Number.isFinite(Number(size)) && Number(size) > 0) {
	preset = sized(Math.floor(Number(size)));
} else if (size === 'small') {
	console.error(
		'  `small` is the console\'s own demo now: start it with --demo haunt, or open it\n' +
			'  from the welcome screen. This script is the measurement instrument.'
	);
	process.exit(2);
} else {
	console.error(
		`  unknown size "${size}" — one of ${Object.keys(PRESETS).join(', ')}, or a fixture count`
	);
	process.exit(2);
}

// The two axes, applied over whatever the size decided. Given explicitly they win,
// which is how the curve gets taken along one axis at a time.
if (askedCues !== null) {
	if (!Number.isFinite(askedCues) || askedCues < 0) {
		console.error(`  --cues wants a count, not "${askedCues}"`);
		process.exit(2);
	}
	preset = { ...preset, cues: Math.floor(askedCues) };
}
if (askedSlice !== null) {
	if (!Number.isFinite(askedSlice) || askedSlice <= 0 || askedSlice > 1) {
		console.error(`  --slice wants a fraction between 0 and 1, not "${askedSlice}"`);
		process.exit(2);
	}
	preset = { ...preset, sliceShare: askedSlice };
}
if (!preset && (askedCues !== null || askedSlice !== null)) {
	console.error('  --cues and --slice need a generated rig: pass --size big, huge or a count');
	process.exit(2);
}

// A big seed puts many writes in flight at once, so an individual one can sit in the
// engine's queue for a while through no fault of its own. The timeout has to be long
// enough that queueing is not mistaken for a broken station.
const station = connect(port, { timeoutMs: preset ? 60_000 : 5000 });
const { get, set, create } = station;

const id = () => crypto.randomUUID();

/** Say how far along a long write is, without a line per row. */
const progress = (what, total) => {
	if (total < 500) return undefined;
	let last = 0;
	return (done) => {
		const step = Math.ceil(total / 10);
		if (done - last >= step || done === total) {
			last = done;
			process.stdout.write(`    ${what}: ${done}/${total}\r`);
			if (done === total) process.stdout.write('\n');
		}
	};
};

// ── A plan to hang the rig on ─────────────────────────────────────────────────
//
// A `StagePlan` names its image by sha256 in the asset store, so seeding one means
// POSTing bytes to `/assets` and using the digest that comes back. Which is the same
// public API as everything else here, reached by a different verb — assets are bytes
// rather than replicated fields, and always have been.

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
	let c = n;
	for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
	return c >>> 0;
});
const crc32 = (buf) => {
	let c = 0xffffffff;
	for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
	return (c ^ 0xffffffff) >>> 0;
};

/** A real PNG of one flat colour — enough for a plan to have an image. */
function png(width, height, [r, g, b]) {
	const chunk = (type, data) => {
		const head = Buffer.alloc(8);
		head.writeUInt32BE(data.length, 0);
		head.write(type, 4, 'ascii');
		const crc = Buffer.alloc(4);
		crc.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, 'ascii'), data])), 0);
		return Buffer.concat([head, data, crc]);
	};

	const ihdr = Buffer.alloc(13);
	ihdr.writeUInt32BE(width, 0);
	ihdr.writeUInt32BE(height, 4);
	ihdr[8] = 8; // bit depth
	ihdr[9] = 2; // truecolour
	// Rows are filter byte 0 followed by RGB triples.
	const raw = Buffer.concat(
		Array.from({ length: height }, () =>
			Buffer.concat([
				Buffer.from([0]),
				Buffer.concat(Array.from({ length: width }, () => Buffer.from([r, g, b])))
			])
		)
	);

	return Buffer.concat([
		Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
		chunk('IHDR', ihdr),
		chunk('IDAT', zlib.deflateSync(raw)),
		chunk('IEND', Buffer.alloc(0))
	]);
}

/**
 * `count` plans, each big enough to have the whole rig on it.
 *
 * Scaled from the rig's own extent rather than a fixed size, so the Plan panel opens
 * on a plan the fixtures are actually *on* — a plan the rig overflows is a worse
 * demo than no plan at all.
 */
async function seedPlans(count, extentM) {
	if (!count) return 0;

	for (let n = 0; n < count; n++) {
		const width = 1000;
		const height = 1000;
		const bytes = png(width, height, [40 + n * 30, 40, 60]);
		const posted = await fetch(`http://127.0.0.1:${port}/assets`, {
			method: 'POST',
			headers: { 'content-type': 'image/png' },
			body: bytes
		});
		if (!posted.ok) throw new Error(`uploading a plan image: ${posted.status}`);
		const { sha256 } = await posted.json();

		await create('stage_plans', {
			id: id(),
			name: `Level ${n + 1}`,
			asset: sha256,
			width_px: width,
			height_px: height,
			origin: { x: -extentM / 2, y: 0, z: -extentM / 2 },
			metres_per_pixel: extentM / width,
			rotation_deg: 0,
			opacity: 0.8,
			visible: n === 0
		});
	}
	return count;
}

await station.open.catch((error) => {
	console.error(`  ${error.message}`);
	process.exit(1);
});

try {
	// The rig rather than the show row: the station seeds a `show` for every file it
	// opens now, so a row is no longer evidence that anybody has done anything.
	const already = await get(['fixtures']);
	if (already?.length) {
		console.log('  this show already has a rig in it; leaving it alone');
		process.exit(0);
	}

	// The station gives every show it loads an operator, so undo works on the demo
	// without anybody being asked who they are. Nothing is created here — this
	// checks the assumption rather than making it true, and says so if it stops
	// holding, which for a script that is also documentation is the useful half.
	const users = await get(['users']);
	if (!users?.length) {
		console.error('  no operator on a fresh show — undo would not work here');
		process.exit(1);
	}

	// The name only. The row itself is the station's — it fills in the history depth,
	// the home fade and the haze from this console's preferences when it opens the
	// file, and replacing the whole singleton here would put those back to nothing.
	await set(['show', 'name'], `Demo ${size}`);

	// A moving head, so there is something to puppeteer. Nothing binds a channel:
	// where a parameter sits belongs to a mode, and a type that names none has the
	// implicit one — a byte per parameter in the order they are listed, three for a
	// colour. So this is intensity at 1, the colour across 2 to 4, pan at 5, tilt at 6.
	const spot = {
		id: id(),
		name: 'Spot',
		manufacturer: 'Generic',
		channel_count: 6,
		parameters: [
			{
				kind: 'Intensity',
				direction: 'Output',
				binding: null,
				default_value: { type: 'Float', value: 0 }
			},
			{
				kind: 'ColorRgb',
				direction: 'Output',
				binding: null,
				default_value: { type: 'Color', value: { r: 1, g: 1, b: 1 } }
			},
			{
				kind: 'Pan',
				direction: 'Output',
				binding: null,
				default_value: { type: 'Float', value: 0.5 }
			},
			{
				kind: 'Tilt',
				direction: 'Output',
				binding: null,
				default_value: { type: 'Float', value: 0.5 }
			}
		]
	};
	await create('fixture_types', spot);

	// The captures a generated cue is made of. Two shapes: a level, and the same
	// parameter driven by a shape instead.
	const capture = (fixture, level) => ({
		fixture_id: fixture.id,
		parameter_kind: 'Intensity',
		value: { type: 'Float', value: level },
		fade_in_ms: 0,
		fade_out_ms: 0,
		delay_in_ms: 0
	});

	// A tempo for effects to follow. 120 bpm halved is one cycle a second: slow
	// enough to watch, fast enough to be obviously moving.
	const master = {
		id: id(),
		name: 'Chases',
		bpm: 120,
		multiplier: 0.5,
		running: true,
		t0: Date.now()
	};
	await create('speed_masters', master);

	// One id across every head that carries it, so the effects panel gathers them
	// back into a single editable effect rather than hundreds of unrelated sines.
	const chaseId = id();

	/**
	 * A colour sine on one head, at the phase given.
	 *
	 * Stored with `t0: null`: a capture's anchor is the cue's `went_at`, decided
	 * afresh on every Go, so two consoles replaying this cue start the same cycle
	 * rather than each remembering its own.
	 */
	const sine = (fixture, phase) => ({
		fixture_id: fixture.id,
		parameter_kind: 'ColorRgb',
		value: { type: 'Color', value: { r: 0, g: 0, b: 0 } },
		fade_in_ms: 0,
		fade_out_ms: 0,
		delay_in_ms: 0,
		effect: {
			effect_id: chaseId,
			curve: { Shape: 'Sine' },
			rate: { Master: { id: master.id, multiplier: 1 } },
			low: { type: 'Color', value: { r: 0.4, g: 0, b: 0 } },
			high: { type: 'Color', value: { r: 0, g: 0.2, b: 1 } },
			width: 0.5,
			direction: 'Forward',
			phase,
			spread: 'Linear',
			t0: null
		},
		easing: 'Linear'
	});

	// ── A rig worth measuring ─────────────────────────────────────────────────
	//
	// Everything below goes through the same `create` the hand-made show above
	// uses, just a great many more of them and with a window of them in flight at
	// once. That is deliberate: a seed this size is the largest exercise of the
	// write path in the repo, and a generator that wrote the showfile directly
	// would skip the engine, the oplog and every validation on the way in.

	const began = Date.now();
	const say = (what) => console.log(`  ${what}`);

	// A six-channel head, so a universe holds 85 of them and a big rig is spread
	// across as many universes as it needs. All of them the same type: what is
	// being measured is the size of the rig, not the variety of it.
	const CHANNELS = spot.channel_count;
	const PER_UNIVERSE = Math.floor(512 / CHANNELS);

	/**
	 * Hung on a grid over the stage, facing down and downstage.
	 *
	 * Placed rather than null because half the cost of a tick is what leaves the
	 * console once a fixture has moved, and because a rig with no positions draws
	 * nothing in the 3D view — which is the panel most likely to be the reason
	 * somebody wanted a big rig in the first place.
	 */
	const across = Math.ceil(Math.sqrt(preset.heads));
	const generated = Array.from({ length: preset.heads }, (_, i) => {
		const column = i % across;
		const row = Math.floor(i / across);
		return {
			id: id(),
			name: `Head ${i + 1}`,
			fixture_type_id: spot.id,
			address: {
				Dmx: {
					mode: 'Default',
					breaks: [
						{
							universe: 2 + Math.floor(i / PER_UNIVERSE),
							address: 1 + (i % PER_UNIVERSE) * CHANNELS
						}
					]
				}
			},
			position: placed(
				{
					x: (column - (across - 1) / 2) * 1.2,
					y: 5 + (row % 3) * 0.5,
					z: (row - (across - 1) / 2) * 1.2
				},
				STEEPER
			)
		};
	});

	say(`patching ${generated.length} fixtures across ${Math.ceil(preset.heads / PER_UNIVERSE)} universes`);
	await inWindow(generated, (fixture) => create('fixtures', fixture), {
		onProgress: progress('fixtures', generated.length)
	});

	/**
	 * A cue stack over the generated rig.
	 *
	 * Each cue takes a slice rather than the whole rig — a stride through the list
	 * so consecutive cues light different fixtures — and one cue in every eight
	 * carries the colour effect, so the show has something moving in it wherever
	 * the operator happens to be in the stack.
	 */
	const sliceSize = Math.max(1, Math.round(generated.length * preset.sliceShare));
	const generatedCues = Array.from({ length: preset.cues }, (_, c) => {
		const from = (c * sliceSize) % generated.length;
		const slice = Array.from(
			{ length: sliceSize },
			(_, k) => generated[(from + k) % generated.length]
		);
		const chased = c % 8 === 0;
		return {
			id: id(),
			name: `Cue ${c + 1}`,
			number: c + 1,
			captures: slice.map((fixture, k) =>
				chased ? sine(fixture, (k % 8) / 8) : capture(fixture, 0.3 + (c % 7) / 10)
			),
			follow_mode: 'Manual',
			fade_in_ms: 1000 + (c % 5) * 500,
			fade_out_ms: (c % 3) * 500,
			is_active: false
		};
	});

	say(`writing ${generatedCues.length} cues of ${sliceSize} fixtures each`);
	await inWindow(generatedCues, (cue) => create('cues', cue), {
		onProgress: progress('cues', generatedCues.length)
	});

	// Several sequences rather than one, because a console runs a handful at once
	// and playback costs what it costs per *live* sequence.
	const perSequence = Math.ceil(generatedCues.length / preset.sequences);
	const generatedSequences = Array.from({ length: preset.sequences }, (_, n) => ({
		id: id(),
		name: `Stack ${n + 1}`,
		cue_ids: generatedCues.slice(n * perSequence, (n + 1) * perSequence).map((c) => c.id),
		active_cue_index: null,
		went_at: null
	})).filter((seq) => seq.cue_ids.length > 0);

	for (const seq of generatedSequences) await create('sequences', seq);

	// And left running. A station with nothing moving settles and stops ticking, so
	// a preset that had to be driven by hand before it could be measured would not
	// be the preset this exists to be.
	for (const seq of generatedSequences) {
		await station.call('sequences.goNext', { sequenceId: seq.id });
	}

	// A little wider than the rig, so it sits inside its plan rather than on its edge.
	const plans = await seedPlans(preset.plans, across * 1.2 * 1.1);

	const took = ((Date.now() - began) / 1000).toFixed(1);
	// The slice is printed as both a fraction and a count. The count is what actually
	// decides the write path's cost, and it is the one a reader comparing two runs at
	// different sizes has to be able to see.
	const perCue = Math.max(1, Math.round(preset.heads * preset.sliceShare));
	console.log(
		`  seeded ${size}: ${generated.length} fixtures, ` +
			`${generatedCues.length} cues, ` +
			`${generatedSequences.length} sequences (all running), ` +
			`${plans} plans, in ${took}s`
	);
	console.log(
		`  each cue captures ${perCue} fixtures (slice ${preset.sliceShare.toFixed(3)}), ` +
			`${(generatedCues.length * perCue).toLocaleString()} captures in all`
	);
	process.exit(0);
} catch (error) {
	console.error(`  seeding failed: ${error.message}`);
	process.exit(1);
}
