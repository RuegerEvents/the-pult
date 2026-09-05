/**
 * How far along the bar each light goes.
 *
 * A plan says where a fixture *is*; it does not say how far to slide it along the truss,
 * and that is the only question anybody up a ladder is actually asking. So a drafting
 * viewport can carry a chain of running dimensions per bar — a witness line down from
 * each head, a gap between one head and the next, and the distance from whichever end
 * the crew is measuring from.
 *
 * # Along the bar, not across the page
 *
 * The measurement is in the *parent's* own frame and in metres, taken from the fixture's
 * local position along the piece's long axis. A truss run at forty degrees to the stage
 * is still measured along itself, which is how a tape works; measuring the projected
 * distance on the page would give a number that shrinks as the truss turns and is right
 * only for a bar that happens to lie square to the drawing.
 *
 * The *drawing* of the chain is then projected like everything else, so it sits under
 * the bar it belongs to whichever way that bar is turned.
 */

import type { Fixture, SceneObject, Vec3 } from '../generated/index.js';
import { BLACK, WEIGHT, stroke, type DrawItem, type Mm, type Pt } from './drawing.js';
import type { Metrics } from './font.js';
import { toPage, type Camera } from './project.js';

/** Which end a crew measures from. Mirrors the schema's `Datum`. */
export type Datum = 'Left' | 'Right' | 'Centre';

export interface DimensionOptions {
	datum: Datum;
	/** Print the distance from the datum as well as the gap to the next head. */
	running: boolean;
	/** Draw the chain above the bar rather than below it. */
	above: boolean;
}

/** One head on one bar, with how far along it sits. */
interface Along {
	fixture: Fixture;
	/** Metres along the parent's own long axis, from its centre. */
	at: number;
	/** Where it lands on the page. */
	page: Pt;
}

/**
 * How far along its parent a fixture sits, and how long the parent is.
 *
 * The long axis is X in a piece's own frame — a truss's length, a bar's run — which is
 * the convention the whole catalogue is built on: `StockPiece::size` is "along X, up Y,
 * along Z", and a 3 m truss is 3 m of X.
 */
function alongOf(fixture: Fixture): number | null {
	// The *local* position, which for a clamped light is exactly where on the bar it is.
	// `mount.along` says the same thing, and is not used here so that a light placed by
	// hand on a piece it is not clamped to is dimensioned too.
	const local = fixture.position?.position as Vec3 | undefined;
	return local ? local.x : null;
}

/** Metres of bar, from its own size. */
export function barLength(size: Vec3 | undefined): number {
	return size ? Math.max(size.x, 0.001) : 0;
}

/** Where the tape's zero is, in the parent's own frame. */
export function zeroOf(datum: Datum, length: number): number {
	if (datum === 'Centre') return 0;
	return datum === 'Left' ? -length / 2 : length / 2;
}

/**
 * How to write every distance on one bar.
 *
 * **The unit is a property of the chain, not of the figure.** Deciding per figure is
 * the obvious thing and it produced a running chain reading "8500, 9500, 10.50 m,
 * 11.50 m" — a rigger reading that has to convert half way along a bar, which is
 * exactly the kind of small ambiguity a drawing exists to remove. So the longest
 * distance in the chain decides, and everything on that bar is written the same way.
 *
 * Millimetres under ten metres, because that is what a tape reads and what somebody
 * repeats back; metres above it, because "12500" is a number people miscount.
 */
export function unitFor(longest: number): (value: number) => string {
	if (Math.abs(longest) < 10) return (value) => `${Math.round(Math.abs(value) * 1000)}`;
	return (value) => `${Math.abs(value).toFixed(2)} m`;
}

/**
 * The chain for one bar.
 *
 * `parentWorld` and `size` describe the piece; `fixtures` are the heads on it. Returns
 * nothing at all for a bar with fewer than two heads *and* no datum worth stating —
 * one light on a bar still gets its distance from the end, which is the thing somebody
 * has to know to hang it.
 */
