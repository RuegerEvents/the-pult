import { describe, it, expect, beforeAll } from 'vitest';
import { readFileSync } from 'node:fs';

import type { ParameterValue } from './generated/index.js';
import { unpack } from './evaluator.js';

/**
 * The corpus, evaluated in wasm.
 *
 * `testdata/driven-values.json` is read here and by
 * `crates/pult-render-wasm/tests/corpus.rs`, which asks the *native* build of the same
 * crate the same questions. Between them they are the guard `values-as-functions` put
 * in place of a TypeScript twin: there is only one implementation of the arithmetic,
 * so what has to be checked is not two implementations agreeing but two compilations
 * of one — and a wasm build that rounds a float differently, or a boundary that packs
 * a colour wrong, fails here rather than on stage.
 *
 * Skipped, loudly, when the artifact has not been built. `scripts/build-evaluator.sh`
 * writes it, CI runs that, and a run without it is a run that has not checked this —
 * which is worth saying rather than passing quietly.
 */

type Case = {
	name: string;
	driving: Record<string, unknown>;
	at: number;
	expect: ParameterValue | null;
};

const corpus: { cases: Case[] } = JSON.parse(
	readFileSync(new URL('../../../testdata/driven-values.json', import.meta.url), 'utf8')
);

type MixCase = {
	name: string;
	color: { r: number; g: number; b: number; overrides?: Record<string, number> };
	emitters: { name: string; rgb?: [number, number, number]; subtractive?: boolean }[];
	expect: number[];
};

const mixCorpus: { cases: MixCase[] } = JSON.parse(
	readFileSync(new URL('../../../testdata/color-mix.json', import.meta.url), 'utf8')
);

/** Close enough that a difference is a bug rather than a rounding. */
const TOLERANCE = 1e-3;

function agree(a: ParameterValue | null, b: ParameterValue | null): boolean {
	if (a === null || b === null) return a === b;
	if (a.type !== b.type) return false;
	if (a.type === 'Float' && b.type === 'Float') return Math.abs(a.value - b.value) < TOLERANCE;
	if (a.type === 'Color' && b.type === 'Color') {
		return (
			Math.abs(a.value.r - b.value.r) < TOLERANCE &&
			Math.abs(a.value.g - b.value.g) < TOLERANCE &&
			Math.abs(a.value.b - b.value.b) < TOLERANCE
		);
	}
	return JSON.stringify(a) === JSON.stringify(b);
}

type Built = {
	initSync: (opts: { module: BufferSource }) => unknown;
	emitter_levels: (color: unknown, emitters: unknown) => [string, number][];
	Evaluator: new () => {
		set_driving(driving: unknown): void;
		watch(keys: unknown): void;
		evaluate(nowMs: number): Float32Array;
		color_overrides(key: string, nowMs: number): Record<string, number>;
		load_track(sha: string, bytes: Uint8Array): void;
		play_track(sha: string, anchorMs: number, positionMs: number, rate: number): void;
		stop_track(sha: string): void;
	};
};

let built: Built | null = null;

beforeAll(async () => {
	try {
		const module = (await import('./evaluator/pult_render_wasm.js')) as unknown as Built;
		// `initSync` with the bytes: the `--target web` module fetches its own wasm from
		// a URL, which is right in a page and not a thing node does.
		const bytes = readFileSync(
			new URL('./evaluator/pult_render_wasm_bg.wasm', import.meta.url)
		);
		module.initSync({ module: bytes });
		built = module;
	} catch {
		built = null;
	}
});

describe('the evaluator, compiled for a browser', () => {
	it('has been built', () => {
		expect(
			built,
			'run scripts/build-evaluator.sh — without it nothing below has checked anything'
		).not.toBeNull();
	});

	it('agrees with the native build on every case in the corpus', () => {
		if (!built) return;
		expect(corpus.cases.length).toBeGreaterThan(30);

		const evaluator = new built.Evaluator();
		const driving: Record<string, unknown> = {};
		corpus.cases.forEach((c, at) => {
			driving[`c${at}/v`] = c.driving;
		});
		evaluator.set_driving(driving);

		// One case at a time, because each names its own moment and a frame is one
		// moment. That is also the shape a page uses: watch what is on screen, then ask
		// for all of it at once.
		const wrong: string[] = [];
		corpus.cases.forEach((c, at) => {
			evaluator.watch([`c${at}/v`]);
			const got = unpack(evaluator.evaluate(c.at), 0) ?? null;
			if (!agree(got, c.expect)) {
				wrong.push(`${c.name}: expected ${JSON.stringify(c.expect)}, got ${JSON.stringify(got)}`);
			}
		});
		expect(wrong, `${wrong.length} cases disagree`).toEqual([]);
	});

	it('answers a whole batch in one crossing, in the order it was asked', () => {
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving({
			'a/Intensity': { fade: fadeTo(1, 0, 1000) },
			'b/Intensity': { fade: fadeTo(0.5, 0, 1000) },
			'c/Intensity': { home: { type: 'Float', value: 0.75 } }
		});
		evaluator.watch(['c/Intensity', 'a/Intensity', 'b/Intensity']);

		const packed = evaluator.evaluate(500);
		expect(unpack(packed, 0)).toEqual({ type: 'Float', value: 0.75 });
		expect(unpack(packed, 1)).toEqual({ type: 'Float', value: 0.5 });
		expect(unpack(packed, 2)).toEqual({ type: 'Float', value: 0.25 });
	});
});

