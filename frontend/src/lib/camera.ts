/**
 * Where the rig view looks from.
 *
 * Four questions an operator asks constantly — what does the front look like, what is
 * the plan, what is the section, and let me see it in the round — and one they ask
 * more often than any of them: show me what I have just selected. Each is a place to
 * put the camera, worked out from the rig's own bounding box, so the same button
 * frames a five-fixture demo and a two-hundred-head festival.
 *
 * Pure arithmetic, in metres, with no three.js in it: the panel owns the renderer and
 * two `rig` tiles can be open at once, so a shot is a value one of them is told to
 * take rather than a camera anything here holds.
 */

import type { Fixture, SceneObject, Vec3 } from './generated/index.js';
import { fixturePoint } from './stage.js';
import { worldTransform } from './scene.js';
import { piece, stockSize } from './stock.js';

/** A box in metres, in world axes. */
export type Bounds = { min: Vec3; max: Vec3 };

/** Somewhere to put a camera, and what to point it at. */
export type Shot = { position: [number, number, number]; target: [number, number, number] };

export type ViewPreset = 'front' | 'plan' | 'section' | 'quarter';

/** The lens the rig view uses. Vertical, in degrees, as three.js means it. */
export const FIELD_OF_VIEW = 50;

/**
 * Eye height, because that is where an operator actually stands.
 *
 * The front view is meant to look like the room rather than like a plan drawn in
 * perspective, which is why it is the one preset that does not sit level with what it
 * is looking at.
 */
const EYE_HEIGHT = 1.7;

/** A shade over the exact fit, so nothing is drawn touching the edge of the frame. */
const PADDING = 1.12;

/** What a view with nothing in it frames: a stage-sized box, so the floor has scale. */
const EMPTY: Bounds = { min: { x: -8, y: 0, z: -6 }, max: { x: 8, y: 6, z: 6 } };

export type BoundsOptions = {
	/**
	 * Which pieces of the drawing count towards the box. Everything in `objects` by
	 * default — but the caller that has a *hidden layer* passes what is being drawn,
	 * since framing a truss nobody can see is framing empty air. `objects` itself
	 * stays whole either way: it is what a parent chain is walked through, and a
	 * light hangs where its truss is whether or not the truss is drawn.
	 */
	pieces?: SceneObject[];
	margin?: number;
	/**
	 * Whether the box reaches the deck.
	 *
	 * True for the rig, because a rig hung at six metres over an empty floor is still
	 * a room with a floor in it, and a front view framing only the bar is a picture of
	 * a bar. False for a *selection*: an operator who picked one head at six metres
	 * wants that head, not six metres of air under it.
	 */
	toFloor?: boolean;
};

/**
 * A box holding everything the rig view draws.
 *
 * Fixtures **and** scene objects, because a truss standing where no lantern hangs is
 * still part of what somebody wants in frame.
 *
 * A **catalogue** piece counts as its whole extent, because the table says how long it
 * is — and once a person can build a rig out of nothing but catalogue pieces, an origin
 * is not enough: a nine-metre bar counted as a point is a nine-metre bar with three
 * metres of itself outside the frame. Its longest dimension is used in every direction,
 * which is generous by up to that length on a turned piece and is the honest answer
 * without composing a box through a rotation. Anything else still counts as its origin,
 * which the margin covers: an imported mesh's size is known only once it has loaded, and
 * a frame that jumped when a download finished would be worse than one that is a shade
 * tight.
 */
export function rigBounds(
	fixtures: Fixture[],
	objects: Map<string, SceneObject>,
	{ pieces, margin = 1, toFloor = true }: BoundsOptions = {}
): Bounds {
	const points: Vec3[] = [];
	for (const fixture of fixtures) {
		const at = fixturePoint(fixture, objects);
		if (at) points.push(at);
	}
	for (const object of pieces ?? objects.values()) {
		const at = worldTransform(object.transform, object.parent, objects).position;
		const entry = piece(object.catalogue);
		if (!entry) {
			points.push(at);
			continue;
		}
		const size = stockSize(entry, object.properties);
		const reach = Math.max(size.x, size.y, size.z) / 2;
		points.push({ x: at.x - reach, y: at.y - reach, z: at.z - reach });
		points.push({ x: at.x + reach, y: at.y + reach, z: at.z + reach });
	}
	if (points.length === 0) return EMPTY;

	const axis = (pick: (p: Vec3) => number) => points.map(pick);
	const box = (pick: (p: Vec3) => number) => ({
		min: Math.min(...axis(pick)) - margin,
		max: Math.max(...axis(pick)) + margin
	});
	const x = box((p) => p.x);
	const y = box((p) => p.y);
	const z = box((p) => p.z);
	return {
		min: { x: x.min, y: toFloor ? Math.min(y.min, 0) : y.min, z: z.min },
		max: { x: x.max, y: y.max, z: z.max }
	};
}

