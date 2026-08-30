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
	joinPosition,
	originForPixel,
	panAngle,
	pixelToPlan,
	planExtent,
	planToPixel,
	splitPosition,
	tiltAngle
} from './stage.js';

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

const fixture = (over: Partial<Fixture> = {}): Fixture => ({
	id: 'f',
	name: 'Front left',
	fixture_type_id: 't',
	address: { Dmx: { universe: 1, address: 1 } },
	position: null,
	live_values: {},
	live_effects: {},
	live_fades: {},
	...over
});

describe('where things are', () => {
	it('reads a point position and an axial one the same way', () => {
		expect(fixturePoint(fixture({ position: { Point: { x: 1, y: 5, z: 2 } } }))).toEqual({
			x: 1,
			y: 5,
			z: 2
		});
		expect(
			fixturePoint(
				fixture({
					position: { Axial: { position: { x: 1, y: 5, z: 2 }, direction: { x: 0, y: -1, z: 0 } } }
				})
			)
		).toEqual({ x: 1, y: 5, z: 2 });
	});

	it('has no position for a fixture that has never been placed', () => {
		expect(fixturePoint(fixture())).toBeNull();
	});

	it('only knows a facing for a fixture placed axially', () => {
		expect(fixtureFacing(fixture({ position: { Point: { x: 0, y: 0, z: 0 } } }))).toBeNull();
		expect(
			fixtureFacing(
				fixture({
					position: { Axial: { position: { x: 0, y: 0, z: 0 }, direction: { x: 0, y: -1, z: 0 } } }
				})
			)
		).toEqual({ x: 0, y: -1, z: 0 });
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
			fixture({ position: { Point: { x: -3, y: 4, z: 0 } } }),
			fixture({ position: { Point: { x: 5, y: 4, z: 7 } } }),
			fixture()
		];
		expect(fixtureBounds(rig, 1)).toEqual({ minX: -4, maxX: 6, minZ: -1, maxZ: 8 });
	});
});

describe('what things are doing', () => {
	it('is dark when nothing is reported', () => {
		expect(fixtureOutput(fixture()).level).toBe(0);
		expect(fixtureTint(fixture())).toBe('rgb(0, 0, 0)');
	});

	it('dims a colour by its own intensity', () => {
		const lit = fixture({
			live_values: {
				Intensity: { type: 'Float', value: 0.5 },
				ColorRgb: { type: 'Color', value: { r: 1, g: 0, b: 0 } }
			}
		});
		expect(fixtureTint(lit)).toBe('rgb(128, 0, 0)');
	});

	it('takes a colour-only fixture at its word', () => {
		// No dimmer channel: reporting it dark because there is no Intensity would
		// be a lie about a fixture that is plainly on.
		const lit = fixture({
			live_values: { ColorRgb: { type: 'Color', value: { r: 0, g: 1, b: 0 } } },
			live_effects: {},
			live_fades: {}
		});
		expect(fixtureOutput(lit).level).toBe(1);
		expect(fixtureTint(lit)).toBe('rgb(0, 255, 0)');
	});

	it('reads a closed relay as full', () => {
		const on = fixture({ live_values: { 'Switch:0': { type: 'Bool', value: true } } });
		expect(fixtureOutput(on).level).toBe(1);
		const off = fixture({ live_values: { 'Switch:0': { type: 'Bool', value: false } } });
		expect(fixtureOutput(off).level).toBe(0);
	});

	it('clamps a level that arrives out of range', () => {
		const over = fixture({ live_values: { Intensity: { type: 'Float', value: 1.4 } } });
		expect(fixtureOutput(over).level).toBe(1);
	});
});

