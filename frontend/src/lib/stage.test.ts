import { describe, it, expect } from 'vitest';
import type { Fixture, FixtureType, ParameterValue, StagePlan } from './generated/index.js';
import {
	HANGING,
	aimAt,
	beamDirection,
	beamSpot,
	calibrationScale,
	fixtureBounds,
	fixtureFacing,
	fixtureOutput,
	fixturePoint,
	fixtureTint,
	originForPixel,
	panAngle,
	pixelToPlan,
	planExtent,
	planToPixel,
	tiltAngle
} from './stage.js';
import { at, facingTransform } from './scene.js';
import { readingOf, NOTHING_YET } from './stores/output.js';
import { aFixture, aFixtureType, aParameter } from './test-fixtures.js';

/**
 * What a fixture is putting out, as a consumer sees it.
 *
 * Everything below takes one of these rather than reading a fixture: nothing stores
 * what a parameter is doing, so a reading is always *for a moment*, and passing one in
 * is how these functions say so.
 */
const showing = (values: Record<string, ParameterValue>) =>
	readingOf(Object.fromEntries(Object.entries(values).map(([key, v]) => [`f/${key}`, v])));

const plan = (over: Partial<StagePlan> = {}): StagePlan => ({
	id: 'p',
	name: 'Ground plan',
	asset: 'abc',
	width_px: 2000,
	height_px: 1000,
	origin: { x: -10, y: 0, z: -5 },
	metres_per_pixel: 0.01,
	rotation_deg: 0,
	opacity: 0.6,
	visible: true,
	...over
});

const fixture = (over: Partial<Fixture> = {}): Fixture =>
	aFixture({ id: 'f', name: 'Front left', fixture_type_id: 't', ...over });

describe('where things are', () => {
	it('reads a point position and an axial one the same way', () => {
		expect(fixturePoint(fixture({ position: at({ x: 1, y: 5, z: 2 }) }))).toEqual({
			x: 1,
			y: 5,
			z: 2
		});
		expect(
			fixturePoint(
				fixture({
					position: facingTransform({ x: 1, y: 5, z: 2 }, { x: 0, y: -1, z: 0 })
				})
			)
		).toEqual({ x: 1, y: 5, z: 2 });
	});

	it('has no position for a fixture that has never been placed', () => {
		expect(fixturePoint(fixture())).toBeNull();
	});

	/// Every placed fixture faces somewhere now: a transform that nobody has turned
	/// hangs straight down, which is what a light on a bar does.
	it('knows a facing for every placed fixture, and none for an unplaced one', () => {
		expect(fixtureFacing(fixture())).toBeNull();
		expect(fixtureFacing(fixture({ position: at({ x: 0, y: 0, z: 0 }) }))).toEqual({
			x: 0,
			y: -1,
			z: 0
		});
		// Close rather than equal: an aim goes to three angles and back, and a right
		// angle through a sine is a right angle plus 1e-16.
		const sideways = fixtureFacing(
			fixture({ position: facingTransform({ x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }) })
		)!;
		expect(sideways.x).toBeCloseTo(1, 6);
		expect(sideways.y).toBeCloseTo(0, 6);
		expect(sideways.z).toBeCloseTo(0, 6);
	});
});

