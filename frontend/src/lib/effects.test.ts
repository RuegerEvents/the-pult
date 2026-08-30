import { describe, it, expect } from 'vitest';

import type { EffectSpec, ParameterValue } from './generated/index.js';
import {
	curveLevel,
	cyclePosition,
	defaultSpec,
	ease,
	mulberry32,
	phases,
	shapesFor,
	specsFor,
	stepValue,
	valueAt
} from './effects.js';

const float = (v: number): ParameterValue => ({ type: 'Float', value: v });

describe('handing out phases', () => {
	/**
	 * A selection of one is in step with itself whatever was asked for. Every spread
	 * has to survive it, because "chase these" with one light selected is an ordinary
	 * thing to do by accident and dividing by n − 1 would give NaN.
	 */
	it('gives a lone fixture phase zero, whatever the spread', () => {
		for (const spread of ['Even', 'Linear', 'Reversed', 'Centre'] as const) {
			expect(phases(spread, 1)).toEqual([0]);
		}
		expect(phases({ Wings: 2 }, 1)).toEqual([0]);
		expect(phases({ Groups: 3 }, 1)).toEqual([0]);
		expect(phases({ Random: { seed: 7 } }, 1)).toEqual([0]);
	});

	it('gives nothing for nothing', () => {
		expect(phases('Linear', 0)).toEqual([]);
	});

	it('puts everything in step for Even', () => {
		expect(phases('Even', 4)).toEqual([0, 0, 0, 0]);
	});

	/**
	 * A cycle spread across the selection, and deliberately `i / n` rather than
	 * `i / (n - 1)`: the last fixture should be one step short of the first, not on
	 * top of it, or a four-light chase would look like three.
	 */
	it('spreads one whole cycle across a chase', () => {
		expect(phases('Linear', 4)).toEqual([0, 0.25, 0.5, 0.75]);
		// Compared loosely: fifths are not exact in binary, and pinning the artefacts
		// would be a test about IEEE 754 rather than about spreads.
		phases('Linear', 5).forEach((p, i) => expect(p).toBeCloseTo(i / 5, 10));
	});

	it('hands the same phases out backwards for Reversed', () => {
		expect(phases('Reversed', 4)).toEqual([0.75, 0.5, 0.25, 0]);
		expect(new Set(phases('Reversed', 4))).toEqual(new Set(phases('Linear', 4)));
	});

	/** The ends together and the middle opposite: a row breathing outwards. */
	it('makes Centre symmetric about the middle', () => {
		expect(phases('Centre', 5)).toEqual([0.5, 0.25, 0, 0.25, 0.5]);
		const four = phases('Centre', 4);
		expect(four[0]).toBeCloseTo(four[3], 10);
		expect(four[1]).toBeCloseTo(four[2], 10);
	});

	/** Mirrored halves, so each wing sweeps outwards rather than across the stage. */
	it('mirrors alternate wings', () => {
		expect(phases({ Wings: 2 }, 4)).toEqual([0, 0.5, 0.5, 0]);
	});

	it('puts every nth fixture in step for Groups', () => {
		expect(phases({ Groups: 3 }, 6)).toEqual([0, 1 / 3, 2 / 3, 0, 1 / 3, 2 / 3]);
	});

	/**
	 * A random spread has to be the same on every console and the same every time it
	 * is drawn, or two stations would chase differently and the panel would disagree
	 * with the lights. The seed is stored; the phases are rebuilt from it.
	 */
	it('gives the same random spread for the same seed, and a different one otherwise', () => {
		const first = phases({ Random: { seed: 12345 } }, 5);
		expect(phases({ Random: { seed: 12345 } }, 5)).toEqual(first);
		expect(phases({ Random: { seed: 12346 } }, 5)).not.toEqual(first);
		expect(first.every((p) => p >= 0 && p < 1)).toBe(true);
	});

	it('the generator stays inside 0..1', () => {
		const next = mulberry32(99);
		for (let i = 0; i < 500; i++) {
			const v = next();
			expect(v).toBeGreaterThanOrEqual(0);
			expect(v).toBeLessThan(1);
		}
	});
});

/**
 * The same table `model/effects.rs` and the two node implementations assert. A
 * browser that disagreed would draw a dot in a place the light is not.
 */
