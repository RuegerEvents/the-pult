/**
 * Making one piece meet another.
 *
 * `crates/pult-schema/tests/stock.rs` states the mating rule over the whole catalogue
 * from the station's side; this is the browser's, and it checks the things only this
 * side does — what counts as a *free* joint, and what a `+` handle builds.
 */
import { describe, expect, it } from 'vitest';

import type { SceneObject, Transform, Vec3 } from './generated/index.js';
import { eulerToBasis, IDENTITY } from './scene.js';
import {
	connectorsOf,
	distance,
	freeConnectors,
	placedOnConnector,
	pointToGrid,
	snapConnectors,
	toGrid
} from './snap.js';
import { piece } from './stock.js';

function object(id: string, catalogue: string | null, transform: Transform): SceneObject {
	return {
		id,
		name: id,
		kind: 'Truss',
		transform,
		parent: null,
		layer: null,
		class: null,
		geometry: [],
		symbol: null,
		catalogue,
		properties: null,
		locked: false
	};
}

const at = (x: number, y: number, z: number): Transform => ({
	...IDENTITY,
	position: { x, y, z }
});

describe('the grid', () => {
	it('rounds to half a metre, and leaves a number alone when it is off', () => {
		expect(toGrid(5.9847, 0.5)).toBe(6);
		expect(toGrid(5.9847, 0)).toBe(5.9847);
		expect(pointToGrid({ x: 1.2, y: 0.1, z: -0.4 }, 0.5)).toEqual({ x: 1, y: 0, z: -0.5 });
	});
});

describe('a piece’s joints', () => {
	it('are where the piece actually is', () => {
		const bar = object('a', 'f34-2m', at(3, 6, 0));

		const joints = connectorsOf(bar, bar.transform);

		expect(joints).toHaveLength(2);
		expect(joints.map((j) => j.at.x).sort()).toEqual([2, 4]);
		expect(joints.every((j) => j.at.y === 6)).toBe(true);
	});

	it('are nothing at all on a mesh out of a drawing', () => {
		// A drawing's truss says what it is with its mesh, and inventing joints for it
		// would make a truss somebody measured snap to a place nobody measured.
		expect(connectorsOf(object('a', null, at(0, 0, 0)), at(0, 0, 0))).toHaveLength(0);
	});
});

describe('which joints are free', () => {
	it('is worked out from the geometry, not from a field', () => {
		// Two two-metre sections end to end: four joints, and the two in the middle are
		// bolted together, so a run offers a handle at each end and nowhere else.
		const left = object('l', 'f34-2m', at(-1, 6, 0));
		const right = object('r', 'f34-2m', at(1, 6, 0));
		const all = [
			...connectorsOf(left, left.transform),
			...connectorsOf(right, right.transform)
		];

		const free = freeConnectors(all);

		expect(free).toHaveLength(2);
		expect(free.map((j) => j.at.x).sort((a, b) => a - b)).toEqual([-2, 2]);
	});
});