export function chainFor(
	fixtures: Fixture[],
	size: Vec3 | undefined,
	toWorld: (local: Vec3) => Vec3,
	camera: Camera,
	options: DimensionOptions,
	metrics: Metrics
): DrawItem[] {
	const length = barLength(size);
	if (length <= 0) return [];

	const heads: Along[] = [];
	for (const fixture of fixtures) {
		const at = alongOf(fixture);
		if (at === null) continue;
		heads.push({ fixture, at, page: toPage(toWorld({ x: at, y: 0, z: 0 }), camera) });
	}
	if (heads.length === 0) return [];
	heads.sort((a, b) => a.at - b.at);

	// The bar's own two ends, projected, which give the chain its direction and the
	// perpendicular to offset it along.
	const from = toPage(toWorld({ x: -length / 2, y: 0, z: 0 }), camera);
	const to = toPage(toWorld({ x: length / 2, y: 0, z: 0 }), camera);
	const dx = to.x - from.x;
	const dy = to.y - from.y;
	const drawn = Math.hypot(dx, dy);
	// A bar seen end-on projects to a point, and a chain along it would be a pile of
	// numbers on one spot. Nothing is drawn, which is the honest answer: this view
	// cannot show that measurement.
	if (drawn < 1) return [];

	const ux = dx / drawn;
	const uy = dy / drawn;
	// Perpendicular, on the side the option asks for. Below by default, because labels
	// go above a head and the de-collision only knows about labels.
	const side = options.above ? -1 : 1;
	const nx = -uy * side;
	const ny = ux * side;

	const OFFSET: Mm = 5.5;
	const WITNESS: Mm = 1.6;
	const TEXT: Mm = 1.7;

	const shift = (p: Pt, by: Mm): Pt => ({ x: p.x + nx * by, y: p.y + ny * by });
	const items: DrawItem[] = [];

	// Witness lines: from just clear of each head down to the dimension line.
	for (const head of heads) {
		items.push({
			kind: 'path',
			points: [[shift(head.page, WITNESS), shift(head.page, OFFSET + 1)]],
			stroke: stroke(WEIGHT.fine, BLACK)
		});
	}

	const zero = zeroOf(options.datum, length);
	// One unit for the whole bar — the gaps and the running chain both — decided by the
	// longest figure either of them will print.
	const write = unitFor(
		Math.max(...heads.map((head) => Math.abs(head.at - zero)), length / 2)
	);
	/** One dimension: a line with a tick at each end and its figure over the middle. */
	const dimension = (a: Pt, b: Pt, text: string, at: Mm) => {
		const start = shift(a, at);
		const end = shift(b, at);
		if (Math.hypot(end.x - start.x, end.y - start.y) < 0.5) return;
		items.push({
			kind: 'path',
			points: [
				[start, end],
				// Ticks: a short stroke at 45°, which is how a drawing terminates a
				// dimension without an arrowhead nobody can see at 1.7 mm.
				[
					{ x: start.x - (ux + nx) * 0.8, y: start.y - (uy + ny) * 0.8 },
					{ x: start.x + (ux + nx) * 0.8, y: start.y + (uy + ny) * 0.8 }
				],
				[
					{ x: end.x - (ux + nx) * 0.8, y: end.y - (uy + ny) * 0.8 },
					{ x: end.x + (ux + nx) * 0.8, y: end.y + (uy + ny) * 0.8 }
				]
			],
			stroke: stroke(WEIGHT.fine, BLACK)
		});
		const middle = { x: (start.x + end.x) / 2, y: (start.y + end.y) / 2 };
		// Above its own line, on the far side from the bar, so a figure never sits on
		// the line it measures.
		const label = shift(middle, side > 0 ? 1.9 : -0.6);
		// Turned to run along the dimension, the way every drawing does it — but never
		// upside down: text past vertical is read by turning the sheet round, and
		// nobody turns an A3 round to read one number.
		let angle = (Math.atan2(uy, ux) * 180) / Math.PI;
		// Folded into [-90, 90] rather than turned by 180 in both directions: adding to
		// an angle already at 180 gives 360, which draws the same and is a figure nobody
		// reading this back can check against "never upside down".
		if (angle > 90) angle -= 180;
		else if (angle < -90) angle += 180;
		items.push({
			kind: 'text',
			at: label,
			text,
			size: TEXT,
			anchor: 'middle',
			baseline: 'baseline',
			rotate: angle
		});
	};

	// The gaps, on the near line: what a crew spaces by.
	for (let i = 0; i + 1 < heads.length; i++) {
		dimension(
			heads[i].page,
			heads[i + 1].page,
			write(heads[i + 1].at - heads[i].at),
			OFFSET
		);
	}

	// And the running distances from the datum, on a second line below.
	//
	// Both, because they fail differently: a chain of gaps alone accumulates a crew's
	// rounding along the bar, and a set of distances alone makes somebody subtract to
	// find a spacing.
	//
	// Drawn as a **running chain** and not as twelve separate dimensions from the
	// datum. That was the obvious thing and it puts twelve lines on top of one another
	// with their figures piled at the left-hand end, which is how the first version of
	// this came out: unreadable, and unreadable in a way that looks like a rendering
	// fault rather than a drawing convention. A running chain is one line with a tick
	// at each head and the figure written *at its own tick*.
	if (options.running) {
		const RUN: Mm = OFFSET + 5.5;
		const anchor = toPage(toWorld({ x: zero, y: 0, z: 0 }), camera);
		const far = heads.reduce(
			(worst, head) => (Math.abs(head.at - zero) > Math.abs(worst.at - zero) ? head : worst),
			heads[0]
		);
		const start = shift(anchor, RUN);
		const end = shift(far.page, RUN);
		items.push({
			kind: 'path',
			points: [
				[start, end],
				// The datum's own mark: a circle is the drawing convention for the
				// origin of a running chain, and a tick would read as one more head.
				[
					{ x: start.x - nx * 1.2, y: start.y - ny * 1.2 },
					{ x: start.x + nx * 1.2, y: start.y + ny * 1.2 }
				]
			],
			stroke: stroke(WEIGHT.fine, BLACK)
		});

		let angle = (Math.atan2(uy, ux) * 180) / Math.PI;
		if (angle > 90) angle -= 180;
		else if (angle < -90) angle += 180;

		for (const head of heads) {
			const tick = shift(head.page, RUN);
			items.push({
				kind: 'path',
				points: [[shift(head.page, RUN - 1.2), tick]],
				stroke: stroke(WEIGHT.fine, BLACK)
			});
			// Written along the chain and clear of it, at the head it belongs to.
			items.push({
				kind: 'text',
				at: shift(tick, side > 0 ? 2.0 : -0.6),
				text: write(head.at - zero),
				size: TEXT,
				anchor: 'middle',
				baseline: 'baseline',
				rotate: angle
			});
		}
	}
	void metrics;
	return items;
}