describe('a plan as a map', () => {
	it('puts the top-left pixel on the origin', () => {
		expect(pixelToPlan(plan(), 0, 0)).toEqual({ x: -10, z: -5 });
	});

	it('walks a pixel into metres', () => {
		// 2000 px at 1 cm each is 20 m wide, so the far corner is 20 m across.
		expect(pixelToPlan(plan(), 2000, 1000)).toEqual({ x: 10, z: 5 });
	});

	it('round-trips a point back to the pixel it came from', () => {
		const p = plan({ rotation_deg: 31 });
		const there = pixelToPlan(p, 640, 480);
		const back = planToPixel(p, there);
		expect(back.px).toBeCloseTo(640, 6);
		expect(back.py).toBeCloseTo(480, 6);
	});

	it('turns a rotated plan about its own corner', () => {
		// A quarter turn sends the image's +x along the room's +z.
		const rotated = pixelToPlan(plan({ rotation_deg: 90 }), 100, 0);
		expect(rotated.x).toBeCloseTo(-10, 6);
		expect(rotated.z).toBeCloseTo(-4, 6);
	});

	it('lands the clicked pixel on the show origin, wherever the plan was', () => {
		// The old origin says nothing: only the pixel that was picked does.
		const p = plan({ origin: { x: 37, y: 0, z: -12 } });
		const moved = plan({ origin: { ...originForPixel(p, 400, 250), y: 0 } });
		expect(pixelToPlan(moved, 400, 250).x).toBeCloseTo(0, 9);
		expect(pixelToPlan(moved, 400, 250).z).toBeCloseTo(0, 9);
	});

	it('takes the plan rotation into account', () => {
		const p = plan({ rotation_deg: 31, origin: { x: -3, y: 0, z: 8 } });
		const moved = plan({
			rotation_deg: 31,
			origin: { ...originForPixel(p, 640, 480), y: 0 }
		});
		expect(pixelToPlan(moved, 640, 480).x).toBeCloseTo(0, 9);
		expect(pixelToPlan(moved, 640, 480).z).toBeCloseTo(0, 9);
	});

	it('says how much room a plan covers', () => {
		expect(planExtent(plan())).toEqual({ width: 20, depth: 10 });
	});
});

describe('calibration', () => {
	it('derives the scale from a known distance', () => {
		// 800 px measured across something 8 m wide is 1 cm a pixel.
		expect(calibrationScale({ px: 100, py: 0 }, { px: 900, py: 0 }, 8)).toBeCloseTo(0.01, 9);
	});

	it('measures diagonally too', () => {
		expect(calibrationScale({ px: 0, py: 0 }, { px: 300, py: 400 }, 10)).toBeCloseTo(0.02, 9);
	});

	it('refuses two points too close together to mean anything', () => {
		expect(calibrationScale({ px: 10, py: 10 }, { px: 10, py: 10 }, 5)).toBeNull();
	});

	it('refuses a distance that is not one', () => {
		expect(calibrationScale({ px: 0, py: 0 }, { px: 100, py: 0 }, 0)).toBeNull();
	});
});

describe('bounds', () => {
	it('falls back to a room-sized box when nothing is placed', () => {
		expect(fixtureBounds([fixture()])).toEqual({ minX: -8, maxX: 8, minZ: -6, maxZ: 6 });
	});

	it('holds every placed fixture with room to spare', () => {
		const rig = [
			fixture({ position: at({ x: -3, y: 4, z: 0 }) }),
			fixture({ position: at({ x: 5, y: 4, z: 7 }) }),
			fixture()
		];
		expect(fixtureBounds(rig, 1)).toEqual({ minX: -4, maxX: 6, minZ: -1, maxZ: 8 });
	});
});

describe('what things are doing', () => {
	it('is dark when nothing can be said', () => {
		expect(fixtureOutput(fixture(), showing({})).level).toBe(0);
		expect(fixtureTint(fixture(), showing({}))).toBe('rgb(0, 0, 0)');
	});

	/// The rule that keeps a wrong picture off the screen: a browser that cannot yet
	/// place itself on the station's clock draws nothing rather than a guess.
	it('is dark when this browser does not know what time it is', () => {
		expect(fixtureOutput(fixture(), NOTHING_YET).level).toBe(0);
		expect(NOTHING_YET.at).toBeNull();
	});

	it('dims a colour by its own intensity', () => {
		const lit = showing({
			Intensity: { type: 'Float', value: 0.5 },
			ColorRgb: { type: 'Color', value: { r: 1, g: 0, b: 0, overrides: {} } }
		});
		expect(fixtureTint(fixture(), lit)).toBe('rgb(128, 0, 0)');
	});

	it('takes a colour-only fixture at its word', () => {
		// No dimmer channel: reporting it dark because there is no Intensity would
		// be a lie about a fixture that is plainly on.
		const lit = showing({ ColorRgb: { type: 'Color', value: { r: 0, g: 1, b: 0, overrides: {} } } });
		expect(fixtureOutput(fixture(), lit).level).toBe(1);
		expect(fixtureTint(fixture(), lit)).toBe('rgb(0, 255, 0)');
	});

	it('reads a closed relay as full', () => {
		const on = showing({ 'Switch:0': { type: 'Bool', value: true } });
		expect(fixtureOutput(fixture(), on).level).toBe(1);
		const off = showing({ 'Switch:0': { type: 'Bool', value: false } });
		expect(fixtureOutput(fixture(), off).level).toBe(0);
	});

	it('clamps a level that arrives out of range', () => {
		const over = showing({ Intensity: { type: 'Float', value: 1.4 } });
		expect(fixtureOutput(fixture(), over).level).toBe(1);
	});
});

