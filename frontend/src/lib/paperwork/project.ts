/**
 * The rig, flattened onto paper at a stated scale.
 *
 * This is what makes a drafting viewport a *drawing* rather than a screenshot: every
 * point goes through one orthographic projection whose only free parameter is the scale
 * denominator, so a metre in the rig is exactly `1000/denominator` millimetres on the
 * page, everywhere on the page. That sentence is the whole contract, and
 * {@link project.test.ts} asserts it directly rather than by looking at a picture.
 *
 * # Why there is no perspective here
 *
 * There is no such thing as a scaled perspective. A drafting viewport is orthographic
 * by construction; a viewport that wants perspective is a *picture* and goes through
 * `raster.ts` and the rig renderer instead, and prints NTS. The two are different code
 * paths because they are different claims.
 *
 * # Filled silhouettes, painted far to near
 *
 * Hidden-line removal proper is a visibility computation per edge against every
 * triangle in the scene, which on a rig of ninety truss sections is not a thing to do
 * in a preview loop. What this does instead is take each object's projected *convex
 * outline*, fill it white, outline it black, and paint the objects back to front. For
 * the convex pieces a rig is mostly made of — truss sections, decks, lantern bodies —
 * it is exact. Where two objects interpenetrate it is wrong, and that is written down
 * rather than hidden: the painter's algorithm has no answer for a truss run through a
 * wall, and neither has this.
 */

import type { Vec3 } from '../generated/index.js';
import type { Mm, PathItem, Pt, RectMm } from './drawing.js';
import { BLACK, WEIGHT, WHITE, stroke } from './drawing.js';

/** How a viewport looks at the rig. One of the schema's `ViewPreset`s, resolved. */
export type Axis = 'plan' | 'front' | 'section' | 'quarter';

/** A camera basis: three unit vectors, right / up / towards the viewer. */
export interface Basis {
	right: Vec3;
	up: Vec3;
	out: Vec3;
}

const V = (x: number, y: number, z: number): Vec3 => ({ x, y, z });

/**
 * Where each view stands, in world axes — the rig is Y-up, X across, Z towards the
 * house.
 *
 * Two of these carry a decision the rig view already made and this inherits rather
 * than re-deciding. **Plan puts upstage at the top of the page**, which is how every
 * lighting plan is drawn and is why its `up` is −Z rather than +Z. And **section looks
 * from stage left**, so the stage reads on the left of the frame the way a section is
 * drawn on paper.
 */
export function basisFor(axis: Axis): Basis {
	switch (axis) {
		case 'plan':
			return { right: V(1, 0, 0), up: V(0, 0, -1), out: V(0, 1, 0) };
		case 'front':
			return { right: V(1, 0, 0), up: V(0, 1, 0), out: V(0, 0, 1) };
		case 'section':
			return { right: V(0, 0, 1), up: V(0, 1, 0), out: V(-1, 0, 0) };
		case 'quarter': {
			// An axonometric: turned 45° about the vertical and tipped down 30°, which
			// is the isometric-ish view every drawing set has one of. Orthographic, so
			// it still has a scale — measurable along the three axes and nowhere else,
			// which is true of every axonometric ever drawn.
			const c = Math.SQRT1_2;
			const tilt = Math.sin(Math.PI / 6);
			const lift = Math.cos(Math.PI / 6);
			return {
				right: V(c, 0, -c),
				up: V(-c * tilt, lift, -c * tilt),
				out: V(c * lift, tilt, c * lift)
			};
		}
	}
}

function dot(a: Vec3, b: Vec3): number {
	return a.x * b.x + a.y * b.y + a.z * b.z;
}

/**
 * Millimetres on the page per metre in the rig.
 *
 * `1:50` is 20 mm/m. The one conversion in the feature, and every scaled figure — the
 * drawing, the scale bar, the size of a plan head — goes through it.
 */
export function mmPerMetre(denominator: number): number {
	return 1000 / Math.max(denominator, 1);
}

/** A viewport's camera: where it looks from, what it is centred on, and how big. */
export interface Camera {
	basis: Basis;
	/** The world point that lands in the middle of the frame. */
	centre: Vec3;
	/** Millimetres on the page per metre in the rig. */
	scale: number;
	/** Where the frame is on the sheet. */
	rect: RectMm;
}

/** A world point, on the page. */
export function toPage(p: Vec3, camera: Camera): Pt {
	const dx = p.x - camera.centre.x;
	const dy = p.y - camera.centre.y;
	const dz = p.z - camera.centre.z;
	const d = V(dx, dy, dz);
	return {
		x: camera.rect.x + camera.rect.w / 2 + dot(d, camera.basis.right) * camera.scale,
		// The page's y runs downwards and the camera's `up` runs upwards, so this is
		// the one sign flip in the projection.
		y: camera.rect.y + camera.rect.h / 2 - dot(d, camera.basis.up) * camera.scale
	};
}