describe('pointing', () => {
	const mover: FixtureType = {
		id: 't',
		name: 'Mover',
		manufacturer: 'Generic',
		channel_count: 4,
		parameters: [
			{ kind: 'Pan', direction: 'Output', binding: { Dmx: { channel: 1 } }, default_value: { type: 'Float', value: 0.5 } }
		]
	};
	const dimmer: FixtureType = { ...mover, parameters: [] };

	it('has no angle for a fixture that cannot move', () => {
		expect(panAngle(fixture({ live_values: { Pan: { type: 'Float', value: 0.5 } } }), dimmer)).toBeNull();
	});

	it('has no angle before a mover has reported one', () => {
		expect(panAngle(fixture(), mover)).toBeNull();
	});

	it('points a centred mover the way it hangs', () => {
		const facing = fixture({
			position: { Axial: { position: { x: 0, y: 5, z: 0 }, direction: { x: 0, y: -1, z: 1 } } },
			live_values: { Pan: { type: 'Float', value: 0.5 } },
			live_effects: {},
			live_fades: {}
		});
		expect(panAngle(facing, mover)).toBeCloseTo(0, 6);
	});

	it('swings either side of centre', () => {
		const swung = fixture({ live_values: { Pan: { type: 'Float', value: 1 } } });
		expect(panAngle(swung, mover)).toBeCloseTo(270, 6);
		const other = fixture({ live_values: { Pan: { type: 'Float', value: 0 } } });
		expect(panAngle(other, mover)).toBeCloseTo(-270, 6);
	});

	const head: FixtureType = {
		...mover,
		parameters: [
			...mover.parameters,
			{ kind: 'Tilt', direction: 'Output', binding: { Dmx: { channel: 2 } }, default_value: { type: 'Float', value: 0.5 } }
		]
	};

	/** Hung facing straight down, which is how a head on a bar hangs. */
	const hung = (over: Record<string, ParameterValue> = {}) =>
		fixture({
			position: { Axial: { position: { x: 0, y: 6, z: 0 }, direction: { x: 0, y: -1, z: 0 } } },
			live_values: { Pan: { type: 'Float', value: 0.5 }, Tilt: { type: 'Float', value: 0.5 }, ...over },
			live_effects: {},
			live_fades: {}
		});

	it('nods either side of the elevation it was hung at', () => {
		expect(tiltAngle(hung(), head)).toBeCloseTo(-90, 6);
		expect(tiltAngle(hung({ Tilt: { type: 'Float', value: 1 } }), head)).toBeCloseTo(45, 6);
	});

	it('has no tilt for a head that cannot nod', () => {
		expect(tiltAngle(hung(), mover)).toBeNull();
	});

	it('points a centred head the way it hangs', () => {
		const beam = beamDirection(hung(), head);
		expect(beam.x).toBeCloseTo(0, 6);
		expect(beam.y).toBeCloseTo(-1, 6);
		expect(beam.z).toBeCloseTo(0, 6);
	});

	it('tilts up through the horizontal', () => {
		// Two thirds of the travel up from straight down is exactly level.
		const level = hung({ Tilt: { type: 'Float', value: 0.5 + 90 / 270 } });
		const beam = beamDirection(level, head);
		expect(beam.y).toBeCloseTo(0, 6);
		expect(beam.z).toBeCloseTo(1, 6);
	});

	it('pans a tilted beam round the vertical', () => {
		// Level, then a quarter turn: 90° of 540 is a sixth of the travel.
		const swung = hung({
			Pan: { type: 'Float', value: 0.5 + 90 / 540 },
			Tilt: { type: 'Float', value: 0.5 + 90 / 270 }
		});
		const beam = beamDirection(swung, head);
		expect(beam.x).toBeCloseTo(1, 6);
		expect(beam.z).toBeCloseTo(0, 6);
	});

	it('leaves a pan-only head where tilt would have put it', () => {
		// The behaviour before tilt existed: swinging keeps the hung elevation.
		const rigged = fixture({
			position: { Axial: { position: { x: 0, y: 5, z: 0 }, direction: { x: 0, y: -3, z: 4 } } },
			live_values: { Pan: { type: 'Float', value: 0.5 } },
			live_effects: {},
			live_fades: {}
		});
		const beam = beamDirection(rigged, mover);
		expect(beam.y).toBeCloseTo(-0.6, 6);
		expect(beam.z).toBeCloseTo(0.8, 6);
	});
});