describe('pointing', () => {
	const mover: FixtureType = aFixtureType({
		id: 't',
		name: 'Mover',
		manufacturer: 'Generic',
		channel_count: 4,
		parameters: [
			aParameter({ kind: 'Pan', default_value: { type: 'Float', value: 0.5 } })
		]
	});
	const dimmer: FixtureType = { ...mover, parameters: [] };

	const panned = (at: number) => showing({ Pan: { type: 'Float', value: at } });

	it('has no angle for a fixture that cannot move', () => {
		expect(panAngle(fixture(), dimmer, panned(0.5))).toBeNull();
	});

	it('has no angle where nothing can say where a mover is', () => {
		expect(panAngle(fixture(), mover, showing({}))).toBeNull();
	});

	it('points a centred mover the way it hangs', () => {
		const facing = fixture({
			position: facingTransform({ x: 0, y: 5, z: 0 }, { x: 0, y: -1, z: 1 })
		});
		expect(panAngle(facing, mover, panned(0.5))).toBeCloseTo(0, 6);
	});

	it('swings either side of centre', () => {
		expect(panAngle(fixture(), mover, panned(1))).toBeCloseTo(270, 6);
		expect(panAngle(fixture(), mover, panned(0))).toBeCloseTo(-270, 6);
	});

	const head: FixtureType = aFixtureType({
		...mover,
		parameters: [
			...mover.parameters,
			aParameter({ kind: 'Tilt', default_value: { type: 'Float', value: 0.5 } })
		]
	});

	/** Hung facing straight down, which is how a head on a bar hangs. */
	const hung = () =>
		fixture({
			position: facingTransform({ x: 0, y: 6, z: 0 }, { x: 0, y: -1, z: 0 })
		});
	/** Where it is showing, pan and tilt centred unless said otherwise. */
	const centred = (over: Record<string, ParameterValue> = {}) =>
		showing({
			Pan: { type: 'Float', value: 0.5 },
			Tilt: { type: 'Float', value: 0.5 },
			...over
		});

	it('nods either side of the elevation it was hung at', () => {
		expect(tiltAngle(hung(), head, centred())).toBeCloseTo(-90, 6);
		expect(
			tiltAngle(hung(), head, centred({ Tilt: { type: 'Float', value: 1 } }))
		).toBeCloseTo(45, 6);
	});

	it('has no tilt for a head that cannot nod', () => {
		expect(tiltAngle(hung(), mover, centred())).toBeNull();
	});

	it('points a centred head the way it hangs', () => {
		const beam = beamDirection(hung(), head, centred());
		expect(beam.x).toBeCloseTo(0, 6);
		expect(beam.y).toBeCloseTo(-1, 6);
		expect(beam.z).toBeCloseTo(0, 6);
	});

	it('tilts up through the horizontal', () => {
		// Two thirds of the travel up from straight down is exactly level.
		const level = centred({ Tilt: { type: 'Float', value: 0.5 + 90 / 270 } });
		const beam = beamDirection(hung(), head, level);
		expect(beam.y).toBeCloseTo(0, 6);
		expect(beam.z).toBeCloseTo(1, 6);
	});

	it('pans a tilted beam round the vertical', () => {
		// Level, then a quarter turn: 90° of 540 is a sixth of the travel.
		const swung = centred({
			Pan: { type: 'Float', value: 0.5 + 90 / 540 },
			Tilt: { type: 'Float', value: 0.5 + 90 / 270 }
		});
		const beam = beamDirection(hung(), head, swung);
		expect(beam.x).toBeCloseTo(1, 6);
		expect(beam.z).toBeCloseTo(0, 6);
	});

	it('leaves a pan-only head where tilt would have put it', () => {
		// The behaviour before tilt existed: swinging keeps the hung elevation.
		const rigged = fixture({
			position: facingTransform({ x: 0, y: 5, z: 0 }, { x: 0, y: -3, z: 4 })
		});
		const beam = beamDirection(rigged, mover, showing({ Pan: { type: 'Float', value: 0.5 } }));
		expect(beam.y).toBeCloseTo(-0.6, 6);
		expect(beam.z).toBeCloseTo(0.8, 6);
	});
});

