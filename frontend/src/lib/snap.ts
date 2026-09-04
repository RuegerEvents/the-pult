/**
 * Making one piece meet another.
 *
 * Two rules, and they are the whole of building a rig by hand.
 *
 * **A grid**, because a rig is set out in half-metres and an operator dragging a truss
 * means "six metres up", not 5.9847. Alt bypasses it for the one time in twenty when
 * somebody means 1.37.
 *
 * **Connectors**, because a grid cannot say that a truss end bolts to a truss end. Each
 * catalogue piece declares where it joins its neighbours and what kind of joint that
 * is; like mates like and nothing else, so a deck edge never catches a truss however
 * close it is dragged. Mating puts the two points together and turns the moving piece
 * so the two faces are opposite — which is what a bolt does, and the reason the
 * rotation comes out of the snap rather than being something else to get right by
 * hand.
 *
 * Pure. Everything here takes world-space placements and answers world-space ones; the
 * panel is what turns a gizmo's drag into a `SceneObject.transform` through `localOf`.
 * `crates/pult-schema/tests/stock.rs` states the mating rule from the other side, over
 * the whole catalogue.
 */

import type { Connector, ConnectorKind, SceneObject, Transform, Vec3 } from './generated/index.js';
import { basisToEuler, eulerToBasis, IDENTITY } from './scene.js';
import { piece } from './stock.js';

/**
 * How close two joints have to be before they bolt together, in metres.
 *
 * Three hundred millimetres, which is about a truss's own square: close enough that
 * nothing snaps by surprise across a stage, generous enough that a drag does not have
 * to be accurate to the centimetre. A joint that *has* snapped shows it by moving, so
 * there is no state to explain.
 */
export const SNAP_RADIUS = 0.3;

/** How close two joints have to be to count as already bolted, in metres. */
const MATED = 0.001;

/** One connector, placed in the world. */
export type PlacedConnector = {
	/** The object it belongs to, so a piece never snaps to itself. */
	object: string;
	/** Which of that piece's connectors, so the `+` handles can name one. */
	index: number;
	at: Vec3;
	facing: Vec3;
	kind: ConnectorKind;
};

/** Round to the grid. Zero — or a held Alt — leaves the number alone. */
export function toGrid(value: number, grid: number): number {
	return grid > 0 ? Math.round(value / grid) * grid : value;
}

export function pointToGrid(point: Vec3, grid: number): Vec3 {
	return { x: toGrid(point.x, grid), y: toGrid(point.y, grid), z: toGrid(point.z, grid) };
}

/**
 * Every joint a piece has, where the piece actually is.
 *
 * An object with no catalogue id has none: a mesh out of a drawing says nothing about
 * where it bolts, and inventing joints for one would make a truss somebody measured
 * snap to a place nobody measured. That is a named limit rather than an oversight —
 * *connectors declared on imported meshes* is out of scope.
 */
export function connectorsOf(object: SceneObject, world: Transform): PlacedConnector[] {
	const entry = piece(object.catalogue);
	if (!entry) return [];
	return entry.connectors.map((connector, index) => ({
		object: object.id,
		index,
		at: through(world, connector.at),
		facing: turned(world, connector.facing),
		kind: connector.kind
	}));
}

/**
 * The joints nothing is bolted to.
 *
 * "Free" is defined by the geometry rather than by a field, because there is no field:
 * two pieces are joined when their connectors are in the same place facing opposite
 * ways, which is what the snap put them in. So a run of four sections offers a `+`
 * handle at each end of the run and nowhere in the middle, and it goes on being right
 * when somebody deletes a section out of the middle.
 */
export function freeConnectors(all: PlacedConnector[]): PlacedConnector[] {
	return all.filter(
		(one) =>
			!all.some(
				(other) =>
					other !== one &&
					other.kind === one.kind &&
					distance(other.at, one.at) < MATED &&
					dot(other.facing, one.facing) < -0.999
			)
	);
}

