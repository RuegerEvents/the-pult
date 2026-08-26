/**
 * The stage, as maths.
 *
 * Both stage views — the plan and the 3D rig — read positions and colours through
 * here, so the two cannot disagree about where anything is or what it is doing.
 *
 * # Which way is which
 *
 * The rig lives in metres. **Y is up. X increases to the right as seen from front
 * of house. Z increases downstage, towards the audience.**
 *
 * That last one is chosen rather than inherited, and for two reasons that agree: a
 * ground plan is drawn with the audience at the bottom of the page, so the image's
 * own +Y — downwards, as images are numbered — is downstage; and three.js points its
 * default camera down −Z, which puts front of house at +Z looking upstage. A plan
 * therefore maps onto the floor with no flip anywhere, and the 3D view opens facing
 * the right way.
 *
 * That convention is stated once here and once in `crates/pult-schema/src/types/
 * stage.rs`. Nothing else should be deciding it.
 */

import type {
	Fixture,
	FixtureType,
	ParameterKind,
	ParameterValue,
	StagePlan,
	Vec3
} from './generated/index.js';
import { parameterKey } from './patch.js';

// ── Where things are ──────────────────────────────────────────────────────────

/** A point on the floor, in metres. The two axes a ground plan can show. */
export type PlanPoint = { x: number; z: number };

/** Where a fixture hangs, or `null` if it has never been placed. */
export function fixturePoint(fixture: Fixture): Vec3 | null {
	const position = fixture.position;
	if (!position) return null;
	return 'Point' in position ? position.Point : position.Axial.position;
}

/** Which way a fixture faces at rest, if it was placed axially. */
export function fixtureFacing(fixture: Fixture): Vec3 | null {
	const position = fixture.position;
	if (!position || !('Axial' in position)) return null;
	return position.Axial.direction;
}

/** Turn a point on the floor into the world position a fixture is stored at. */
export const planToWorld = (point: PlanPoint, y: number): Vec3 => ({ x: point.x, y, z: point.z });

/**
 * Where a plan's pixel lands on the floor, and back again.
 *
 * `origin` is the world point under the image's top-left corner, and rotation turns
 * the image about that corner — so a plan drawn at an angle to the room can be
 * squared up without moving where it starts.
 */
export function pixelToPlan(plan: StagePlan, px: number, py: number): PlanPoint {
	const s = plan.metres_per_pixel;
	const [dx, dz] = rotate(px * s, py * s, plan.rotation_deg);
	return { x: plan.origin.x + dx, z: plan.origin.z + dz };
}

export function planToPixel(plan: StagePlan, point: PlanPoint): { px: number; py: number } {
	const [dx, dz] = rotate(point.x - plan.origin.x, point.z - plan.origin.z, -plan.rotation_deg);
	return { px: dx / plan.metres_per_pixel, py: dz / plan.metres_per_pixel };
}

/**
 * The origin a plan needs for one of its pixels to sit on the show's origin.
 *
 * Lining a drawing up with the room is done by naming the pixel that is (0, 0);
 * the image's corner then goes wherever that leaves it, rotation included. The
 * plan's old origin says nothing about where the new one belongs.
 */
export function originForPixel(plan: StagePlan, px: number, py: number): PlanPoint {
	const s = plan.metres_per_pixel;
	const [dx, dz] = rotate(px * s, py * s, plan.rotation_deg);
	return { x: -dx, z: -dz };
}

function rotate(x: number, z: number, degrees: number): [number, number] {
	if (degrees === 0) return [x, z];
	const r = (degrees * Math.PI) / 180;
	const cos = Math.cos(r);
	const sin = Math.sin(r);
	return [x * cos - z * sin, x * sin + z * cos];
}

/** How wide and deep a plan is on the floor, in metres. */
export const planExtent = (plan: StagePlan) => ({
	width: plan.width_px * plan.metres_per_pixel,
	depth: plan.height_px * plan.metres_per_pixel
});

/**
 * The scale that makes two clicked points the distance apart they really are.
 *
 * Calibrating from something known — the width of the stage, a truss span — is the
 * only way a drawing becomes a map, and it is one division.
 */
export function calibrationScale(
	a: { px: number; py: number },
	b: { px: number; py: number },
	metres: number
): number | null {
	const pixels = Math.hypot(b.px - a.px, b.py - a.py);
	if (pixels < 1 || !(metres > 0)) return null;
	return metres / pixels;
}

/** A box in metres that holds every placed fixture, with room to breathe. */
export function fixtureBounds(fixtures: Fixture[], margin = 2) {
	const points = fixtures.map(fixturePoint).filter((p) => p !== null);
	if (points.length === 0) return { minX: -8, maxX: 8, minZ: -6, maxZ: 6 };
	const xs = points.map((p) => p.x);
	const zs = points.map((p) => p.z);
	return {
		minX: Math.min(...xs) - margin,
		maxX: Math.max(...xs) + margin,
		minZ: Math.min(...zs) - margin,
		maxZ: Math.max(...zs) + margin
	};
}

