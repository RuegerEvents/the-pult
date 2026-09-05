/**
 * Measuring along a bar.
 *
 * The arithmetic here is what a rigger reads off a sheet and repeats back to somebody
 * holding a tape, so the cases worth testing are the ones where a plausible-looking
 * implementation gives a plausible-looking wrong number: the datum, and a bar that is
 * not square to the drawing.
 */

import { describe, expect, it } from 'vitest';

import { barLength, chainFor, unitFor, zeroOf } from './dimensions.js';
import { basisFor, mmPerMetre, type Camera } from './project.js';
import type { Fixture, Vec3 } from '../generated/index.js';
import type { Metrics } from './font.js';

const FLAT: Metrics = {
	widthOf: (text, size) => text.length * size * 0.5,
	capRatio: 0.71,
	ascentRatio: 1.07,
	descentRatio: 0.29,
	bytes: { regular: new Uint8Array(), bold: new Uint8Array() }
};

const camera: Camera = {
	basis: basisFor('plan'),
	centre: { x: 0, y: 0, z: 0 },
	scale: mmPerMetre(50),
	rect: { x: 0, y: 0, w: 200, h: 200 }
};

/** A head at `x` metres along its bar. */
function head(x: number): Fixture {
	return {
		id: `f${x}`,
		name: `Head ${x}`,
		position: {
			position: { x, y: 0, z: 0 },
			rotation: { x: 0, y: 0, z: 0 },
			scale: { x: 1, y: 1, z: 1 }
		}
	} as unknown as Fixture;
}

const SIZE: Vec3 = { x: 12, y: 0.29, z: 0.29 };
/** A bar lying along world X, unturned. */
const square = (local: Vec3) => local;
/** The same bar, turned forty degrees about the vertical. */
const turned = (local: Vec3) => {
	const a = (40 * Math.PI) / 180;
	return { x: local.x * Math.cos(a), y: local.y, z: -local.x * Math.sin(a) };
};

const textsOf = (items: { kind: string }[]) =>
	items.filter((i) => i.kind === 'text').map((i) => (i as unknown as { text: string }).text);

describe('the datum', () => {
	it('measures from the left end', () => {
		expect(zeroOf('Left', 12)).toBe(-6);
	});

	it('measures from the right end', () => {
		expect(zeroOf('Right', 12)).toBe(6);
	});

	it('measures from the centre line', () => {
		expect(zeroOf('Centre', 12)).toBe(0);
	});

	it('reads the length of a bar off its own size', () => {
		expect(barLength(SIZE)).toBe(12);
		expect(barLength(undefined)).toBe(0);
	});
});

describe('how a distance is written', () => {
	it('is millimetres on a short bar, which is what a tape reads', () => {
		const write = unitFor(6);
		expect(write(1.5)).toBe('1500');
		expect(write(0.325)).toBe('325');
	});

	it('is metres on a long one, so nobody counts digits', () => {
		expect(unitFor(14)(12.5)).toBe('12.50 m');
	});

	it('is one unit for the whole chain, decided by its longest figure', () => {
		// Deciding per figure gives "9500, 10.50 m" half way along a bar, and a rigger
		// reading that has to convert mid-chain.
		const write = unitFor(11.5);
		expect(write(0.5)).toBe('0.50 m');
		expect(write(11.5)).toBe('11.50 m');
	});

	it('is never negative: a distance measured backwards is still a distance', () => {
		expect(unitFor(6)(-1.5)).toBe('1500');
	});
});

describe('a chain along a bar', () => {
	const heads = [head(-4), head(-1), head(2)];

	it('prints the gap between each pair', () => {
		const texts = textsOf(
			chainFor(heads, SIZE, square, camera, { datum: 'Left', running: false, above: false }, FLAT)
		);
		// Three metres between each of the two pairs.
		expect(texts.filter((t) => t === '3000')).toHaveLength(2);
	});

	it('prints the distance from the datum as well, when asked', () => {
		const texts = textsOf(
			chainFor(heads, SIZE, square, camera, { datum: 'Left', running: true, above: false }, FLAT)
		);
		// From the left end at −6 m: 2 m, 5 m, 8 m. Millimetres, because the longest
		// figure on this bar is 8 m.
		expect(texts).toContain('2000');
		expect(texts).toContain('5000');
		expect(texts).toContain('8000');
	});

	it('measures from the other end when the crew does', () => {
		const texts = textsOf(
			chainFor(heads, SIZE, square, camera, { datum: 'Right', running: true, above: false }, FLAT)
		);
		// From the right end at +6 m: 10 m, 7 m, 4 m — and the whole chain in metres,
		// because its longest figure is over ten.
		expect(texts).toContain('10.00 m');
		expect(texts).toContain('7.00 m');
		expect(texts).toContain('4.00 m');
	});

	it('measures along the bar, not across the page', () => {
		// The same rig on a truss turned forty degrees. A tape laid along the truss reads
		// the same numbers; a projected measurement would read them shorter by cos 40°,
		// which is the mistake this exists to not make.
		const straight = textsOf(
			chainFor(heads, SIZE, square, camera, { datum: 'Left', running: true, above: false }, FLAT)
		);
		const angled = textsOf(
			chainFor(heads, SIZE, turned, camera, { datum: 'Left', running: true, above: false }, FLAT)
		);
		expect(angled.sort()).toEqual(straight.sort());
	});

	it('draws nothing for a bar seen end-on', () => {
		// A section looking down a truss projects it to a point, and a chain there would
		// be a pile of numbers on one spot. Saying nothing is the honest answer.
		const endOn: Camera = { ...camera, basis: basisFor('section') };
		expect(
			chainFor(heads, SIZE, square, endOn, { datum: 'Left', running: true, above: false }, FLAT)
		).toEqual([]);
	});

	it('draws nothing for a bar with nothing on it', () => {
		expect(
			chainFor([], SIZE, square, camera, { datum: 'Left', running: true, above: false }, FLAT)
		).toEqual([]);
	});

	it('still dimensions a single head, which is the one somebody has to place', () => {
		const texts = textsOf(
			chainFor(
				[head(0)],
				SIZE,
				square,
				camera,
				{ datum: 'Left', running: true, above: false },
				FLAT
			)
		);
		expect(texts).toContain('6000');
	});

	it('never prints a figure upside down', () => {
		// Text past vertical is read by turning the sheet round, and nobody turns an A3
		// round to read one number.
		const items = chainFor(
			heads,
			SIZE,
			(local) => ({ x: -local.x, y: local.y, z: local.z }),
			camera,
			{ datum: 'Left', running: true, above: false },
			FLAT
		);
		for (const item of items) {
			if (item.kind !== 'text') continue;
			const angle = item.rotate ?? 0;
			expect(angle).toBeGreaterThanOrEqual(-90);
			expect(angle).toBeLessThanOrEqual(90);
		}
	});
});