/** What a snap found: where the moving piece has to go, and which joint it took. */
export type Snap = {
	/** The moving piece's new world placement. */
	transform: Transform;
	/** The joint it bolted to, for the panel to say so. */
	onto: PlacedConnector;
	/** And which of its own it used. */
	mine: PlacedConnector;
};

/**
 * The nearest pair of like joints within the radius, and the placement that mates
 * them.
 *
 * `mine` is the moving piece's joints as they are *now*, mid-drag; `theirs` is
 * everything else's free ones. The nearest pair wins, and the answer is the moving
 * piece's whole placement — position and rotation — because a bolted joint decides
 * both. Its scale is left exactly as it was: a mirrored truss stays mirrored.
 */
export function snapConnectors(
	moving: Transform,
	mine: PlacedConnector[],
	theirs: PlacedConnector[]
): Snap | null {
	let best: { mine: PlacedConnector; onto: PlacedConnector; away: number } | null = null;
	for (const one of mine) {
		for (const other of theirs) {
			if (other.object === one.object || other.kind !== one.kind) continue;
			const away = distance(one.at, other.at);
			if (away > SNAP_RADIUS) continue;
			if (!best || away < best.away) best = { mine: one, onto: other, away };
		}
	}
	if (!best) return null;
	return { transform: mate(moving, best.mine, best.onto), onto: best.onto, mine: best.mine };
}

/**
 * The placement that brings one joint onto another, facing the opposite way.
 *
 * Worked out as a turn *about the moving piece's current placement* rather than from
 * scratch, so a section dragged in at some angle keeps everything the mating does not
 * decide — its scale, and the roll about the joint's own axis, which is the one degree
 * a bolted circle of holes leaves free.
 */
export function mate(moving: Transform, mine: PlacedConnector, onto: PlacedConnector): Transform {
	const wanted = { x: -onto.facing.x, y: -onto.facing.y, z: -onto.facing.z };
	const turn = rotationTaking(mine.facing, wanted);

	// The moving piece's own rotation, with that turn applied on top of it.
	const rotation = basisToEuler(multiply(turn, eulerToBasis(moving.rotation)));
	// And then slide it so the joint lands exactly where the other one is. The joint's
	// offset from the piece's origin turns with the piece, so it is recomputed rather
	// than reused.
	const offset = rotate(turn, {
		x: mine.at.x - moving.position.x,
		y: mine.at.y - moving.position.y,
		z: mine.at.z - moving.position.z
	});
	return {
		...moving,
		position: {
			x: onto.at.x - offset.x,
			y: onto.at.y - offset.y,
			z: onto.at.z - offset.z
		},
		rotation
	};
}

/**
 * Where a new piece goes when somebody presses the `+` on a free joint.
 *
 * Its own first connector of that kind meets the one that was pressed, which is what
 * makes clicking three times lay a run: each new section bolts to the end of the last.
 * A corner is the exception a person notices — six joints, all alike — and it goes on
 * whichever face was pressed, which is what makes the run turn.
 */
export function placedOnConnector(
	catalogueId: string,
	onto: PlacedConnector
): Transform | null {
	const entry = piece(catalogueId);
	const mine = entry?.connectors.find((connector) => connector.kind === onto.kind);
	if (!entry || !mine) return null;
	// From its own frame at the origin, which is what a piece that does not exist yet
	// has: the mate then works out the whole placement.
	return mate(IDENTITY, asPlaced(mine, entry.id), onto);
}

function asPlaced(connector: Connector, id: string): PlacedConnector {
	return { object: id, index: 0, at: connector.at, facing: connector.facing, kind: connector.kind };
}

// ── The arithmetic ────────────────────────────────────────────────────────────

type Basis = number[][];

