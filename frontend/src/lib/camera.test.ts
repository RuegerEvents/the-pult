import { describe, expect, it } from 'vitest';

import type { Fixture, SceneObject } from './generated/index.js';
import { at } from './scene.js';
import { fitDistance, focusShot, presetShot, rigBounds } from './camera.js';

const fixture = (x: number, y: number, z: number): Fixture =>
	({
		id: `${x}/${y}/${z}`,
		position: at({ x, y, z }),
		parent: null
	}) as unknown as Fixture;

/** A rig eight metres wide, hung at four, three metres deep. */
const rig = [fixture(-4, 4, -1), fixture(4, 4, 2)];
const nothing = new Map<string, SceneObject>();

describe('the box a view has to frame', () => {
	it('holds every fixture, with room to breathe', () => {
		const bounds = rigBounds(rig, nothing);
		expect(bounds.min.x).toBeCloseTo(-5, 6);
		expect(bounds.max.x).toBeCloseTo(5, 6);
		expect(bounds.max.z).toBeCloseTo(3, 6);
	});

	it('reaches the floor under a rig that is all in the air', () => {
		// Otherwise a front view of a rig hung at four metres is a picture of a bar,
		// with the deck it is lighting outside the frame entirely.
		expect(rigBounds(rig, nothing).min.y).toBeLessThanOrEqual(0);
	});

	it('leaves the floor out when it is framing a selection', () => {
		// An operator who picked one head at four metres wants that head, not four
		// metres of air under it.
		const one = rigBounds([fixture(3, 4, 0)], nothing, { margin: 0.6, toFloor: false });
		expect(one.min.y).toBeCloseTo(3.4, 6);
	});

	it('frames a stage-sized room when there is nothing in it', () => {
		const bounds = rigBounds([], nothing);
		expect(bounds.max.x).toBeGreaterThan(bounds.min.x);
	});

	it('leaves out a piece of the drawing that is not being drawn', () => {
		// A hidden layer takes its trusses out of the view, and framing one nobody can
		// see is framing empty air. The map itself still holds them, because a light
		// hangs where its truss is whether or not the truss is drawn.
		const truss = {
			id: 't',
			transform: at({ x: 40, y: 6, z: 0 }),
			parent: null
		} as unknown as SceneObject;
		const objects = new Map([['t', truss]]);
		expect(rigBounds(rig, objects).max.x).toBeCloseTo(41, 6);
		expect(rigBounds(rig, objects, { pieces: [] }).max.x).toBeCloseTo(5, 6);
	});
});

describe('how far back a camera has to stand', () => {
	it('takes whichever of the two lens angles needs more room', () => {
		// A wide rig on a portrait tile is framed by its width; the same rig on a wide
		// monitor is not. A fit that only ever asked the vertical would cut it off.
		const wide = fitDistance(20, 3, 0.5);
		const same = fitDistance(20, 3, 2.5);
		expect(wide).toBeGreaterThan(same);
	});

	it('stands further back for a bigger rig', () => {
		expect(fitDistance(30, 4, 1.6)).toBeGreaterThan(fitDistance(6, 4, 1.6));
	});
});

describe('the four places to stand', () => {
	const bounds = rigBounds(rig, nothing);

	it('puts the front view out in the house at eye height', () => {
		const shot = presetShot('front', bounds, 1.6);
		expect(shot.position[1]).toBeCloseTo(1.7, 6);
		expect(shot.position[2]).toBeGreaterThan(bounds.max.z);
		expect(shot.position[0]).toBeCloseTo(0, 6);
	});

	it('puts the plan overhead, and not quite exactly overhead', () => {
		const shot = presetShot('plan', bounds, 1.6);
		expect(shot.position[1]).toBeGreaterThan(bounds.max.y);
		// A camera looking straight down has nothing to resolve its own roll, so the
		// view rolls to whatever the maths picks. A couple of degrees off does not.
		expect(shot.position[2]).not.toBeCloseTo(shot.target[2], 6);
	});

	it('takes the section from the side the stage is drawn from', () => {
		// Stage left, so the stage is on the left of the frame and the house on the
		// right, which is how a section is drawn.
		const shot = presetShot('section', bounds, 1.6);
		expect(shot.position[0]).toBeLessThan(bounds.min.x);
		expect(shot.position[1]).toBeCloseTo(shot.target[1], 6);
	});

	it('puts the three-quarter above the rig and off to one side', () => {
		const shot = presetShot('quarter', bounds, 1.6);
		expect(shot.position[0]).toBeGreaterThan(0);
		expect(shot.position[2]).toBeGreaterThan(0);
		expect(shot.position[1]).toBeGreaterThan(bounds.max.y / 2);
	});

	it('frames a festival from further away than a demo', () => {
		const festival = rigBounds([fixture(-30, 8, -8), fixture(30, 8, 8)], nothing);
		const near = presetShot('front', bounds, 1.6);
		const far = presetShot('front', festival, 1.6);
		expect(far.position[2]).toBeGreaterThan(near.position[2]);
	});
});

describe('focusing on what is selected', () => {
	const one = rigBounds([fixture(3, 5, -2)], nothing, { margin: 0.6, toFloor: false });

	it('keeps the direction the operator was already looking from', () => {
		const from = { x: 3, y: 5, z: 20 };
		const shot = focusShot(one, from, 1.6);
		expect(shot.target).toEqual([3, expect.closeTo(5, 6), -2]);
		// Still downstage of it, which is where the camera was.
		expect(shot.position[2]).toBeGreaterThan(shot.target[2]);
		expect(shot.position[0]).toBeCloseTo(3, 6);
	});

	it('has somewhere to stand even from inside the thing it is framing', () => {
		const shot = focusShot(one, { x: 3, y: 2.5, z: -2 }, 1.6);
		expect(shot.position.every(Number.isFinite)).toBe(true);
	});
});
