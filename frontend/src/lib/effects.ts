/**
 * Building an effect, and drawing one.
 *
 * The rendering here mirrors `pult-backend`'s `model/effects.rs` and exists for one
 * reason: a waveform with a dot on it, showing where each selected fixture sits in
 * the cycle *right now*. The values the lights take come from the engine, not from
 * this — a browser that disagreed with the engine would draw a lie, so these numbers
 * have to match, and the tests assert the same table the engine's do.
 *
 * The spread functions are the interesting half. Everything else is arithmetic.
 */

import type { Curve, EffectSpec, ParameterKind, ParameterValue, Shape, Spread, Step } from './generated/index.js';

// ── Spreads ───────────────────────────────────────────────────────────────────

/**
 * The phase each fixture in a selection gets, 0..1.
 *
 * This is where "make them chase" turns into a number per fixture. The engine never
 * sees a spread: by the time an effect is running, each entry carries its own phase,
 * which is what keeps rendering a pure function of one entry rather than of an entry
 * plus its position in a selection that may since have changed.
 *
 * A selection of one is always in phase with itself, whatever was asked for.
 */
export function phases(spread: Spread, n: number): number[] {
	if (n <= 0) return [];
	if (n === 1) return [0];

	const index = [...Array(n).keys()];

	if (spread === 'Even') return index.map(() => 0);
	if (spread === 'Linear') return index.map((i) => i / n);
	// From the other end. Not the same as negating: a reversed four-fixture spread is
	// still four evenly spaced phases, just handed out right to left.
	if (spread === 'Reversed') return index.map((i) => (n - 1 - i) / n);
	// Symmetric about the middle: the ends move together and the centre is opposite,
	// which is what makes a row of front light look like it is breathing outwards.
	if (spread === 'Centre') return index.map((i) => Math.abs((2 * i) / (n - 1) - 1) / 2);

	if (typeof spread === 'object') {
		if ('Wings' in spread) {
			// Mirrored in `w` wings, so a rig split left and right sweeps outwards from
			// the middle of each half rather than across the whole stage.
			const wings = Math.max(1, spread.Wings);
			const per = Math.ceil(n / wings);
			return index.map((i) => {
				const withinWing = i % per;
				const wing = Math.floor(i / per);
				const forward = per <= 1 ? 0 : withinWing / per;
				// Odd wings run backwards, which is what "mirrored" means.
				return wing % 2 === 0 ? forward : (per - withinWing - 1) / per;
			});
		}
		if ('Groups' in spread) {
			// `g` groups, each in step with itself. Every third light together.
			const groups = Math.max(1, spread.Groups);
			return index.map((i) => (i % groups) / groups);
		}
		if ('Random' in spread) {
			const next = mulberry32(spread.Random.seed);
			return index.map(() => next());
		}
	}
	return index.map(() => 0);
}

/**
 * A small deterministic generator, so a random spread is the same every time it is
 * drawn and the same on every console.
 *
 * `Math.random()` would make the spread a fact about this browser at this moment,
 * which is exactly what the phases must not be — the seed is stored and the phases
 * are rebuilt from it.
 */