describe('snapping one piece to another', () => {
	it('bolts a section to the end of a run', () => {
		const fixed = object('a', 'f34-2m', at(0, 6, 0));
		const theirs = freeConnectors(connectorsOf(fixed, fixed.transform));
		// Dragged in roughly at the right-hand end, off by 150 mm.
		const dragged = at(2.15, 6.05, 0.04);
		const moving = object('b', 'f34-2m', dragged);

		const snap = snapConnectors(dragged, connectorsOf(moving, dragged), theirs);

		expect(snap).not.toBeNull();
		// Two metres of truss meeting two metres of truss: the second one's middle is
		// exactly two metres from the first's.
		expect(snap!.transform.position.x).toBeCloseTo(2);
		expect(snap!.transform.position.y).toBeCloseTo(6);
		expect(snap!.transform.position.z).toBeCloseTo(0);
	});

	it('leaves a piece alone when nothing is near it', () => {
		const fixed = object('a', 'f34-2m', at(0, 6, 0));
		const theirs = freeConnectors(connectorsOf(fixed, fixed.transform));
		const away = at(9, 6, 0);

		expect(snapConnectors(away, connectorsOf(object('b', 'f34-2m', away), away), theirs)).toBeNull();
	});

	it('never bolts a deck edge to a truss end', () => {
		const bar = object('a', 'f34-2m', at(0, 0, 0));
		const theirs = freeConnectors(connectorsOf(bar, bar.transform));
		// A deck dragged right on top of the truss's end.
		const deck = at(1.5, 0, 0);

		expect(snapConnectors(deck, connectorsOf(object('d', 'deck-1x1', deck), deck), theirs)).toBeNull();
	});

	it('turns a section end for end when the joint it takes faces the other way', () => {
		// A tower: a base plate on the floor, with its one joint pointing up.
		const base = object('p', 'f34-base', at(0, 0, 0));
		const theirs = freeConnectors(connectorsOf(base, base.transform));
		// Dragged in lying down with one end near the plate, which is how somebody
		// actually does it: the mating is what stands it up.
		const dragged = at(0.95, 0.15, 0.02);
		const moving = object('t', 'f34-2m', dragged);

		const snap = snapConnectors(dragged, connectorsOf(moving, dragged), theirs);

		expect(snap).not.toBeNull();
		// A two-metre section standing on a plate whose joint is 90 mm up: its middle
		// is a metre above that.
		expect(snap!.transform.position.y).toBeCloseTo(1.09);
		// And it is standing up, not lying down: its own X now runs along the world's Y.
		const turned = snap!.transform.rotation;
		expect(Math.abs(turned.x) + Math.abs(turned.y) + Math.abs(turned.z)).toBeGreaterThan(45);
	});
});

describe('turning a piece end for end', () => {
	it('turns it about the vertical, so it comes back the right way up', () => {
		const bar = object('a', 'f34-3m', at(0, 6, 0));
		const end = freeConnectors(connectorsOf(bar, bar.transform)).find((j) => j.at.x < 0)!;

		const placed = placedOnConnector('f34-3m', end)!;

		// Half a turn, and it is a *yaw*. Said as what the rotation does rather than as
		// three angles, because a 180° yaw comes out of XYZ extraction as (180, 0, 180)
		// and the numbers are not the fact being checked: the truss ends up the other
		// way round and still the right way up, so a light clamped to its top chord is
		// still on top.
		const basis = eulerToBasis(placed.rotation);
		const through = (v: Vec3) => ({
			x: basis[0][0] * v.x + basis[0][1] * v.y + basis[0][2] * v.z,
			y: basis[1][0] * v.x + basis[1][1] * v.y + basis[1][2] * v.z,
			z: basis[2][0] * v.x + basis[2][1] * v.y + basis[2][2] * v.z
		});
		const up = through({ x: 0, y: 1, z: 0 });
		const along = through({ x: 1, y: 0, z: 0 });
		expect(up.y).toBeCloseTo(1, 3);
		expect(along.x).toBeCloseTo(-1, 3);
	});
});

describe('the + handle', () => {
	it('lays the next section on the end of the last', () => {
		const bar = object('a', 'f34-3m', at(0, 6, 0));
		const end = freeConnectors(connectorsOf(bar, bar.transform)).find((j) => j.at.x > 0)!;

		const placed = placedOnConnector('f34-3m', end)!;

		expect(placed.position.x).toBeCloseTo(3);
		expect(placed.position.y).toBeCloseTo(6);
	});

	it('puts a corner on whichever face was pressed', () => {
		const bar = object('a', 'f34-3m', at(0, 6, 0));
		const end = freeConnectors(connectorsOf(bar, bar.transform)).find((j) => j.at.x > 0)!;

		const placed = placedOnConnector('f34-corner', end)!;

		// The corner's block is 290 mm, so its middle sits 145 mm past the truss's end.
		expect(distance(placed.position, { x: 1.645, y: 6, z: 0 })).toBeLessThan(1e-3);
	});

	it('says nothing for a piece that has no joint of that kind', () => {
		const bar = object('a', 'pipe-2m', at(0, 6, 0));
		const end = freeConnectors(connectorsOf(bar, bar.transform))[0];

		expect(placedOnConnector('deck-1x1', end)).toBeNull();
		// And the catalogue is what says which pieces do: a pipe end takes a pipe.
		expect(piece('pipe-3m')!.connectors[0].kind).toBe('PipeEnd');
	});
});