describe('aiming a head', () => {
	const head: FixtureType = aFixtureType({
		id: 't',
		name: 'Head',
		manufacturer: 'Generic',
		channel_count: 4,
		parameters: [
			aParameter({ kind: 'Pan', default_value: { type: 'Float', value: 0.5 } }),
			aParameter({ kind: 'Tilt', default_value: { type: 'Float', value: 0.5 } })
		]
	});
	const hung = () =>
		fixture({
			position: facingTransform({ x: 0, y: 6, z: 0 }, { x: 0, y: -1, z: 0 })
		});
	const centred = (over: Record<string, ParameterValue> = {}) =>
		showing({
			Pan: { type: 'Float', value: 0.5 },
			Tilt: { type: 'Float', value: 0.5 },
			...over
		});

	it('lands the beam where it was asked to', () => {
		const target = { x: 3, y: 0, z: 2 };
		const { pan, tilt } = aimAt(hung(), head, target);
		const aimed = centred({
			Pan: { type: 'Float', value: pan! },
			Tilt: { type: 'Float', value: tilt! }
		});
		const spot = beamSpot(hung(), head, aimed)!;
		expect(spot.x).toBeCloseTo(target.x, 4);
		expect(spot.z).toBeCloseTo(target.z, 4);
	});

	it('leaves a head aimed at its own feet where it hangs', () => {
		const { pan, tilt } = aimAt(hung(), head, { x: 0, y: 0, z: 0 });
		expect(tilt).toBeCloseTo(0.5, 6);
		expect(pan).toBeCloseTo(0.5, 6);
	});

	it('says nothing about an axis the head does not have', () => {
		const panOnly: FixtureType = { ...head, parameters: [head.parameters[0]] };
		expect(aimAt(hung(), panOnly, { x: 1, y: 0, z: 1 }).tilt).toBeNull();
	});

	it('takes the short way round rather than the long one', () => {
		// Directly behind: a bearing of 180° must not come back as −180 / 540 off scale.
		const { pan } = aimAt(hung(), head, { x: 0, y: 0, z: -4 });
		expect(pan).toBeGreaterThanOrEqual(0);
		expect(pan).toBeLessThanOrEqual(1);
	});

	it('gives the closest it can manage rather than something unusable', () => {
		// 270° of travel cannot be asked for more than 135° either way.
		const { tilt } = aimAt(hung(), head, { x: 0, y: 100, z: 0 });
		expect(tilt).toBe(1);
	});

	it('keeps the beam spot within reach of the plan', () => {
		// Tilted a long way past the horizontal, so the beam never meets the floor and
		// the honest landing point is tens of metres away — off any plan anyone is
		// looking at, and out from under the pointer trying to drag it.
		const flat = centred({ Tilt: { type: 'Float', value: 1 } });
		const far = beamSpot(hung(), head, flat)!;
		expect(Math.hypot(far.x, far.z)).toBeGreaterThan(6);

		const near = beamSpot(hung(), head, flat, { maxThrow: 6 })!;
		expect(Math.hypot(near.x - 0, near.y - 6, near.z - 0)).toBeCloseTo(6, 6);
	});

	it('leaves a beam that lands well inside the cap alone', () => {
		const down = beamSpot(hung(), head, centred(), { maxThrow: 40 })!;
		expect(down.x).toBeCloseTo(0, 6);
		expect(down.z).toBeCloseTo(0, 6);
		expect(down.y).toBeCloseTo(0, 6);
	});

	it('has nothing to aim for a fixture that has never been placed', () => {
		expect(aimAt(fixture(), head, { x: 0, y: 0, z: 0 })).toEqual({ pan: null, tilt: null });
		expect(beamSpot(fixture(), head, centred())).toBeNull();
	});
});