const centreOf = (b: Bounds): Vec3 => ({
	x: (b.min.x + b.max.x) / 2,
	y: (b.min.y + b.max.y) / 2,
	z: (b.min.z + b.max.z) / 2
});

const sizeOf = (b: Bounds): Vec3 => ({
	x: b.max.x - b.min.x,
	y: b.max.y - b.min.y,
	z: b.max.z - b.min.z
});

/**
 * How far back a camera has to be for a box that tall and that wide to fit.
 *
 * The vertical lens and the horizontal one are different questions — a 21:9 tile has
 * room for a wide rig that a portrait tablet has not — so both are worked out and the
 * one that needs more distance wins. Which is what makes these presets frame the same
 * rig on a phone and on a projector.
 */
export function fitDistance(
	halfWidth: number,
	halfHeight: number,
	aspect: number,
	fov = FIELD_OF_VIEW
): number {
	const half = (fov * Math.PI) / 360;
	const vertical = halfHeight / Math.tan(half);
	// The horizontal half-angle of a perspective camera is not `fov * aspect`; it is
	// the angle whose tangent is `tan(fov/2) * aspect`, which is the same thing said
	// the way that stays right at wide angles.
	const horizontal = halfWidth / (Math.tan(half) * Math.max(aspect, 0.1));
	return Math.max(vertical, horizontal, 1) * PADDING;
}

/** Half the diagonal: what a box needs to fit from a direction that is not an axis. */
const radiusOf = (b: Bounds): number => {
	const s = sizeOf(b);
	return Math.sqrt(s.x * s.x + s.y * s.y + s.z * s.z) / 2;
};

/**
 * One of the four places to stand.
 *
 * `front` is the house, at eye height; `plan` is straight down; `section` looks across
 * the stage from the side it is conventionally drawn from — stage left, so the stage
 * is on the left of the frame and the auditorium on the right; `quarter` is the
 * three-quarter view, off to one side and above, which is the one that reads as a
 * room rather than as a drawing.
 */
export function presetShot(preset: ViewPreset, bounds: Bounds, aspect = 16 / 9): Shot {
	const c = centreOf(bounds);
	const s = sizeOf(bounds);
	const target: [number, number, number] = [c.x, c.y, c.z];

	switch (preset) {
		case 'front': {
			const back = fitDistance(s.x / 2, s.y / 2, aspect);
			// From the far face rather than the centre, so the downstage truss is
			// inside the frame and not on the glass.
			return { position: [c.x, EYE_HEIGHT, bounds.max.z + back], target };
		}
		case 'plan': {
			const up = fitDistance(s.x / 2, s.z / 2, aspect);
			// Not *exactly* overhead: straight down leaves the camera's up vector with
			// nothing to resolve it, and the view rolls to whatever the maths picks.
			// A couple of degrees off is a plan an operator cannot tell from one.
			return { position: [c.x, bounds.max.y + up, c.z + up * 0.04], target };
		}
		case 'section': {
			const across = fitDistance(s.z / 2, s.y / 2, aspect);
			return { position: [bounds.min.x - across, c.y, c.z], target };
		}
		case 'quarter': {
			// A direction rather than a face, so the fit is the box's own radius: no
			// pair of axes describes what is on screen from here.
			const away = fitDistance(radiusOf(bounds), radiusOf(bounds), aspect);
			const dir = { x: 0.62, y: 0.45, z: 0.65 };
			const length = Math.hypot(dir.x, dir.y, dir.z);
			return {
				position: [
					c.x + (dir.x / length) * away,
					// Above the rig looking down at it, and never underneath it.
					Math.max(c.y + (dir.y / length) * away, EYE_HEIGHT),
					c.z + (dir.z / length) * away
				],
				target
			};
		}
	}
}