export function mulberry32(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

/** The spreads the panel offers, in the order it offers them. */
export const SPREADS: { label: string; make: (n: number) => Spread }[] = [
	{ label: 'Together', make: () => 'Even' },
	{ label: 'Chase', make: () => 'Linear' },
	{ label: 'Reversed', make: () => 'Reversed' },
	{ label: 'Centre out', make: () => 'Centre' },
	{ label: 'Wings', make: () => ({ Wings: 2 }) },
	{ label: 'Groups', make: () => ({ Groups: 3 }) },
	{ label: 'Random', make: () => ({ Random: { seed: (Math.random() * 2 ** 32) >>> 0 } }) }
];

/** Which of the above a spread is, for the picker. */
export function spreadLabel(spread: Spread): string {
	if (typeof spread === 'string') {
		return { Even: 'Together', Linear: 'Chase', Reversed: 'Reversed', Centre: 'Centre out' }[spread];
	}
	if ('Wings' in spread) return 'Wings';
	if ('Groups' in spread) return 'Groups';
	return 'Random';
}

// ── Rendering, for the waveform ───────────────────────────────────────────────

/** A shape's level at a cycle position, 0..1. Mirrors `model/effects.rs`. */
export function curveLevel(shape: Shape, width: number, x: number): number {
	switch (shape) {
		case 'Sine':
			return 0.5 + 0.5 * Math.sin(2 * Math.PI * x);
		case 'Triangle':
			return x < 0.5 ? x * 2 : 2 - x * 2;
		case 'Square':
			return x < Math.min(1, Math.max(0, width)) ? 1 : 0;
		case 'SawUp':
			return x;
		case 'SawDown':
			return 1 - x;
		default:
			return 0;
	}
}

/** Where in its cycle an effect is at `nowMs`, 0..1. */
export function cyclePosition(
	rateHz: number,
	backward: boolean,
	phase: number,
	t0: number,
	nowMs: number
): number {
	if (rateHz <= 0) return ((phase % 1) + 1) % 1;
	const cycles = ((nowMs - t0) / 1000) * rateHz;
	const travelled = backward ? -cycles : cycles;
	return (((travelled + phase) % 1) + 1) % 1;
}

/** The value a step list is showing at a cycle position. */
export function stepValue(steps: Step[], x: number): ParameterValue | null {
	if (steps.length === 0) return null;
	const order = [...steps].sort((a, b) => a.at - b.at);
	let current = order.length - 1;
	for (let i = 0; i < order.length; i++) if (x >= order[i].at) current = i;

	const step = order[current];
	const next = order[(current + 1) % order.length];
	if (step.easing === 'Step' || order.length === 1) return step.value;

	let span = next.at - step.at;
	if (span <= 0) span += 1;
	let travelled = x - step.at;
	if (travelled < 0) travelled += 1;
	return blend(step.value, next.value, ease(step.easing, travelled / span));
}

/** The shape of a transition, 0..1 in, 0..1 out. */
export function ease(easing: Step['easing'], t: number): number {
	const clamped = Math.min(1, Math.max(0, t));
	switch (easing) {
		case 'Step':
			return clamped >= 1 ? 1 : 0;
		case 'EaseIn':
			return clamped * clamped;
		case 'EaseOut':
			return clamped * (2 - clamped);
		case 'EaseInOut':
			return clamped < 0.5
				? 2 * clamped * clamped
				: 1 - 2 * (1 - clamped) * (1 - clamped);
		default:
			return clamped;
	}
}

/** Blend two values. Anything without a midpoint turns over at halfway. */
export function blend(low: ParameterValue, high: ParameterValue, level: number): ParameterValue {
	const t = Math.min(1, Math.max(0, level));
	if (low.type === 'Float' && high.type === 'Float') {
		return { type: 'Float', value: low.value + (high.value - low.value) * t };
	}
	if (low.type === 'Int' && high.type === 'Int') {
		return { type: 'Int', value: Math.round(low.value + (high.value - low.value) * t) };
	}
	if (low.type === 'Color' && high.type === 'Color') {
		return {
			type: 'Color',
			value: {
				r: low.value.r + (high.value.r - low.value.r) * t,
				g: low.value.g + (high.value.g - low.value.g) * t,
				b: low.value.b + (high.value.b - low.value.b) * t
			}
		};
	}
	return t >= 0.5 ? high : low;
}

/** What one spec is asserting at `nowMs`, for the dot on the waveform. */
export function valueAt(spec: EffectSpec, rateHz: number, t0: number, nowMs: number): ParameterValue {
	const x = cyclePosition(rateHz, spec.direction === 'Backward', spec.phase, t0, nowMs);
	if ('Steps' in spec.curve) return stepValue(spec.curve.Steps, x) ?? spec.low;
	return blend(spec.low, spec.high, curveLevel(spec.curve.Shape, spec.width, x));
}

// ── Building one ──────────────────────────────────────────────────────────────

/** The level a shape is at, for drawing the waveform itself. */
export function curveAt(curve: Curve, width: number, x: number): number {
	if ('Steps' in curve) {
		// A step list has no single level, so the outline is drawn from where the
		// steps are rather than from a curve: this is the step index, normalised.
		const steps = curve.Steps;
		if (steps.length === 0) return 0;
		const order = [...steps].sort((a, b) => a.at - b.at);
		let current = order.length - 1;
		for (let i = 0; i < order.length; i++) if (x >= order[i].at) current = i;
		return order.length === 1 ? 1 : current / (order.length - 1);
	}
	return curveLevel(curve.Shape, width, x);
}

/**
 * One spec per fixture, sharing an id and differing only in phase.
 *
 * The shared `effect_id` is what lets the panel gather a selection's worth of specs
 * back into one editable effect afterwards, rather than the operator finding six
 * unrelated sines they have to change one at a time.
 */
export function specsFor(
	fixtureIds: string[],
	base: Omit<EffectSpec, 'effect_id' | 'phase'>,
	spread: Spread
): Record<string, EffectSpec> {
	const effectId = crypto.randomUUID();
	const offsets = phases(spread, fixtureIds.length);
	const out: Record<string, EffectSpec> = {};
	fixtureIds.forEach((id, i) => {
		out[id] = { ...base, effect_id: effectId, phase: offsets[i] ?? 0, spread };
	});
	return out;
}

/** Somewhere sensible to start, for whatever kind of parameter this is. */
export function defaultSpec(kind: ParameterKind, fallback: ParameterValue): EffectSpec {
	const numeric = fallback.type === 'Float' || fallback.type === 'Int';
	const colour = fallback.type === 'Color';
	return {
		effect_id: crypto.randomUUID(),
		// A boolean or a string has nothing between two values for a sine to trace, so
		// it starts on the one shape that only ever asks for the ends.
		curve: { Shape: numeric || colour ? 'Sine' : 'Square' },
		rate: { Hz: 0.5 },
		low: zeroLike(fallback),
		high: fullLike(fallback),
		width: 0.5,
		direction: 'Forward',
		phase: 0,
		spread: 'Even',
		// Set on apply, not here: an effect anchored when the panel opened would start
		// part way through its first cycle.
		t0: null
	};
}

/** Whether this kind can be traced at all, and with what. */
export function shapesFor(value: ParameterValue): Shape[] {
	if (value.type === 'Text') return [];
	if (value.type === 'Bool') return ['Square'];
	return ['Sine', 'Triangle', 'Square', 'SawUp', 'SawDown'];
}

export function zeroLike(like: ParameterValue): ParameterValue {
	switch (like.type) {
		case 'Float':
			return { type: 'Float', value: 0 };
		case 'Int':
			return { type: 'Int', value: 0 };
		case 'Color':
			return { type: 'Color', value: { r: 0, g: 0, b: 0 } };
		case 'Bool':
			return { type: 'Bool', value: false };
		default:
			return { type: 'Text', value: '' };
	}
}

export function fullLike(like: ParameterValue): ParameterValue {
	switch (like.type) {
		case 'Float':
			return { type: 'Float', value: 1 };
		case 'Int':
			return { type: 'Int', value: 255 };
		case 'Color':
			return { type: 'Color', value: { r: 1, g: 1, b: 1 } };
		case 'Bool':
			return { type: 'Bool', value: true };
		default:
			return { type: 'Text', value: '' };
	}
}