/**
 * One recording, written out by hand.
 *
 * Deliberately by hand rather than through a TypeScript encoder, because there must
 * not be a TypeScript encoder: `pult_render::track` is the only implementation and the
 * page hands it bytes. Writing the bytes here is an assertion about the *format* — that
 * it is little-endian, magic `PLTK`, version 1, and the layout the module header
 * describes — and it fails the moment either end changes its mind about that.
 */
function aTrack(fixtureId: string, key: string, points: [number, number][]): Uint8Array {
	const bytes: number[] = [];
	const u16 = (n: number) => bytes.push(n & 0xff, (n >> 8) & 0xff);
	const u32 = (n: number) => {
		bytes.push(n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff);
	};
	const f32 = (n: number) => {
		const view = new DataView(new ArrayBuffer(4));
		view.setFloat32(0, n, true);
		for (let i = 0; i < 4; i++) bytes.push(view.getUint8(i));
	};

	bytes.push(0x50, 0x4c, 0x54, 0x4b); // PLTK
	u16(1); // version
	u32(1); // one key
	for (const pair of fixtureId.replace(/-/g, '').match(/../g) ?? []) {
		bytes.push(parseInt(pair, 16));
	}
	const name = new TextEncoder().encode(key);
	u16(name.length);
	for (const byte of name) bytes.push(byte);
	u32(points.length);
	for (const [ms, value] of points) {
		u32(ms);
		bytes.push(0); // Float
		f32(value);
	}
	return new Uint8Array(bytes);
}

describe('a recording, played in the page', () => {
	const FIXTURE = '00000000-0000-0000-0000-0000000000ff';
	const KEY = `${FIXTURE}/Intensity`;

	/** A float that came back through an `f32`, so 0.4 is 0.4 rather than exactly it. */
	const at = (evaluator: { evaluate(ms: number): Float32Array }, ms: number) => {
		const value = unpack(evaluator.evaluate(ms), 0);
		expect(value?.type).toBe('Float');
		return value?.type === 'Float' ? value.value : NaN;
	};

	it('drives the parameter it recorded, stepwise, over what is under it', () => {
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving({ [KEY]: { home: { type: 'Float', value: 0.1 } } });
		evaluator.watch([KEY]);

		evaluator.load_track(
			'sha',
			aTrack(FIXTURE, 'Intensity', [
				[0, 0.25],
				[1000, 0.75]
			])
		);
		// Anchored at console 5000, playhead at 0.
		evaluator.play_track('sha', 5000, 0, 1);

		expect(at(evaluator, 5000)).toBeCloseTo(0.25, 5);
		expect(at(evaluator, 5999)).toBeCloseTo(0.25, 5);
		expect(at(evaluator, 6000)).toBeCloseTo(0.75, 5);

		// And stopping it hands the parameter back to what is underneath.
		evaluator.stop_track('sha');
		expect(at(evaluator, 6000)).toBeCloseTo(0.1, 5);
	});

	it('runs the playhead at the rate it was given', () => {
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving({ [KEY]: { home: { type: 'Float', value: 0 } } });
		evaluator.watch([KEY]);
		evaluator.load_track(
			'sha',
			aTrack(FIXTURE, 'Intensity', [
				[0, 0.25],
				[1000, 0.75]
			])
		);
		evaluator.play_track('sha', 0, 0, 2);
		expect(at(evaluator, 500)).toBeCloseTo(0.75, 5);
	});

	it('says nothing before its first change point, so what is under it shows through', () => {
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving({ [KEY]: { home: { type: 'Float', value: 0.4 } } });
		evaluator.watch([KEY]);
		evaluator.load_track('sha', aTrack(FIXTURE, 'Intensity', [[2000, 1]]));
		evaluator.play_track('sha', 0, 0, 1);
		expect(at(evaluator, 500)).toBeCloseTo(0.4, 5);
		expect(at(evaluator, 2500)).toBeCloseTo(1, 5);
	});
});