/**
 * Frame a box from where the camera already is.
 *
 * Which is what focusing on a selection has to do: an operator picking three heads
 * wants to see those three heads, and a button that also swung round to the front of
 * them would have thrown away the angle they were working at. So only the distance
 * changes — the direction is the one they are already looking from.
 */
export function focusShot(bounds: Bounds, from: Vec3, aspect = 16 / 9): Shot {
	const c = centreOf(bounds);
	const radius = Math.max(radiusOf(bounds), 0.75);
	const away = fitDistance(radius, radius, aspect);

	let back = { x: from.x - c.x, y: from.y - c.y, z: from.z - c.z };
	const length = Math.hypot(back.x, back.y, back.z);
	// Standing exactly where the thing is means there is no direction to keep, so the
	// view falls back to looking at it from the house.
	if (length < 1e-6) back = { x: 0, y: 0.35, z: 1 };
	const unit = Math.hypot(back.x, back.y, back.z);
	return {
		position: [
			c.x + (back.x / unit) * away,
			Math.max(c.y + (back.y / unit) * away, 0.3),
			c.z + (back.z / unit) * away
		],
		target: [c.x, c.y, c.z]
	};
}

/**
 * How much of the world an orthographic camera at this distance shows.
 *
 * The perspective presets work out a *distance* from the lens angle; an ortho camera
 * has no lens angle, so what it needs instead is a frame. Same box, same padding, same
 * two-questions rule — the tile's own shape decides whether the width or the height is
 * the one that has to fit.
 *
 * The camera still stands where the preset put it, because the distance is what
 * `camera-controls` orbits about and what a near/far plane is measured from. It just
 * stops being what decides how much is on screen.
 */
export function orthoFrame(
	bounds: Bounds,
	aspect: number,
	preset: ViewPreset = 'quarter'
): { halfWidth: number; halfHeight: number } {
	const s = sizeOf(bounds);
	// Which two axes are across the frame depends on where the camera is standing. A
	// plan sees the floor, a section sees the depth, and a three-quarter view sees no
	// pair of axes at all — so that one gets the box's own radius, the same answer
	// `presetShot` gives it.
	const across = { front: s.x, plan: s.x, section: s.z, quarter: radiusOf(bounds) * 2 };
	const up = { front: s.y, plan: s.z, section: s.y, quarter: radiusOf(bounds) * 2 };
	const halfWidth = Math.max(across[preset] / 2, 0.5) * PADDING;
	const halfHeight = Math.max(up[preset] / 2, 0.5) * PADDING;
	// Whichever needs more room wins, so the same button frames a rig on a phone and
	// on a projector — the rule `fitDistance` follows for the other camera.
	const wanted = Math.max(halfWidth, halfHeight / Math.max(aspect, 0.1));
	return { halfWidth: wanted, halfHeight: wanted / Math.max(aspect, 0.1) };
}

/**
 * Which projection a preset opens in.
 *
 * A plan and a section are drawings and are read straight on: parallel lines stay
 * parallel and a metre is a metre wherever it is on the page, which is the whole
 * reason anybody draws them. The front and the three-quarter are pictures of a room.
 * It is a *default*, not a rule — the toggle is beside the presets and stays wherever
 * somebody puts it.
 */
export function projectionFor(preset: ViewPreset): 'perspective' | 'ortho' {
	return preset === 'plan' || preset === 'section' ? 'ortho' : 'perspective';
}

/** What each preset is called, and what it answers. */
export const VIEW_PRESETS: { value: ViewPreset; label: string; blurb: string }[] = [
	{ value: 'front', label: 'Front', blurb: 'From the house, at eye height.' },
	{ value: 'plan', label: 'Plan', blurb: 'Straight down, the way a plan is drawn. Opens flat.' },
	{ value: 'section', label: 'Section', blurb: 'Across the stage from the side. Opens flat.' },
	{ value: 'quarter', label: '¾', blurb: 'Off to one side and above.' }
];
