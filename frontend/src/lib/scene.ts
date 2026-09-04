/**
 * The rig as a drawing: where things are, once every truss above them is applied.
 *
 * The browser's half of `crates/pult-schema/src/types/scene.rs`, and it exists for
 * the reason `selection.ts` exists beside `group.rs`: dragging a truss re-composes
 * every child's place per frame, which cannot be a round trip. The two are held
 * together by `testdata/transforms.json`, which both test suites read. A new rule
 * about composing transforms needs a case there or it is only half implemented.
 *
 * Angles are XYZ Euler degrees — three.js's default order, and so the console's.
 * A negative scale is a reflection: a fifth of the trusses in a real drawing are
 * mirrored, and no rotation is one. Anything drawing a mirrored object needs a
 * two-sided material, because negative scale flips its normals.
 */
import type { SceneObject, Transform, Vec3 } from './generated/index.js';

/** At the origin, unturned, at its own size. */
export const IDENTITY: Transform = {
	position: { x: 0, y: 0, z: 0 },
	rotation: { x: 0, y: 0, z: 0 },
	scale: { x: 1, y: 1, z: 1 }
};

/** How deep a parent chain may go before this stops walking it. */
export const MAX_DEPTH = 64;

/** At a point, unturned. */
export function at(position: Vec3): Transform {
	return { ...IDENTITY, position };
}

/**
 * Where something actually is, with every parent's placement applied.
 *
 * Anything naming a parent that is not in `objects` is treated as having none: a
 * truss somebody deleted should leave its lights where they were rather than move
 * them to the origin.
 */
export function worldTransform(
	local: Transform,
	parent: string | null,
	objects: Map<string, SceneObject>
): Transform {
	let matrix = toMatrix(local);
	let next = parent;
	for (let seen = 0; next && seen < MAX_DEPTH; seen++) {
		const object = objects.get(next);
		if (!object) break;
		matrix = multiply(toMatrix(object.transform), matrix);
		next = object.parent;
	}
	return fromMatrix(matrix);
}

/** The objects a parent chain can walk, keyed by id. */
export function byId(objects: SceneObject[]): Map<string, SceneObject> {
	return new Map(objects.map((object) => [object.id, object]));
}

/**
 * The placement that undoes this one.
 *
 * Worked out through the matrix rather than by negating the three fields, because
 * negating them is only right when there is no rotation: the inverse of "moved two
 * metres and then turned" is "turned back and then moved two metres in the turned
 * frame", and −position is neither half of that.
 */
export function inverse(transform: Transform): Transform {
	return fromMatrix(invert(toMatrix(transform)));
}

/**
 * Where something has to be written down, given where it has got to and where its
 * parent is.
 *
 * The other direction from `worldTransform`, and the reason it exists: a gizmo hands
 * back a world placement and a `SceneObject.transform` is relative to its parent, so
 * every drag in the editor crosses this seam. `parentWorld` is the parent's
 * *composed* placement — what `worldTransform` answers for it.
 */
export function localOf(world: Transform, parentWorld: Transform | null): Transform {
	if (!parentWorld) return world;
	return fromMatrix(multiply(invert(toMatrix(parentWorld)), toMatrix(world)));
}

/** The parent chain of an object, composed — or `null` where it hangs off nothing. */
export function parentWorld(
	parent: string | null,
	objects: Map<string, SceneObject>
): Transform | null {
	if (!parent) return null;
	const object = objects.get(parent);
	if (!object) return null;
	return worldTransform(object.transform, object.parent, objects);
}

/**
 * The direction a transform points a fixture's own down axis.
 *
 * Negative zero is turned back into zero, and that is not tidiness: a fixture hanging
 * straight down comes out as `{x: -0, y: -1, z: -0}`, and `Math.atan2(0, -0)` is π —
 * so a bearing taken off it is 180° out, and every beam in the rig points upstage.
 */
export function facing(transform: Transform): Vec3 {
	const basis = eulerToBasis(transform.rotation);
	const zeroed = (n: number) => (n === 0 ? 0 : n);
	return {
		x: zeroed(-basis[0][1]),
		y: zeroed(-basis[1][1]),
		z: zeroed(-basis[2][1])
	};
}

/**
 * A transform that points a fixture's own down axis along a direction.
 *
 * Yaw and pitch and no roll, which is what aiming a light at something means: there
 * is nothing in "point it over there" that says which way up it is.
 */
export function facingTransform(position: Vec3, direction: Vec3): Transform {
	const length = Math.hypot(direction.x, direction.y, direction.z);
	if (length < 1e-6) return at(position);
	const d = { x: direction.x / length, y: direction.y / length, z: direction.z / length };

	// Tip away from straight down, then turn. That order is not expressible as two of
	// the three XYZ angles — XYZ applies the tip first, and a light aimed sideways
	// then loses its turn entirely — so the basis is built and decomposed instead.
	const pitch = Math.acos(Math.min(1, Math.max(-1, -d.y)));
	// Straight down and straight up have no bearing to speak of, and asking for one
	// gives `Math.atan2(-0, -0)`, which is −π: a hanging light stored as turned all
	// the way round, and the epsilons that come back out of the angles then read as a
	// bearing of 45°.
	const horizontal = Math.hypot(d.x, d.z);
	const yaw = horizontal < 1e-9 ? 0 : Math.atan2(-d.x, -d.z);
	const [sp, cp] = [Math.sin(pitch), Math.cos(pitch)];
	const [sy, cy] = [Math.sin(yaw), Math.cos(yaw)];
	const basis = [
		[cy, sy * sp, sy * cp],
		[0, cp, -sp],
		[-sy, cy * sp, cy * cp]
	];
	return { ...IDENTITY, position, rotation: basisToEuler(basis) };
}