describe('guessing a scale', () => {
	it('makes a new plan about a stage wide', async () => {
		const { guessScale } = await import('./components/stage/upload.js');
		expect(guessScale(2400) * 2400).toBeCloseTo(12, 6);
	});

	it('does not divide by zero for a plan with no width', async () => {
		const { guessScale } = await import('./components/stage/upload.js');
		expect(Number.isFinite(guessScale(0))).toBe(true);
	});
});

describe('the rig in three dimensions', () => {
	it('points a fixture with no stated direction at the floor', async () => {
		const { beamDirection } = await import('./stage.js');
		expect(
			beamDirection(fixture({ position: at({ x: 0, y: 5, z: 0 }) }), undefined, showing({}))
		).toEqual({
			x: 0,
			y: -1,
			z: 0
		});
	});

	it('follows the direction a fixture was hung at', async () => {
		const { beamDirection } = await import('./stage.js');
		const hung = fixture({
			position: facingTransform({ x: 0, y: 5, z: 4 }, { x: 0, y: -3, z: -4 })
		});
		const beam = beamDirection(hung, undefined, showing({}));
		expect(Math.hypot(beam.x, beam.y, beam.z)).toBeCloseTo(1, 6);
		expect(beam.y).toBeCloseTo(-0.6, 6);
		expect(beam.z).toBeCloseTo(-0.8, 6);
	});

	it('measures the throw down to the floor', async () => {
		const { throwDistance } = await import('./stage.js');
		expect(throwDistance({ x: 0, y: 6, z: 0 }, { x: 0, y: -1, z: 0 })).toBeCloseTo(6, 6);
		// Straight down from 6 m at 45° is 6√2.
		const diagonal = Math.SQRT1_2;
		expect(throwDistance({ x: 0, y: 6, z: 0 }, { x: diagonal, y: -diagonal, z: 0 })).toBeCloseTo(
			6 * Math.SQRT2,
			5
		);
	});

	it('draws a slanted beam on until its whole end ring is under the floor', async () => {
		const { drawnLength } = await import('./stage.js');
		const lens = 0.1;
		const spread = Math.tan((7 * Math.PI) / 180);
		// Straight down, the square cut is level with the floor and nothing is added.
		expect(drawnLength(6, { x: 0, y: -1, z: 0 }, spread, lens)).toBeCloseTo(6, 6);
		// At an angle, the end ring's highest point must be at or under the floor —
		// checked against the geometry rather than against a number.
		for (const degrees of [15, 45, 70]) {
			const theta = (degrees * Math.PI) / 180;
			const direction = { x: Math.sin(theta), y: -Math.cos(theta), z: 0 };
			const height = 6;
			const throwToFloor = height / Math.cos(theta);
			const drawn = drawnLength(throwToFloor, direction, spread, lens);
			expect(drawn).toBeGreaterThan(throwToFloor);
			const radius = lens + drawn * spread;
			const centreY = height + drawn * direction.y;
			const highest = centreY + radius * Math.sin(theta);
			expect(highest).toBeLessThanOrEqual(1e-6);
			// And only just: the beam is not run on further than that needs.
			expect(highest).toBeGreaterThan(-1e-3);
		}
		// A beam pointing at the sky has no floor to be cut by and is left alone.
		expect(drawnLength(12, { x: 0, y: 1, z: 0 }, spread, lens)).toBe(12);
	});

	it('gives a beam pointing at the sky a length rather than an infinity', async () => {
		const { throwDistance } = await import('./stage.js');
		expect(Number.isFinite(throwDistance({ x: 0, y: 2, z: 0 }, { x: 0, y: 1, z: 0 }))).toBe(true);
	});
});

