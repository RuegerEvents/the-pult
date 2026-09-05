/**
 * The drawing model: clipping, and the units.
 *
 * Clipping is here rather than in either renderer, so this is where it is checked. The
 * cases are the ones that were wrong the first time: a line that leaves the frame and
 * comes back, and a filled shape half outside it.
 */

import { describe, expect, it } from 'vitest';

import { clipPolygon, clipPolyline, clipToRect, inside, rectPath } from './drawing.js';

const FRAME = { x: 0, y: 0, w: 10, h: 10 };

describe('clipping a line', () => {
	it('keeps a line that is entirely inside', () => {
		const runs = clipPolyline(
			[
				{ x: 1, y: 1 },
				{ x: 9, y: 9 }
			],
			FRAME
		);
		expect(runs).toHaveLength(1);
		expect(runs[0]).toEqual([
			{ x: 1, y: 1 },
			{ x: 9, y: 9 }
		]);
	});

	it('cuts a line at the frame', () => {
		const runs = clipPolyline(
			[
				{ x: -5, y: 5 },
				{ x: 5, y: 5 }
			],
			FRAME
		);
		expect(runs[0][0].x).toBeCloseTo(0);
		expect(runs[0][1].x).toBeCloseTo(5);
	});

	it('drops a line that never enters', () => {
		expect(
			clipPolyline(
				[
					{ x: -5, y: -5 },
					{ x: -1, y: -1 }
				],
				FRAME
			)
		).toEqual([]);
	});

	it('gives two runs for a line that leaves and comes back', () => {
		// Out of the top and back in. Joining these would draw a straight line across
		// the frame that is not in the rig, which is the whole reason runs exist.
		const runs = clipPolyline(
			[
				{ x: 2, y: 5 },
				{ x: 4, y: -5 },
				{ x: 6, y: 5 }
			],
			FRAME
		);
		expect(runs).toHaveLength(2);
	});
});

describe('clipping a filled shape', () => {
	it('keeps a rectangle half outside as a closed shape', () => {
		const cut = clipPolygon(rectPath({ x: -5, y: 2, w: 10, h: 4 }), FRAME);
		expect(cut.length).toBeGreaterThan(3);
		expect(cut[0]).toEqual(cut[cut.length - 1]);
		for (const p of cut) expect(inside(p, FRAME)).toBe(true);
	});

	it('drops one entirely outside', () => {
		expect(clipPolygon(rectPath({ x: 20, y: 20, w: 4, h: 4 }), FRAME)).toEqual([]);
	});
});

describe('clipping an item', () => {
	it('keeps or drops a label whole, by its anchor', () => {
		const label = {
			kind: 'text' as const,
			at: { x: 5, y: 5 },
			text: 'Spikie 1',
			size: 2
		};
		expect(clipToRect(label, FRAME)).not.toBeNull();
		expect(clipToRect({ ...label, at: { x: 50, y: 5 } }, FRAME)).toBeNull();
	});

	it('drops a path with nothing left of it', () => {
		const path = {
			kind: 'path' as const,
			points: [
				[
					{ x: 30, y: 30 },
					{ x: 40, y: 40 }
				]
			]
		};
		expect(clipToRect(path, FRAME)).toBeNull();
	});
});