/** A point through a whole placement: scaled, turned, moved. */
function through(transform: Transform, point: Vec3): Vec3 {
	const basis = eulerToBasis(transform.rotation);
	const scaled = {
		x: point.x * transform.scale.x,
		y: point.y * transform.scale.y,
		z: point.z * transform.scale.z
	};
	const turnedPoint = rotate(basis, scaled);
	return {
		x: turnedPoint.x + transform.position.x,
		y: turnedPoint.y + transform.position.y,
		z: turnedPoint.z + transform.position.z
	};
}

/**
 * A direction through a placement: turned, and never moved.
 *
 * The scale is applied and then the length taken back out, because a mirrored piece's
 * joints really do face the other way — that is what a reflection is, and a facing that
 * ignored it would snap a mirrored truss back to front.
 */
function turned(transform: Transform, direction: Vec3): Vec3 {
	const basis = eulerToBasis(transform.rotation);
	const scaled = {
		x: direction.x * Math.sign(transform.scale.x || 1),
		y: direction.y * Math.sign(transform.scale.y || 1),
		z: direction.z * Math.sign(transform.scale.z || 1)
	};
	return normalise(rotate(basis, scaled));
}

function rotate(basis: Basis, v: Vec3): Vec3 {
	return {
		x: basis[0][0] * v.x + basis[0][1] * v.y + basis[0][2] * v.z,
		y: basis[1][0] * v.x + basis[1][1] * v.y + basis[1][2] * v.z,
		z: basis[2][0] * v.x + basis[2][1] * v.y + basis[2][2] * v.z
	};
}

function multiply(a: Basis, b: Basis): Basis {
	return [0, 1, 2].map((i) =>
		[0, 1, 2].map((j) => [0, 1, 2].reduce((sum, k) => sum + a[i][k] * b[k][j], 0))
	);
}

/** The rotation matrix taking one unit vector to another, the short way round. */
export function rotationTaking(from: Vec3, to: Vec3): Basis {
	const a = normalise(from);
	const b = normalise(to);
	const along = Math.min(1, Math.max(-1, dot(a, b)));
	if (along > 0.999999) return eulerToBasis({ x: 0, y: 0, z: 0 });
	// Exactly opposite — a section going on the far end of a run, which is most of
	// them. Any perpendicular axis mates the two joints, and they are not all the same:
	// a flip about the piece's own long axis or about a diagonal puts its up somewhere
	// else, and then a light clamped to its top chord comes back hanging underneath.
	// So it turns about the **vertical** where the vertical is perpendicular, which is
	// what a person does to a bar, and about X for a piece that is standing up.
	const axis =
		along < -0.999999
			? Math.abs(a.y) < 0.9
				? { x: 0, y: 1, z: 0 }
				: { x: 1, y: 0, z: 0 }
			: normalise(cross(a, b));
	return axisAngle(axis, Math.acos(along));
}

function axisAngle(axis: Vec3, angle: number): Basis {
	const [sin, cos] = [Math.sin(angle), Math.cos(angle)];
	const t = 1 - cos;
	return [
		[t * axis.x * axis.x + cos, t * axis.x * axis.y - sin * axis.z, t * axis.x * axis.z + sin * axis.y],
		[t * axis.x * axis.y + sin * axis.z, t * axis.y * axis.y + cos, t * axis.y * axis.z - sin * axis.x],
		[t * axis.x * axis.z - sin * axis.y, t * axis.y * axis.z + sin * axis.x, t * axis.z * axis.z + cos]
	];
}

function cross(a: Vec3, b: Vec3): Vec3 {
	return {
		x: a.y * b.z - a.z * b.y,
		y: a.z * b.x - a.x * b.z,
		z: a.x * b.y - a.y * b.x
	};
}

function dot(a: Vec3, b: Vec3): number {
	return a.x * b.x + a.y * b.y + a.z * b.z;
}

function normalise(v: Vec3): Vec3 {
	const length = Math.hypot(v.x, v.y, v.z);
	return length < 1e-9 ? { x: 1, y: 0, z: 0 } : { x: v.x / length, y: v.y / length, z: v.z / length };
}

export function distance(a: Vec3, b: Vec3): number {
	return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}
