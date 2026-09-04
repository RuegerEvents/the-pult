/**
 * What a light is clamped to.
 *
 * The browser's half of `crates/pult-schema/src/types/mount.rs`, and it exists for a
 * sharper reason than `scene.ts` does: the browser is the **writer**. Resolving a
 * mount on a truss that came out of a drawing means knowing where that truss's chords
 * are, and those come off the mesh's own bounds — which only this side ever measures,
 * because the station never loads a mesh. So a station's own arithmetic here is only
 * ever checking work this file did, and `testdata/mounts.json` is what holds the two
 * to each other.
 *
 * A mount is two degrees: which chord, how far along it, how far round. That is what
 * a hook clamp has, and it is why a mounted fixture gets two handles rather than the
 * six a free placement would need — every one of which would take the light off the
 * truss.
 */

import type { Mount, StockPiece, Transform, Vec3 } from './generated/index.js';
import { IDENTITY } from './scene.js';

/**
 * How far below the chord a clamped fixture's body sits, in metres.
 *
 * A hook clamp plus the top of a body: 205 mm. On an F34, whose chords are 145 mm
 * either side of the centre line, that puts a hung lantern 350 mm below the bar.
 */
export const HUNG_BELOW = 0.205;

/**
 * A line a clamp can go round, in the piece's own frame.
 *
 * Always along **+X**, which is the axis every straight piece in the catalogue runs
 * along; `at` is where the line crosses `x = 0`, so `at.x` is always nought.
 */
export type Chord = { at: Vec3 };

/** A mount with nothing said: hung under the first chord, at the piece's middle. */
export const HANGING: Mount = { chord: 0, along: 0, roll: 0 };

/** The chord a mount names, or a line through the piece's own origin. */
export function chordOf(mount: Mount, chords: Chord[]): Chord {
	if (chords.length === 0) return { at: { x: 0, y: 0, z: 0 } };
	// Out of range wraps rather than refusing: a piece that lost a chord between two
	// versions of this console should leave the light on the truss.
	return chords[mount.chord % chords.length];
}

/** Where a fixture clamped here sits, in the parent piece's own frame. */
export function mountPoint(mount: Mount, chords: Chord[]): Vec3 {
	const chord = chordOf(mount, chords);
	const radians = (mount.roll * Math.PI) / 180;
	return {
		x: chord.at.x + mount.along,
		y: chord.at.y - HUNG_BELOW * Math.cos(radians),
		z: chord.at.z - HUNG_BELOW * Math.sin(radians)
	};
}

/**
 * The whole placement: where it sits, and which way up.
 *
 * The rotation is the roll and nothing else, so a fixture nobody has aimed hangs
 * looking at the floor — a fixture's own axis is −Y, so zero rotation *is* hanging.
 */
export function mountTransform(mount: Mount, chords: Chord[]): Transform {
	return {
		...IDENTITY,
		position: mountPoint(mount, chords),
		rotation: { x: mount.roll, y: 0, z: 0 }
	};
}

/**
 * The nearest place on this piece to a point in its own frame, and how far off it is.
 *
 * What snapping a dragged light onto a bar comes to. The distance is what decides
 * whether it is a clamp at all: past the radius the light is simply somewhere, and
 * dragging it away is how a fixture stops being mounted.
 *
 * `Infinity` for a piece with no chords, which is not a distance but an answer: there
 * is nothing here to clamp to.
 */
export function nearestMount(point: Vec3, chords: Chord[]): { mount: Mount; distance: number } {
	if (chords.length === 0) return { mount: { ...HANGING }, distance: Infinity };

	let best = { mount: { ...HANGING }, distance: Infinity };
	chords.forEach((chord, index) => {
		const mount: Mount = {
			chord: index,
			along: point.x - chord.at.x,
			roll: quarterTurn(point.y - chord.at.y, point.z - chord.at.z)
		};
		const landed = mountPoint(mount, chords);
		const distance = Math.hypot(landed.x - point.x, landed.y - point.y, landed.z - point.z);
		if (distance < best.distance) best = { mount, distance };
	});
	return best;
}

/**
 * Which of the four quarter turns points nearest at `(dy, dz)` from the chord.
 *
 * Zero hangs — the offset a roll of zero gives is straight down — and the turns go
 * round towards +Z, which is the direction `mountPoint` rolls in.
 */
function quarterTurn(dy: number, dz: number): number {
	const angle = (Math.atan2(-dz, -dy) * 180) / Math.PI;
	const snapped = Math.round(angle / 90) * 90;
	// Into 0..360, so a stored roll never reads as −90 where 270 was meant.
	return ((snapped % 360) + 360) % 360;
}

/**
 * The chords of whatever a light is being dragged onto.
 *
 * A catalogue piece declares its own. Anything else — a truss out of somebody's
 * drawing — gets **one**, worked out from the mesh's bounds: a line along the long
 * axis at the bottom face. Which is a guess, and deliberately the smallest one
 * available: the console does not know whether that mesh is a box truss or a ladder,
 * and offering four chords it invented would put lights on corners that are not there.
 */
export function chordsFor(piece: StockPiece | undefined, meshSize: Vec3 | null): Chord[] {
	if (piece) return piece.chords;
	if (!meshSize) return [];
	// The long axis is the run; the bottom face is where a clamp goes. A mesh is
	// measured about its own origin, which for every drawing this console has read is
	// its middle, so half the height down is the underside.
	return [{ at: { x: 0, y: -meshSize.y / 2, z: 0 } }];
}