describe('aiming a head', () => {
	const head: FixtureType = {
		id: 't',
		name: 'Head',
		manufacturer: 'Generic',
		channel_count: 4,
		parameters: [
			{ kind: 'Pan', direction: 'Output', binding: { Dmx: { channel: 1 } }, default_value: { type: 'Float', value: 0.5 } },
			{ kind: 'Tilt', direction: 'Output', binding: { Dmx: { channel: 2 } }, default_value: { type: 'Float', value: 0.5 } }
		]
	};
	const hung = (over: Record<string, ParameterValue> = {}) =>
		fixture({
			position: { Axial: { position: { x: 0, y: 6, z: 0 }, direction: { x: 0, y: -1, z: 0 } } },
			live_values: { Pan: { type: 'Float', value: 0.5 }, Tilt: { type: 'Float', value: 0.5 }, ...over },
			live_effects: {},
			live_fades: {}
		});

	it('lands the beam where it was asked to', () => {
		const target = { x: 3, y: 0, z: 2 };
		const { pan, tilt } = aimAt(hung(), head, target);
		const aimed = hung({
			Pan: { type: 'Float', value: pan! },
			Tilt: { type: 'Float', value: tilt! }
		});
		const spot = beamSpot(aimed, head)!;
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
		const flat = hung({ Tilt: { type: 'Float', value: 1 } });
		const far = beamSpot(flat, head)!;
		expect(Math.hypot(far.x, far.z)).toBeGreaterThan(6);

		const near = beamSpot(flat, head, { maxThrow: 6 })!;
		expect(Math.hypot(near.x - 0, near.y - 6, near.z - 0)).toBeCloseTo(6, 6);
	});

	it('leaves a beam that lands well inside the cap alone', () => {
		const down = beamSpot(hung(), head, { maxThrow: 40 })!;
		expect(down.x).toBeCloseTo(0, 6);
		expect(down.z).toBeCloseTo(0, 6);
		expect(down.y).toBeCloseTo(0, 6);
	});

	it('has nothing to aim for a fixture that has never been placed', () => {
		expect(aimAt(fixture(), head, { x: 0, y: 0, z: 0 })).toEqual({ pan: null, tilt: null });
		expect(beamSpot(fixture(), head)).toBeNull();
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
	it('opens at front of house looking at the stage', async () => {
		const { fohCamera } = await import('./stage.js');
		const rig = [
			fixture({ position: { Point: { x: -4, y: 5, z: -2 } } }),
			fixture({ position: { Point: { x: 4, y: 5, z: 1 } } })
		];
		const camera = fohCamera(rig);
		expect(camera.position[1]).toBeCloseTo(1.7, 6);
		// Downstage of everything, out in the house.
		expect(camera.position[2]).toBeGreaterThan(1);
		expect(camera.position[0]).toBeCloseTo(0, 6);
	});

	it('stands further back for a wider rig', async () => {
		const { fohCamera } = await import('./stage.js');
		const near = fohCamera([fixture({ position: { Point: { x: 0, y: 4, z: 0 } } })]);
		const wide = fohCamera([
			fixture({ position: { Point: { x: -20, y: 4, z: 0 } } }),
			fixture({ position: { Point: { x: 20, y: 4, z: 0 } } })
		]);
		expect(wide.position[2]).toBeGreaterThan(near.position[2]);
	});

	it('points a fixture with no stated direction at the floor', async () => {
		const { beamDirection } = await import('./stage.js');
		expect(beamDirection(fixture({ position: { Point: { x: 0, y: 5, z: 0 } } }), undefined)).toEqual({
			x: 0,
			y: -1,
			z: 0
		});
	});

	it('follows the direction a fixture was hung at', async () => {
		const { beamDirection } = await import('./stage.js');
		const hung = fixture({
			position: { Axial: { position: { x: 0, y: 5, z: 4 }, direction: { x: 0, y: -3, z: -4 } } }
		});
		const beam = beamDirection(hung, undefined);
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

	it('gives a beam pointing at the sky a length rather than an infinity', async () => {
		const { throwDistance } = await import('./stage.js');
		expect(Number.isFinite(throwDistance({ x: 0, y: 2, z: 0 }, { x: 0, y: 1, z: 0 }))).toBe(true);
	});
});

describe('a position, in its two forms', () => {
	/**
	 * `Point` and `Axial` are the same fact with one detail added, so an editor
	 * should not make an operator choose a variant before they can type a number.
	 * These two are what keep the panels from drifting apart about which is which.
	 */
	it('splits a plain point and puts it back unchanged', () => {
		const point = { Point: { x: 1, y: 2, z: 3 } };
		const parts = splitPosition(point);

		expect(parts.point).toEqual({ x: 1, y: 2, z: 3 });
		expect(parts.direction).toBeNull();
		expect(joinPosition(parts.point, parts.direction)).toEqual(point);
	});

	it('splits an axial one and puts it back unchanged', () => {
		const axial = {
			Axial: { position: { x: 1, y: 6, z: -2 }, direction: { x: 0, y: -1, z: 0.5 } }
		};
		const parts = splitPosition(axial);

		expect(parts.point).toEqual({ x: 1, y: 6, z: -2 });
		expect(parts.direction).toEqual({ x: 0, y: -1, z: 0.5 });
		expect(joinPosition(parts.point, parts.direction)).toEqual(axial);
	});

	it('treats an unplaced fixture as the origin, facing nowhere', () => {
		const parts = splitPosition(null);
		expect(parts.point).toEqual({ x: 0, y: 0, z: 0 });
		expect(parts.direction).toBeNull();
	});

	/** A direction turns a point into an axial position, and only that. */
	it('picks the variant from whether there is a direction', () => {
		expect(joinPosition({ x: 0, y: 0, z: 0 }, null)).toHaveProperty('Point');
		expect(joinPosition({ x: 0, y: 0, z: 0 }, HANGING)).toHaveProperty('Axial');
	});

	/**
	 * A light with no aim yet is hanging, so the default direction points at the
	 * floor beneath it rather than at the origin — which for a light upstage would
	 * be aiming it across the room.
	 */
	it('defaults a new direction to straight down', () => {
		expect(HANGING).toEqual({ x: 0, y: -1, z: 0 });
	});
});
