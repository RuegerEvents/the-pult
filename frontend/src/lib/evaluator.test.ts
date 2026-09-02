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
	Evaluator: new () => {
		set_driving(driving: unknown): void;
		watch(keys: unknown): void;
		evaluate(nowMs: number): Float32Array;
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
