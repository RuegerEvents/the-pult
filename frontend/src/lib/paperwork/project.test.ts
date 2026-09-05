/**
 * Is the drawing to scale?
 *
 * The one question a plan has to be able to answer, asked directly: put two points a
 * known number of metres apart into the projection, and measure how far apart they land
 * in millimetres. Everything else about a viewport is arrangement; this is the claim.
 */

import { describe, expect, it } from 'vitest';

import {
	basisFor,
	extentIn,
	fittedDenominator,
	hull,
	mmPerMetre,
	painted,
	toPage,
	type Camera
} from './project.js';

const RECT = { x: 0, y: 0, w: 100, h: 100 };

function cameraAt(denominator: number, axis: 'plan' | 'front' | 'section' = 'plan'): Camera {
	return {
		basis: basisFor(axis),
		centre: { x: 0, y: 0, z: 0 },
		scale: mmPerMetre(denominator),
		rect: RECT
	};
}

describe('scale', () => {
	it('puts a metre 20 mm apart at 1:50', () => {
		expect(mmPerMetre(50)).toBeCloseTo(20);
		expect(mmPerMetre(100)).toBeCloseTo(10);
		expect(mmPerMetre(35)).toBeCloseTo(28.5714, 3);
	});

	it('holds the same scale everywhere on the page', () => {
		const camera = cameraAt(50);
		// The same one-metre step, at the middle of the frame and near its corner.
		const middle = [
			toPage({ x: 0, y: 0, z: 0 }, camera),
			toPage({ x: 1, y: 0, z: 0 }, camera)
		];
		const corner = [
			toPage({ x: 2, y: 0, z: 2 }, camera),
			toPage({ x: 3, y: 0, z: 2 }, camera)
		];
		expect(middle[1].x - middle[0].x).toBeCloseTo(20);
		expect(corner[1].x - corner[0].x).toBeCloseTo(20);
	});

	it('is the same in both directions', () => {
		const camera = cameraAt(50);
		const across = toPage({ x: 1, y: 0, z: 0 }, camera).x - toPage({ x: 0, y: 0, z: 0 }, camera).x;
		const up = toPage({ x: 0, y: 0, z: 1 }, camera).y - toPage({ x: 0, y: 0, z: 0 }, camera).y;
		expect(Math.abs(across)).toBeCloseTo(Math.abs(up));
	});
});

describe('where each view looks from', () => {
	it('puts upstage at the top of a plan', () => {
		// Upstage is −Z, and the page's y runs downwards, so upstage must have the
		// *smaller* y. A plan drawn the other way up is a plan somebody rigs backwards
		// from.
		const camera = cameraAt(50, 'plan');
		const upstage = toPage({ x: 0, y: 0, z: -3 }, camera);
		const downstage = toPage({ x: 0, y: 0, z: 3 }, camera);
		expect(upstage.y).toBeLessThan(downstage.y);
	});

	it('puts up at the top of a front elevation', () => {
		const camera = cameraAt(50, 'front');
		const high = toPage({ x: 0, y: 6, z: 0 }, camera);
		const low = toPage({ x: 0, y: 0, z: 0 }, camera);
		expect(high.y).toBeLessThan(low.y);
	});

	it('puts the stage on the left of a section', () => {
		// Looking from stage left means downstage (+Z) is to the right of the page.
		const camera = cameraAt(50, 'section');
		const upstage = toPage({ x: 0, y: 0, z: -3 }, camera);
		const downstage = toPage({ x: 0, y: 0, z: 3 }, camera);
		expect(upstage.x).toBeLessThan(downstage.x);
	});
});

describe('fitting', () => {
	it('gives the denominator that just fills the frame', () => {
		// Ten metres across a 100 mm frame is 1:100 before padding.
		const fitted = fittedDenominator({ across: 10, up: 5 }, RECT, 1);
		expect(fitted).toBeCloseTo(100);
	});

	it('is decided by whichever direction is tighter', () => {
		const wide = fittedDenominator({ across: 20, up: 1 }, RECT, 1);
		const tall = fittedDenominator({ across: 1, up: 20 }, RECT, 1);
		expect(wide).toBeCloseTo(tall);
	});

	it('measures the extent in the camera axes, not the world ones', () => {
		// A truss run on the diagonal is wider on a plan than its extent in X.
		const points = [
			{ x: 0, y: 0, z: 0 },
			{ x: 4, y: 0, z: 4 }
		];
		const { across, up } = extentIn(points, basisFor('plan'));
		expect(across).toBeCloseTo(4);
		expect(up).toBeCloseTo(4);
	});

	it('centres on what it is framing', () => {
		const { centre } = extentIn(
			[
				{ x: 2, y: 0, z: 0 },
				{ x: 8, y: 0, z: 0 }
			],
			basisFor('plan')
		);
		expect(centre.x).toBeCloseTo(5);
	});
});

describe('outlines', () => {
	it('hulls a square to its four corners', () => {
		const ring = hull([
			{ x: 0, y: 0 },
			{ x: 1, y: 0 },
			{ x: 1, y: 1 },
			{ x: 0, y: 1 },
			{ x: 0.5, y: 0.5 }
		]);
		// Four corners plus the repeat that closes it. The interior point is gone.
		expect(ring).toHaveLength(5);
		expect(ring[0]).toEqual(ring[ring.length - 1]);
	});
});

describe('painting order', () => {
	it('draws far before near, so what is in front covers what is behind', () => {
		const items = painted([
			{ depth: 5, items: [{ kind: 'path', points: [[{ x: 1, y: 1 }]] }] },
			{ depth: -5, items: [{ kind: 'path', points: [[{ x: 2, y: 2 }]] }] }
		]);
		expect(items[0].points[0][0]).toEqual({ x: 2, y: 2 });
	});

	it('is stable at equal depth', () => {
		const items = painted([
			{ depth: 0, items: [{ kind: 'path', points: [[{ x: 1, y: 1 }]] }] },
			{ depth: 0, items: [{ kind: 'path', points: [[{ x: 2, y: 2 }]] }] }
		]);
		expect(items[0].points[0][0]).toEqual({ x: 1, y: 1 });
	});
});
