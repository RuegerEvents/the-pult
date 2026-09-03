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
	SceneObject,
	StagePlan,
	Vec3
} from './generated/index.js';
import { facing, worldTransform } from './scene.js';
import { parameterKey } from './patch.js';
import type { Showing } from './stores/output.js';

// ── Where things are ──────────────────────────────────────────────────────────

/** A point on the floor, in metres. The two axes a ground plan can show. */
export type PlanPoint = { x: number; z: number };

/**
 * Straight down, which is where a light hung on a bar points before anybody aims it.
 *
 * A transform that has not been turned faces this way already; it is named because a
 * caller that has no fixture at all still needs an answer.
 */
export const HANGING = { x: 0, y: -1, z: 0 };

/**
 * Where a fixture hangs, with every truss above it applied.
 *
 * `objects` is what it may hang off. Pass none and the answer is the fixture's own
 * numbers, which is right for a rig nobody has drawn and wrong for one somebody has,
 * so a caller that has the drawing should hand it over.
 */
export function fixturePoint(fixture: Fixture, objects?: Map<string, SceneObject>): Vec3 | null {
	if (!fixture.position) return null;
	if (!objects) return fixture.position.position;
	return worldTransform(fixture.position, fixture.parent, objects).position;
}

/** Which way a fixture faces at rest, or `null` if it has never been placed. */
export function fixtureFacing(fixture: Fixture, objects?: Map<string, SceneObject>): Vec3 | null {
	if (!fixture.position) return null;
	const placed = objects
		? worldTransform(fixture.position, fixture.parent, objects)
		: fixture.position;
	return facing(placed);
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
export function fixtureBounds(
	fixtures: Fixture[],
	margin = 2,
	objects?: Map<string, SceneObject>
) {
	const points = fixtures.map((f) => fixturePoint(f, objects)).filter((p) => p !== null);
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

/**
 * What a fixture is putting out: a colour and how much of it, 0–1.
 *
 * Asked of a [`Showing`] rather than read off the fixture, because nothing stores what
 * a parameter is doing any more — the console keeps what is *driving* it and every
 * consumer works out a number for the moment it is drawing. A `Showing` that cannot
 * place itself on the station's clock answers nothing, and a fixture then reads as
 * dark: a light nobody can vouch for is better drawn as off than as a guess.
 */
export function fixtureOutput(
	fixture: Fixture,
	showing: Showing
): { r: number; g: number; b: number; level: number } {
	const read = (kind: ParameterKind) => showing.value(fixture.id, parameterKey(kind)) ?? undefined;
	const colour = read('ColorRgb');
	const rgb =
		colour?.type === 'Color' ? colour.value : { r: 1, g: 1, b: 1 };
	// A fixture with no dimmer channel is as bright as its colour: a colour-mixing
	// LED with everything up is on, and reporting it dark would be a lie.
	const intensity = asNumber(read('Intensity'));
	// A relay driving a practical is either on or off, and that is its level.
	const switched = asNumber(read({ Switch: 0 }));
	const level = intensity ?? switched ?? (colour ? 1 : 0);
	return { r: rgb.r, g: rgb.g, b: rgb.b, level: clamp(level) };
}

/** That output as a CSS colour, dimmed by its own level. */
export function fixtureTint(fixture: Fixture, showing: Showing): string {
	const { r, g, b, level } = fixtureOutput(fixture, showing);
	const byte = (v: number) => Math.round(clamp(v) * level * 255);
	return `rgb(${byte(r)}, ${byte(g)}, ${byte(b)})`;
}

/**
 * How far a head swings and nods, end to end, in degrees, for a type that does not
 * say.
 *
 * The fallback and no longer the answer: a fixture imported from a GDTF file carries
 * the travel its manufacturer measured, and {@link travelOf} reads that. These two
 * are what is left for a type somebody typed into the editor and for a light whose
 * file never said — right about which way a head is moving, wrong in detail for any
 * particular one, and named rather than written twice so the two views and the
 * inverse below cannot drift apart about them.
 */
export const PAN_TRAVEL = 540;
export const TILT_TRAVEL = 270;

/**
 * How far a parameter of this type actually travels, in degrees.
 *
 * The type's own `physical` range where it has one — which is the whole point of
 * importing a fixture definition: a head with 540° of pan and one with 630° stop
 * pointing at the same place for the same number.
 *
 * A range in any unit but degrees is ignored rather than trusted: this function
 * answers an angle, and a channel measured in seconds is not one.
 */
export function travelOf(
	type: FixtureType | undefined,
	kind: 'Pan' | 'Tilt'
): number {
	const fallback = kind === 'Pan' ? PAN_TRAVEL : TILT_TRAVEL;
	const range = type?.parameters.find((p) => p.kind === kind)?.physical;
	if (!range || range.unit !== 'Degrees') return fallback;
	const travel = Math.abs(range.to - range.from);
	return travel > 0 ? travel : fallback;
}

/**
 * The direction a fixture points with pan and tilt both centred.
 *
 * `objects` is the drawing it hangs in. Without it the answer is the fixture's own
 * rotation, which is right for a rig nobody has drawn and wrong for a light on a
 * truss somebody turned — so every caller that has the drawing passes it, and the
 * two stage views both do.
 */
function restDirection(fixture: Fixture, objects?: Map<string, SceneObject>): Vec3 {
	return normalise(fixtureFacing(fixture, objects) ?? HANGING);
}

/** Compass bearing of a direction on the floor: degrees from downstage towards +X. */
const bearingOf = (v: Vec3) => (Math.atan2(v.x, v.z) * 180) / Math.PI;

/** How far above the horizontal a direction points, in degrees. Down is negative. */
const elevationOf = (v: Vec3) => (Math.atan2(v.y, Math.hypot(v.x, v.z)) * 180) / Math.PI;

/** A direction back from a bearing and an elevation, both in degrees. */
function fromAngles(bearing: number, elevation: number): Vec3 {
	const b = (bearing * Math.PI) / 180;
	const e = (elevation * Math.PI) / 180;
	const flat = Math.cos(e);
	return { x: flat * Math.sin(b), y: Math.sin(e), z: flat * Math.cos(b) };
}

const hasParameter = (type: FixtureType | undefined, kind: ParameterKind) =>
	type?.parameters.some((p) => p.kind === kind) ?? false;

/**
 * Where a moving head is pointing, as an angle on the plan in degrees.
 *
 * Pan is reported 0–1 across the fixture's whole travel, centred on the way the
 * fixture hangs, so 0.5 is always the rest bearing however the head was rigged.
 */
export function panAngle(
	fixture: Fixture,
	type: FixtureType | undefined,
	showing: Showing,
	/**
	 * The drawing this fixture hangs in, where the caller has it. A light on a truss
	 * somebody turned points where the truss points it, and without this the answer
	 * is the fixture's own rotation alone.
	 */
	objects?: Map<string, SceneObject>
): number | null {
	if (!hasParameter(type, 'Pan')) return null;
	const pan = asNumber(showing.value(fixture.id, 'Pan') ?? undefined);
	if (pan === null) return null;
	return bearingOf(restDirection(fixture, objects)) + (pan - 0.5) * travelOf(type, 'Pan');
}

/**
 * How far above the horizontal it is pointing, in degrees. Down is negative.
 *
 * The same shape as {@link panAngle}: 0.5 is the elevation it was hung at, and the
 * travel opens either side of that.
 */
export function tiltAngle(
	fixture: Fixture,
	type: FixtureType | undefined,
	showing: Showing,
	/**
	 * The drawing this fixture hangs in, where the caller has it. A light on a truss
	 * somebody turned points where the truss points it, and without this the answer
	 * is the fixture's own rotation alone.
	 */
	objects?: Map<string, SceneObject>
): number | null {
	if (!hasParameter(type, 'Tilt')) return null;
	const tilt = asNumber(showing.value(fixture.id, 'Tilt') ?? undefined);
	if (tilt === null) return null;
	return elevationOf(restDirection(fixture, objects)) + (tilt - 0.5) * travelOf(type, 'Tilt');
}

/**
 * The pan and tilt that would put a head's beam on a point in the room.
 *
 * The inverse of {@link beamDirection}, and the thing that makes a beam draggable:
 * both stage views let an operator take hold of where the light lands rather than of
 * the two numbers that put it there, which is the spec's puppeteering asked for from
 * the other end.
 *
 * Returned clamped to 0–1, so aiming behind a head that cannot turn that far gives
 * the closest it can manage rather than a value nothing can accept.
 */
export function aimAt(
	fixture: Fixture,
	type: FixtureType | undefined,
	target: Vec3,
	/**
	 * The drawing this fixture hangs in, where the caller has it. A light on a truss
	 * somebody turned points where the truss points it, and without this the answer
	 * is the fixture's own rotation alone.
	 */
	objects?: Map<string, SceneObject>
): { pan: number | null; tilt: number | null } {
	const at = fixturePoint(fixture, objects);
	if (!at) return { pan: null, tilt: null };
	const towards = normalise({ x: target.x - at.x, y: target.y - at.y, z: target.z - at.z });
	const rest = restDirection(fixture, objects);

	const pan = hasParameter(type, 'Pan')
		? clamp(0.5 + wrapDegrees(bearingOf(towards) - bearingOf(rest)) / travelOf(type, 'Pan'))
		: null;
	const tilt = hasParameter(type, 'Tilt')
		? clamp(0.5 + (elevationOf(towards) - elevationOf(rest)) / travelOf(type, 'Tilt'))
		: null;
	return { pan, tilt };
}

/**
 * An angle brought into −180…180, so the short way round is the way taken.
 *
 * Shared with the gizmos, which add up a drag one move at a time: without this a
 * pointer crossing the back of a fixture would read as most of a turn the other way.
 */
export function wrapDegrees(degrees: number): number {
	const wrapped = ((degrees + 180) % 360 + 360) % 360;
	return wrapped - 180;
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
 *
 * A head that can move turns from there: pan swings the bearing about the vertical,
 * tilt nods about the horizontal axis that swing left behind. Either one on its own
 * leaves the other where the fixture was hung, so a rig with only pan patched still
 * points the way it did before tilt existed.
 */
export function beamDirection(
	fixture: Fixture,
	type: FixtureType | undefined,
	showing: Showing,
	/**
	 * The drawing this fixture hangs in, where the caller has it. A light on a truss
	 * somebody turned points where the truss points it, and without this the answer
	 * is the fixture's own rotation alone.
	 */
	objects?: Map<string, SceneObject>
): Vec3 {
	const rest = restDirection(fixture, objects);
	const bearing = panAngle(fixture, type, showing, objects);
	const elevation = tiltAngle(fixture, type, showing, objects);
	if (bearing === null && elevation === null) return rest;
	return fromAngles(bearing ?? bearingOf(rest), elevation ?? elevationOf(rest));
}

/**
 * Where a fixture's beam meets the floor, which is the handle an operator grabs.
 *
 * `maxThrow` is what keeps that handle in reach. A beam near the horizontal lands
 * arbitrarily far away and one above it never lands at all, so the honest answer is
 * a point tens of metres off the plan — which is a handle nobody can take hold of,
 * and which slides out from under the pointer the moment a drag flattens the beam.
 * Capped, it stops at the furthest distance worth drawing and still says which way
 * the light is going.
 */
export function beamSpot(
	fixture: Fixture,
	type: FixtureType | undefined,
	showing: Showing,
	{ floorY = 0, maxThrow = Infinity }: { floorY?: number; maxThrow?: number } = {},
	/**
	 * The drawing this fixture hangs in, where the caller has it. A light on a truss
	 * somebody turned points where the truss points it, and without this the answer
	 * is the fixture's own rotation alone.
	 */
	objects?: Map<string, SceneObject>
): Vec3 | null {
	const at = fixturePoint(fixture, objects);
	if (!at) return null;
	const direction = beamDirection(fixture, type, showing, objects);
	const length = Math.min(throwDistance(at, direction, floorY), maxThrow);
	return {
		x: at.x + direction.x * length,
		y: at.y + direction.y * length,
		z: at.z + direction.z * length
	};
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

/**
 * How long to *draw* a beam whose axis meets the floor after `length` metres.
 *
 * The drawn beam is a cone cut square to its own axis, and a square cut at the
 * axis's floor hit is level with the floor only when the beam is vertical. Aimed at
 * an angle, the cut tilts with it: the downhill half of the end ring is under the
 * deck and the uphill half stands in the air, ending in a visible slanted edge above
 * the spot it is lighting. So the cone is run on until its whole end ring is below
 * the floor, and the floor — the shader's fade over the deck and the deck's own
 * depth — does the cutting, which is the only cut that is level with it.
 *
 * The highest point of the end ring sits `r · sin θ` above the ring's centre, θ being
 * the beam's angle from the vertical and `r` the ring's radius, which itself grows
 * with the length by `spread` per metre. Solving for the extra run that puts the
 * centre that far under the floor gives the closed form below. A beam grazing the
 * floor so flat that it widens faster than it descends has no such length, and gets
 * the longest throw drawn at all.
 */
export function drawnLength(length: number, direction: Vec3, spread: number, lens: number): number {
	const d = normalise(direction);
	if (d.y >= -1e-3) return length;
	const sinTheta = Math.hypot(d.x, d.z);
	const tanTheta = sinTheta / -d.y;
	const room = 1 - spread * tanTheta;
	if (room <= 1e-3) return 40;
	const extra = ((lens + length * spread) * tanTheta) / room;
	return Math.min(40, length + extra);
}