describe('the numeric table, again', () => {
	it('starts a sine halfway up and peaks a quarter in', () => {
		expect(curveLevel('Sine', 0.5, 0)).toBeCloseTo(0.5, 4);
		expect(curveLevel('Sine', 0.5, 0.25)).toBeCloseTo(1, 4);
		expect(curveLevel('Sine', 0.5, 0.5)).toBeCloseTo(0.5, 4);
		expect(curveLevel('Sine', 0.5, 0.75)).toBeCloseTo(0, 4);
	});

	it('rises a triangle over the first half', () => {
		expect(curveLevel('Triangle', 0.5, 0.25)).toBeCloseTo(0.5, 4);
		expect(curveLevel('Triangle', 0.5, 0.5)).toBeCloseTo(1, 4);
	});

	it('spends width of a square high', () => {
		expect(curveLevel('Square', 0.5, 0.49)).toBe(1);
		expect(curveLevel('Square', 0.5, 0.5)).toBe(0);
		expect(curveLevel('Square', 0.25, 0.26)).toBe(0);
	});

	it('runs the saws opposite ways', () => {
		expect(curveLevel('SawUp', 0.5, 0.75)).toBeCloseTo(0.75, 4);
		expect(curveLevel('SawDown', 0.5, 0.75)).toBeCloseTo(0.25, 4);
	});

	it('takes a second over a one-hertz cycle', () => {
		expect(cyclePosition(1, false, 0, 1000, 1250)).toBeCloseTo(0.25, 6);
		expect(cyclePosition(1, false, 0, 1000, 2000)).toBeCloseTo(0, 6);
	});

	it('wraps rather than going negative before the anchor', () => {
		expect(cyclePosition(1, false, 0, 1000, 750)).toBeCloseTo(0.75, 6);
	});

	it('holds at the phase when the rate is zero', () => {
		expect(cyclePosition(0, false, 0.3, 0, 999999)).toBeCloseTo(0.3, 6);
	});

	it('runs every easing from nothing to everything', () => {
		for (const e of ['Step', 'Linear', 'EaseIn', 'EaseOut', 'EaseInOut'] as const) {
			expect(ease(e, 0)).toBe(0);
			expect(ease(e, 1)).toBe(1);
		}
	});
});

describe('a step list', () => {
	const chase = [
		{ at: 0, value: float(0), easing: 'Step' as const },
		{ at: 0.5, value: float(1), easing: 'Step' as const }
	];

	it('shows each step from where it starts', () => {
		expect(stepValue(chase, 0)).toEqual(float(0));
		expect(stepValue(chase, 0.49)).toEqual(float(0));
		expect(stepValue(chase, 0.5)).toEqual(float(1));
	});

	it('renders the same however the steps were ordered', () => {
		const shuffled = [chase[1], chase[0]];
		for (const x of [0, 0.25, 0.5, 0.9]) {
			expect(stepValue(shuffled, x)).toEqual(stepValue(chase, x));
		}
	});

	it('crossfades when a step asks it to, wrapping past the end', () => {
		const smooth = chase.map((s) => ({ ...s, easing: 'Linear' as const }));
		expect(stepValue(smooth, 0.25)).toEqual(float(0.5));
		// Three quarters round is halfway from the second step back to the first.
		expect(stepValue(smooth, 0.75)).toEqual(float(0.5));
	});

	it('has nothing to show with no steps', () => {
		expect(stepValue([], 0.5)).toBeNull();
	});
});

describe('building an effect for a selection', () => {
	const base = {
		curve: { Shape: 'Sine' as const },
		rate: { Hz: 1 },
		low: float(0),
		high: float(1),
		width: 0.5,
		direction: 'Forward' as const,
		spread: 'Even' as const,
		t0: null
	};

	/**
	 * One id across the selection is what lets the panel gather them back into a
	 * single editable effect, rather than the operator finding six unrelated sines.
	 */
	it('gives every fixture the same effect id and its own phase', () => {
		const specs = specsFor(['a', 'b', 'c', 'd'], base, 'Linear');
		const ids = new Set(Object.values(specs).map((s) => s.effect_id));

		expect(ids.size).toBe(1);
		expect(Object.values(specs).map((s) => s.phase)).toEqual([0, 0.25, 0.5, 0.75]);
		expect(Object.keys(specs)).toEqual(['a', 'b', 'c', 'd']);
	});

	it('remembers which spread was asked for, so it can be re-applied later', () => {
		const specs = specsFor(['a', 'b'], base, { Groups: 2 });
		expect(Object.values(specs)[0].spread).toEqual({ Groups: 2 });
	});

	/**
	 * The anchor is set on apply, not on building: one set when the panel opened
	 * would start the effect part way through its first cycle.
	 */
	it('leaves a fresh effect unanchored', () => {
		expect(defaultSpec('Intensity', float(0)).t0).toBeNull();
	});

	it('starts a boolean on the one shape that only asks for the ends', () => {
		const relay = defaultSpec({ Switch: 0 }, { type: 'Bool', value: false });
		expect(relay.curve).toEqual({ Shape: 'Square' });
		expect(shapesFor({ type: 'Bool', value: false })).toEqual(['Square']);
		expect(shapesFor({ type: 'Text', value: '' })).toEqual([]);
	});

	it('reads a spec back at a moment, for the dot on the waveform', () => {
		const spec: EffectSpec = { ...base, effect_id: 'fx', phase: 0 };
		expect(valueAt(spec, 1, 0, 250)).toEqual(float(1));
		expect(valueAt(spec, 1, 0, 750)).toEqual(float(0));
	});
});
