import { describe, it, expect } from 'vitest';
import type { Fixture, FixtureType, ParameterValue } from './generated/index.js';
import { bearingFromPoint, elevationFromPoint, rayOnPlane } from './puppeteer.js';

const head: FixtureType = {
	id: 'head',
	name: 'Head',
	manufacturer: 'Generic',
	channel_count: 6,
	parameters: [
		{ kind: 'Pan', direction: 'Output', binding: { Dmx: { channel: 1 } }, default_value: { type: 'Float', value: 0.5 } },
		{ kind: 'Tilt', direction: 'Output', binding: { Dmx: { channel: 2 } }, default_value: { type: 'Float', value: 0.5 } }
	]
};

/** Hung six metres up, facing straight down, pan and tilt centred. */
const hung = (over: Record<string, ParameterValue> = {}): Fixture => ({
	id: 'f',
	name: 'Head',
	fixture_type_id: 'head',
	address: { Dmx: { universe: 1, address: 1 } },
	position: { Axial: { position: { x: 0, y: 6, z: 0 }, direction: { x: 0, y: -1, z: 0 } } },
	live_values: { Pan: { type: 'Float', value: 0.5 }, Tilt: { type: 'Float', value: 0.5 }, ...over },
});

describe('a pointer meeting a plane', () => {
	const down = { origin: { x: 1, y: 10, z: 2 }, direction: { x: 0, y: -1, z: 0 } };

	it('lands where the ray crosses the floor', () => {
		expect(rayOnPlane(down, { x: 0, y: 0, z: 0 }, { x: 0, y: 1, z: 0 })).toEqual({
			x: 1,
			y: 0,
			z: 2
		});
	});

	it('crosses a plane behind the pointer as readily as one in front', () => {
		// The ray is a line, not a half-line: dragging a gizmo past the camera plane
		// should keep tracking rather than stop dead.
		const up = rayOnPlane(down, { x: 0, y: 20, z: 0 }, { x: 0, y: 1, z: 0 });
		expect(up).toEqual({ x: 1, y: 20, z: 2 });
	});

	it('has no answer for a ray running along the plane', () => {
		const along = { origin: { x: 0, y: 3, z: 0 }, direction: { x: 1, y: 0, z: 0 } };
		expect(rayOnPlane(along, { x: 0, y: 0, z: 0 }, { x: 0, y: 1, z: 0 })).toBeNull();
	});
});

describe('reading a bearing off a drag', () => {
	it('measures from downstage, turning towards +X', () => {
		expect(bearingFromPoint(hung(), { x: 0, y: 1, z: 4 })).toBeCloseTo(0, 6);
		expect(bearingFromPoint(hung(), { x: 4, y: 1, z: 0 })).toBeCloseTo(90, 6);
		expect(bearingFromPoint(hung(), { x: 0, y: 1, z: -4 })).toBeCloseTo(180, 6);
	});

	it('ignores how high up the ring was grabbed', () => {
		const low = bearingFromPoint(hung(), { x: 2, y: 0, z: 2 });
		const high = bearingFromPoint(hung(), { x: 2, y: 5, z: 2 });
		expect(low).toBeCloseTo(high!, 9);
	});

	it('has nothing to read from a grab on the axis itself', () => {
		expect(bearingFromPoint(hung(), { x: 0, y: 3, z: 0 })).toBeNull();
	});

	it('has nothing to read for a head that has never been placed', () => {
		expect(bearingFromPoint({ ...hung(), position: null }, { x: 1, y: 0, z: 1 })).toBeNull();
	});

	it('is what a turn is measured against, not where the head points', () => {
		// The same point reads the same bearing whatever the head is currently doing:
		// a gizmo drag is a difference, and a difference needs a fixed reference.
		const swung = hung({ Pan: { type: 'Float', value: 0.9 } });
		expect(bearingFromPoint(swung, { x: 4, y: 1, z: 0 })).toBeCloseTo(90, 6);
	});
});

describe('reading an elevation off a drag', () => {
	it('measures up from level with the fixture', () => {
		// Hung six metres up and pointing straight down, so its own plane is downstage.
		expect(elevationFromPoint(hung(), head, { x: 0, y: 6, z: 4 })).toBeCloseTo(0, 6);
		expect(elevationFromPoint(hung(), head, { x: 0, y: 2, z: 4 })).toBeCloseTo(-45, 6);
		expect(elevationFromPoint(hung(), head, { x: 0, y: 10, z: 4 })).toBeCloseTo(45, 6);
	});

	it('reads a sideways wander as the nod it was meant to be', () => {
		const inPlane = elevationFromPoint(hung(), head, { x: 0, y: 3, z: 3 });
		const wandered = elevationFromPoint(hung(), head, { x: 3, y: 3, z: 3 });
		expect(wandered).toBeCloseTo(inPlane!, 9);
	});

	it('keeps counting the same way round behind the fixture', () => {
		// Level with the head but the other side of it: the reach is negative, so the
		// angle carries on past a right angle rather than mirroring back.
		expect(elevationFromPoint(hung(), head, { x: 0, y: 6, z: -4 })).toBeCloseTo(180, 6);
	});

	it('has nothing to read from a grab on the fixture itself', () => {
		expect(elevationFromPoint(hung(), head, { x: 0, y: 6, z: 0 })).toBeNull();
	});

	it('has nothing to read for a head that has never been placed', () => {
		expect(elevationFromPoint({ ...hung(), position: null }, head, { x: 0, y: 0, z: 4 })).toBeNull();
	});
});