/** How far towards the viewer a point is, for painting order. Larger is nearer. */
export function depthOf(p: Vec3, camera: Camera): number {
	return dot(p, camera.basis.out);
}

/**
 * The denominator a free fit would come to, before it is snapped to a real scale.
 *
 * Given the extent the drawing has to cover, in metres across and up, and the frame it
 * has to cover it in. Whichever direction is tighter decides, which is what makes the
 * same viewport frame a five-fixture demo and a festival.
 */
export function fittedDenominator(
	extent: { across: number; up: number },
	rect: RectMm,
	padding = 1.06
): number {
	const across = (Math.max(extent.across, 0.001) * padding * 1000) / Math.max(rect.w, 1);
	const up = (Math.max(extent.up, 0.001) * padding * 1000) / Math.max(rect.h, 1);
	return Math.max(across, up);
}

/**
 * How wide and how tall a set of world points is, seen from a basis.
 *
 * In the *camera's* axes rather than the world's, because that is what has to fit the
 * frame: a truss run at 30° to the stage is wider on a plan than its extent in X.
 */
export function extentIn(points: Vec3[], basis: Basis): { across: number; up: number; centre: Vec3 } {
	if (points.length === 0) {
		return { across: 1, up: 1, centre: V(0, 0, 0) };
	}
	let minR = Infinity;
	let maxR = -Infinity;
	let minU = Infinity;
	let maxU = -Infinity;
	for (const p of points) {
		const r = dot(p, basis.right);
		const u = dot(p, basis.up);
		if (r < minR) minR = r;
		if (r > maxR) maxR = r;
		if (u < minU) minU = u;
		if (u > maxU) maxU = u;
	}
	const midR = (minR + maxR) / 2;
	const midU = (minU + maxU) / 2;
	// Back into world space: the centre is the point that lands in the middle of the
	// frame, and the two in-plane offsets are all that is known about it — the depth
	// does not matter to an orthographic camera, so it is left at zero.
	const centre = V(
		basis.right.x * midR + basis.up.x * midU,
		basis.right.y * midR + basis.up.y * midU,
		basis.right.z * midR + basis.up.z * midU
	);
	return { across: maxR - minR, up: maxU - minU, centre };
}

// ── Outlines ─────────────────────────────────────────────────────────────────

/**
 * The convex outline of a set of page points, anticlockwise, closed.
 *
 * Andrew's monotone chain. Used for a silhouette: the outline of what an object covers
 * on the page, which for a convex body is exactly its silhouette and for anything else
 * is the tightest honest overstatement of one.
 */
export function hull(points: Pt[]): Pt[] {
	if (points.length < 3) return points.slice();
	const sorted = points.slice().sort((a, b) => a.x - b.x || a.y - b.y);
	const cross = (o: Pt, a: Pt, b: Pt) =>
		(a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);

	const lower: Pt[] = [];
	for (const p of sorted) {
		while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) {
			lower.pop();
		}
		lower.push(p);
	}
	const upper: Pt[] = [];
	for (let i = sorted.length - 1; i >= 0; i--) {
		const p = sorted[i];
		while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) {
			upper.pop();
		}
		upper.push(p);
	}
	lower.pop();
	upper.pop();
	const ring = lower.concat(upper);
	return ring.length >= 3 ? [...ring, ring[0]] : points.slice();
}

/** One thing to draw, with the depth it is painted at. */
export interface Drawable {
	depth: number;
	items: PathItem[];
}

/**
 * A body's silhouette: filled white, outlined, so what is in front covers what is
 * behind when these are painted in depth order.
 */
export function silhouette(pagePoints: Pt[], weight: Mm = WEIGHT.heavy): PathItem {
	return {
		kind: 'path',
		points: [hull(pagePoints)],
		fill: WHITE,
		stroke: stroke(weight, BLACK)
	};
}

/** Every edge of a body, nothing hidden. */
export function wireframe(edges: Array<[Pt, Pt]>, weight: Mm = WEIGHT.thin): PathItem {
	return {
		kind: 'path',
		points: edges.map(([a, b]) => [a, b]),
		stroke: stroke(weight, BLACK)
	};
}

/**
 * Paint a set of drawables far to near.
 *
 * Stable within a depth, so two pieces at the same distance keep the order the rig
 * gave them and a redraw does not shuffle them.
 */
export function painted(drawables: Drawable[]): PathItem[] {
	return drawables
		.map((d, index) => ({ d, index }))
		.sort((a, b) => a.d.depth - b.d.depth || a.index - b.index)
		.flatMap(({ d }) => d.items);
}