describe('mixing a colour onto the dies that make it', () => {
	it('agrees with the native build on every case in the corpus', () => {
		if (!built) return;
		expect(mixCorpus.cases.length).toBeGreaterThan(10);

		const wrong: string[] = [];
		for (const c of mixCorpus.cases) {
			const got = built.emitter_levels(c.color, c.emitters).map(([, level]) => level);
			const ok =
				got.length === c.expect.length &&
				got.every((level, at) => Math.abs(level - c.expect[at]) < TOLERANCE);
			if (!ok) {
				wrong.push(`${c.name}: expected ${JSON.stringify(c.expect)}, got ${JSON.stringify(got)}`);
			}
		}
		expect(wrong, `${wrong.length} cases disagree`).toEqual([]);
	});

	it('names the emitters in the order the fixture lists them', () => {
		if (!built) return;
		const levels = built.emitter_levels({ r: 1, g: 1, b: 1 }, [
			{ name: 'Red', rgb: [1, 0, 0] },
			{ name: 'White', rgb: [1, 1, 1] }
		]);
		expect(levels.map(([name]) => name)).toEqual(['Red', 'White']);
	});
});

describe("a colour's pinned emitters", () => {
	it('come back from the evaluator, which the packed answer has no room for', () => {
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving({
			'spot/ColorRgb': {
				home: { type: 'Color', value: { r: 1, g: 1, b: 1, overrides: { White: 0.25 } } }
			}
		});
		expect(evaluator.color_overrides('spot/ColorRgb', 0)).toEqual({ White: 0.25 });
	});

	it('are an empty map on anything that is not a colour', () => {
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving({ 'spot/Intensity': { home: { type: 'Float', value: 0.5 } } });
		expect(evaluator.color_overrides('spot/Intensity', 0)).toEqual({});
	});
});

/**
 * The boundary is not the new cost.
 *
 * The mistake this change is fixing one level up is paying a per-fixture price for
 * every value; a crossing per fixture per frame would be the same mistake in a
 * different currency. So the entry point is a batch, and this is the number that says
 * it was worth being one: the same work, asked for once rather than once each.
 *
 * Printed rather than bounded tightly. A loaded CI box would fail a duration
 * assertion, and a test that fails because somebody else was compiling is a test
 * people delete. The one assertion is the shape — that the batch is not *slower*.
 */
describe('what a frame costs', () => {
	it('is dominated by the arithmetic rather than by the crossing', () => {
		if (!built) return;

		// Forty movers on screen, five parameters each: a realistic set for a plan or
		// a 3D view, on a rig that may be very much larger.
		const ON_SCREEN = 40 * 5;
		const evaluator = new built.Evaluator();
		const driving: Record<string, unknown> = {};
		const keys: string[] = [];
		for (let i = 0; i < ON_SCREEN; i++) {
			const key = `f${i}/Intensity`;
			keys.push(key);
			driving[key] = { fade: fadeTo(1, 0, 4000) };
		}
		evaluator.set_driving(driving);

		const FRAMES = 200;

		evaluator.watch(keys);
		const batched = time(FRAMES, (frame) => {
			evaluator.evaluate(frame);
		});

		// The same values, one crossing each — which is what a page would do if it
		// asked per fixture instead.
		const oneAtATime = time(FRAMES, (frame) => {
			for (const key of keys) {
				evaluator.watch([key]);
				evaluator.evaluate(frame);
			}
		});
		evaluator.watch(keys);

		console.log(
			`  ${ON_SCREEN} parameters a frame: ${us(batched)} batched, ${us(oneAtATime)} one at a time`
		);
		expect(batched).toBeLessThan(oneAtATime);
	});
});

/** Microseconds per iteration, as a string. */
const us = (perFrame: number) => `${(perFrame * 1000).toFixed(1)} µs`;

/** Mean time of one iteration, in milliseconds. */
function time(iterations: number, run: (frame: number) => void): number {
	// A warm pass first: the first call through a fresh wasm instance pays for its
	// own JIT, and averaging that in measures the loader rather than the loop.
	run(0);
	const began = performance.now();
	for (let i = 0; i < iterations; i++) run(i);
	return (performance.now() - began) / iterations;
}

function fadeTo(to: number, t0: number, durationMs: number) {
	return {
		from: { type: 'Float', value: 0 },
		to: { type: 'Float', value: to },
		t0,
		duration_ms: durationMs,
		easing: 'Linear',
		cue_id: '00000000-0000-0000-0000-000000000000'
	};
}