// ── What things are doing ─────────────────────────────────────────────────────

const asNumber = (value: ParameterValue | undefined): number | null => {
	if (!value) return null;
	if (value.type === 'Float') return value.value;
	if (value.type === 'Int') return value.value;
	if (value.type === 'Bool') return value.value ? 1 : 0;
	return null;
};

const read = (fixture: Fixture, kind: ParameterKind) => fixture.live_values[parameterKey(kind)];

/** What a fixture is putting out: a colour and how much of it, 0–1. */
export function fixtureOutput(fixture: Fixture): { r: number; g: number; b: number; level: number } {
	const colour = read(fixture, 'ColorRgb');
	const rgb =
		colour?.type === 'Color' ? colour.value : { r: 1, g: 1, b: 1 };
	// A fixture with no dimmer channel is as bright as its colour: a colour-mixing
	// LED with everything up is on, and reporting it dark would be a lie.
	const intensity = asNumber(read(fixture, 'Intensity'));
	// A relay driving a practical is either on or off, and that is its level.
	const switched = asNumber(read(fixture, { Switch: 0 }));
	const level = intensity ?? switched ?? (colour ? 1 : 0);
	return { r: rgb.r, g: rgb.g, b: rgb.b, level: clamp(level) };
}

/** That output as a CSS colour, dimmed by its own level. */
export function fixtureTint(fixture: Fixture): string {
	const { r, g, b, level } = fixtureOutput(fixture);
	const byte = (v: number) => Math.round(clamp(v) * level * 255);
	return `rgb(${byte(r)}, ${byte(g)}, ${byte(b)})`;
}

/**
 * Where a moving head is pointing, as an angle on the plan in degrees.
 *
 * Pan is reported 0–1 across the fixture's whole travel, which for want of a
 * per-type range is taken as 540° centred on the way the fixture faces — wrong in
 * detail for any specific head, right about which way it is swinging, and replaced
 * the moment `FixtureType` carries real ranges.
 */
export function panAngle(fixture: Fixture, type: FixtureType | undefined): number | null {
	if (!type?.parameters.some((p) => p.kind === 'Pan')) return null;
	const pan = asNumber(read(fixture, 'Pan'));
	if (pan === null) return null;
	const facing = fixtureFacing(fixture);
	const rest = facing ? (Math.atan2(facing.x, facing.z) * 180) / Math.PI : 0;
	return rest + (pan - 0.5) * 540;
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));


// ── The rig in three dimensions ───────────────────────────────────────────────

/**
 * Where the camera starts: front of house, eye height, looking at the stage.
 *
 * The spec calls the FOH perspective "the primary operational perspective", so the
 * view opens there rather than somewhere neutral. How far back depends on how wide
 * the rig is — a festival stage seen from three metres is a wall.
 */
export function fohCamera(fixtures: Fixture[]): { position: [number, number, number]; target: [number, number, number] } {
	const bounds = fixtureBounds(fixtures, 0);
	const width = Math.max(bounds.maxX - bounds.minX, 6);
	const centreX = (bounds.minX + bounds.maxX) / 2;
	// Far enough back that the whole width is comfortably inside a 50° lens, and
	// never closer than the front row would be.
	const back = Math.max(width * 1.4, 11);
	return {
		// Eye height, because that is where an operator actually stands. The view
		// is meant to look like the room, not like a plan drawn in perspective.
		position: [centreX, 1.7, bounds.maxZ + back],
		target: [centreX, 1.8, (bounds.minZ + bounds.maxZ) / 2]
	};
}

/**
 * Which way a fixture's beam leaves it, as a unit vector.
 *
 * A fixture hung axially says so itself. Anything else is assumed to be pointing
 * at the floor, which is what a par on a bar is doing and is at least not a
 * statement about a direction nobody has given.
 */
export function beamDirection(fixture: Fixture, type: FixtureType | undefined): Vec3 {
	const facing = fixtureFacing(fixture) ?? { x: 0, y: -1, z: 0 };
	const angle = panAngle(fixture, type);
	if (angle === null) return normalise(facing);

	// Pan swings about the vertical, so the beam keeps its tilt and turns.
	const down = Math.min(0, facing.y);
	const spread = Math.hypot(facing.x, facing.z) || 0.35;
	const radians = (angle * Math.PI) / 180;
	return normalise({ x: Math.sin(radians) * spread, y: down, z: Math.cos(radians) * spread });
}

function normalise(v: Vec3): Vec3 {
	const length = Math.hypot(v.x, v.y, v.z) || 1;
	return { x: v.x / length, y: v.y / length, z: v.z / length };
}

/** How far a beam runs before it meets the floor, in metres. */
export function throwDistance(from: Vec3, direction: Vec3, floorY = 0): number {
	if (direction.y >= -1e-3) return 12;
	return Math.min(40, Math.max(0.5, (from.y - floorY) / -direction.y));
}
