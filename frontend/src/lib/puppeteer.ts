/**
 * Grabbing a moving head in three dimensions.
 *
 * The spec asks for pan and tilt to be programmed by taking hold of the axis and
 * turning it, "just like it would behave in real life". What the 3D view actually
 * has to work with is a pointer, which is a ray through the scene — so every drag
 * comes down to the same two steps: put the ray on the plane the axis turns in, and
 * read the angle it lands at.
 *
 * Both steps are here, and both are pure, so the geometry is testable without a
 * canvas and the component is left with nothing but the pointer events. The angles
 * themselves come from `stage.ts`, which is the only place that knows how pan and
 * tilt turn into a beam.
 */

import type { Fixture, FixtureType, Vec3 } from './generated/index.js';
import { beamDirection, fixtureFacing, fixturePoint } from './stage.js';
import type { Showing } from './stores/output.js';

/** A pointer, as the scene sees it: somewhere to look from and a way to look. */
export type Ray = { origin: Vec3; direction: Vec3 };

/**
 * Where a ray crosses a plane, or null when it runs parallel to it.
 *
 * `normal` need not be a unit vector; only its direction matters.
 */
export function rayOnPlane(ray: Ray, point: Vec3, normal: Vec3): Vec3 | null {
	const denominator = dot(ray.direction, normal);
	if (Math.abs(denominator) < 1e-6) return null;
	const t = dot(sub(point, ray.origin), normal) / denominator;
	if (!Number.isFinite(t)) return null;
	return {
		x: ray.origin.x + ray.direction.x * t,
		y: ray.origin.y + ray.direction.y * t,
		z: ray.origin.z + ray.direction.z * t
	};
}

/**
 * The bearing from a fixture to a point, in degrees. Null on the axis itself.
 *
 * A bearing rather than a pan value, and that is the point: a gizmo is *turned*, not
 * aimed. What matters is how far round the pointer has moved since it took hold, so
 * the axis it is attached to moves by the same amount from wherever it already was.
 * Reading an absolute angle instead would make the head snap to the pointer the
 * instant the ring was touched, which is not what grabbing a yoke does.
 */
export function bearingFromPoint(fixture: Fixture, point: Vec3): number | null {
	const at = fixturePoint(fixture);
	if (!at) return null;
	const dx = point.x - at.x;
	const dz = point.z - at.z;
	if (Math.hypot(dx, dz) < 1e-4) return null;
	return (Math.atan2(dx, dz) * 180) / Math.PI;
}

/**
 * How far above the fixture a point lies, in degrees, measured in the vertical plane
 * the head is currently panned to.
 *
 * The reach is signed along that plane, so a drag that wanders round behind the
 * fixture keeps counting in the same direction instead of mirroring.
 */
export function elevationFromPoint(
	fixture: Fixture,
	type: FixtureType | undefined,
	point: Vec3,
	showing: Showing
): number | null {
	const at = fixturePoint(fixture);
	if (!at) return null;
	const facing = bearingOnFloor(fixture, type, showing);
	const reach = (point.x - at.x) * facing.x + (point.z - at.z) * facing.z;
	const rise = point.y - at.y;
	if (Math.abs(reach) < 1e-4 && Math.abs(rise) < 1e-4) return null;
	return (Math.atan2(rise, reach) * 180) / Math.PI;
}

/**
 * The plane tilt turns in, as a unit vector on the floor.
 *
 * The way the head is pointing now, flattened. A head hanging straight down has no
 * bearing of its own, so it falls back to the way it was hung — which for a
 * straight-down rig is downstage, and is at any rate the same zero that pan is
 * measured from.
 *
 * Exported because the 3D view has to *draw* that plane as well as read angles out
 * of it: the tilt arc is a gizmo lying in exactly this direction.
 */
export function bearingOnFloor(
	fixture: Fixture,
	type: FixtureType | undefined,
	showing: Showing
): { x: number; z: number } {
	for (const v of [
		beamDirection(fixture, type, showing),
		fixtureFacing(fixture) ?? { x: 0, y: 0, z: 1 }
	]) {
		const length = Math.hypot(v.x, v.z);
		if (length > 1e-4) return { x: v.x / length, z: v.z / length };
	}
	return { x: 0, z: 1 };
}

const dot = (a: Vec3, b: Vec3) => a.x * b.x + a.y * b.y + a.z * b.z;
const sub = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x - b.x, y: a.y - b.y, z: a.z - b.z });