// ── The arithmetic ────────────────────────────────────────────────────────────

/** A transform as a 4x4, column-vector convention: `world = M · local`. */
type Matrix4 = number[][];

function toMatrix(transform: Transform): Matrix4 {
	const basis = eulerToBasis(transform.rotation);
	const scale = [transform.scale.x, transform.scale.y, transform.scale.z];
	const out = [
		[0, 0, 0, transform.position.x],
		[0, 0, 0, transform.position.y],
		[0, 0, 0, transform.position.z],
		[0, 0, 0, 1]
	];
	for (let row = 0; row < 3; row++) {
		for (let col = 0; col < 3; col++) out[row][col] = basis[row][col] * scale[col];
	}
	return out;
}

function fromMatrix(matrix: Matrix4): Transform {
	const basis = [0, 1, 2].map((row) => [0, 1, 2].map((col) => matrix[row][col]));

	const scale = [0, 1, 2].map((axis) => {
		const length = Math.hypot(basis[0][axis], basis[1][axis], basis[2][axis]);
		return length < 1e-6 ? 1 : length;
	});
	if (determinant(basis) < 0) scale[0] = -scale[0];
	for (let row = 0; row < 3; row++) {
		for (let axis = 0; axis < 3; axis++) basis[row][axis] /= scale[axis];
	}

	return {
		position: { x: matrix[0][3], y: matrix[1][3], z: matrix[2][3] },
		rotation: basisToEuler(basis),
		scale: { x: scale[0], y: scale[1], z: scale[2] }
	};
}

function multiply(a: Matrix4, b: Matrix4): Matrix4 {
	return [0, 1, 2, 3].map((i) =>
		[0, 1, 2, 3].map((j) => [0, 1, 2, 3].reduce((sum, k) => sum + a[i][k] * b[k][j], 0))
	);
}

/**
 * The inverse of an affine 4x4 whose bottom row is `0 0 0 1`.
 *
 * The basis is inverted properly rather than transposed: a transform may carry a
 * non-uniform or negative scale, and a transpose is the inverse of a rotation only. A
 * basis that cannot be inverted — a scale of zero on some axis — gives the identity
 * back, which leaves a thing where it was rather than sending it to infinity.
 */
function invert(matrix: Matrix4): Matrix4 {
	const m = [0, 1, 2].map((row) => [0, 1, 2].map((col) => matrix[row][col]));
	const det = determinant(m);
	const identity = [
		[1, 0, 0, 0],
		[0, 1, 0, 0],
		[0, 0, 1, 0],
		[0, 0, 0, 1]
	];
	if (Math.abs(det) < 1e-12) return identity;

	// The adjugate over the determinant.
	const cofactor = [
		[
			m[1][1] * m[2][2] - m[1][2] * m[2][1],
			m[0][2] * m[2][1] - m[0][1] * m[2][2],
			m[0][1] * m[1][2] - m[0][2] * m[1][1]
		],
		[
			m[1][2] * m[2][0] - m[1][0] * m[2][2],
			m[0][0] * m[2][2] - m[0][2] * m[2][0],
			m[0][2] * m[1][0] - m[0][0] * m[1][2]
		],
		[
			m[1][0] * m[2][1] - m[1][1] * m[2][0],
			m[0][1] * m[2][0] - m[0][0] * m[2][1],
			m[0][0] * m[1][1] - m[0][1] * m[1][0]
		]
	];

	const out = identity.map((row) => [...row]);
	for (let row = 0; row < 3; row++) {
		for (let col = 0; col < 3; col++) out[row][col] = cofactor[row][col] / det;
	}
	// And the translation, taken back through the inverted basis.
	const t = [matrix[0][3], matrix[1][3], matrix[2][3]];
	for (let row = 0; row < 3; row++) {
		out[row][3] = -[0, 1, 2].reduce((sum, k) => sum + out[row][k] * t[k], 0);
	}
	return out;
}

function determinant(m: number[][]): number {
	return (
		m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) -
		m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) +
		m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
	);
}

/** Three degrees to a rotation matrix, XYZ order: `R = Rx · Ry · Rz`. */
export function eulerToBasis(euler: Vec3): number[][] {
	const rad = Math.PI / 180;
	const [sx, cx] = [Math.sin(euler.x * rad), Math.cos(euler.x * rad)];
	const [sy, cy] = [Math.sin(euler.y * rad), Math.cos(euler.y * rad)];
	const [sz, cz] = [Math.sin(euler.z * rad), Math.cos(euler.z * rad)];
	return [
		[cy * cz, -cy * sz, sy],
		[cx * sz + sx * sy * cz, cx * cz - sx * sy * sz, -sx * cy],
		[sx * sz - cx * sy * cz, sx * cz + cx * sy * sz, cx * cy]
	];
}

/** And back. */
export function basisToEuler(basis: number[][]): Vec3 {
	const deg = 180 / Math.PI;
	const sy = Math.min(1, Math.max(-1, basis[0][2]));
	const y = Math.asin(sy);
	// Gimbal lock: roll and yaw turn about the same axis, so it all goes in x.
	const [x, z] =
		Math.abs(sy) < 0.999999
			? [Math.atan2(-basis[1][2], basis[2][2]), Math.atan2(-basis[0][1], basis[0][0])]
			: [Math.atan2(basis[2][1], basis[1][1]), 0];
	return { x: x * deg, y: y * deg, z: z * deg };
}
