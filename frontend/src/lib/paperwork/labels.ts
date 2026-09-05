/**
 * Fixture labels that can actually be read.
 *
 * The drawing this feature was designed against prints, on its own Fixtures sheet,
 * four labels reading "HydraPanel 1" to "HydraPanel 4" exactly on top of one another,
 * and two "Auro Spot Z300" labels likewise. That is not a fault of the person who drew
 * it — it is what every plan does when the heads are closer together than their names
 * are wide, and it is the one place this console can do better than the tool the rig
 * was drawn in.
 *
 * So: labels are laid out, pushed apart where they collide, and given a leader line
 * back to the head when they have moved far enough that the reader could not otherwise
 * tell which light is which.
 *
 * # Deterministic, because the preview is a promise
 *
 * No randomness and no time-dependence anywhere in here. The relaxation runs a fixed
 * number of passes in a fixed order over a fixed input, so the same rig produces the
 * same sheet — which it has to, because the SVG somebody approves and the PDF they
 * send are two renderings of one layout and a label that settled differently in each
 * would make the preview a lie.
 */

import type { Mm, PathItem, Pt, RectMm, TextItem } from './drawing.js';
import { BLACK, WEIGHT, stroke } from './drawing.js';
import type { Metrics } from './font.js';

/** A head that wants a label. */
export interface LabelRequest {
	/** Where the fixture is on the page. */
	anchor: Pt;
	/** The lines of the label, top to bottom. Empty lines are dropped. */
	lines: string[];
}

export interface LabelOptions {
	/** Em size in millimetres. 1.8 mm is about 5 pt — a plan's small text. */
	size?: Mm;
	/** Line spacing as a multiple of the size. */
	leading?: number;
	/** How far above the head the label sits before anything pushes it. */
	offset?: Mm;
	/** Labels must end up at least this far apart. */
	gap?: Mm;
	/** Beyond this much displacement a leader is drawn. */
	leaderAfter?: Mm;
	/** How many relaxation passes. More is tidier and slower; two dozen settles a rig. */
	passes?: number;
	/** Keep every label inside this, where one is given. */
	bounds?: RectMm;
}

const DEFAULTS: Required<Omit<LabelOptions, 'bounds'>> = {
	size: 1.8,
	leading: 1.15,
	offset: 3.2,
	gap: 0.5,
	leaderAfter: 2.0,
	passes: 24
};

/** Where one label ended up. */
export interface PlacedLabel {
	/** The centre of the label block. */
	at: Pt;
	lines: string[];
	box: RectMm;
	/** The head this belongs to, for the leader. */
	anchor: Pt;
	/** Whether it moved far enough to need a line back to its head. */
	leader: boolean;
}

/**
 * Lay out every label, pushing them apart until they stop overlapping.
 *
 * The relaxation is deliberately simple — pairwise separation along whichever axis
 * needs the smaller push, damped, for a fixed number of passes — because the job is
 * not to find an optimal arrangement. It is to make a dense corner of a plan legible,
 * and a label that has moved four millimetres with a leader drawn to it does that
 * completely, whereas a clever placement that is different on the second run does not.
 *
 * Labels that still overlap after the last pass are left overlapping rather than
 * hidden: a reader who can see two names on top of each other knows to zoom in, and a
 * reader looking at the gap where a name should be knows nothing at all.
 */
export function layOutLabels(
	requests: LabelRequest[],
	metrics: Metrics,
	options: LabelOptions = {}
): PlacedLabel[] {
	const o = { ...DEFAULTS, ...options };
	const placed: PlacedLabel[] = [];

	for (const request of requests) {
		const lines = request.lines.filter((line) => line.length > 0);
		if (lines.length === 0) continue;
		const width = Math.max(...lines.map((line) => metrics.widthOf(line, o.size)));
		const height = lines.length * o.size * o.leading;
		const at = { x: request.anchor.x, y: request.anchor.y - o.offset - height / 2 };
		placed.push({
			at,
			lines,
			anchor: request.anchor,
			box: { x: at.x - width / 2, y: at.y - height / 2, w: width, h: height },
			leader: false
		});
	}

	for (let pass = 0; pass < o.passes; pass++) {
		let moved = false;
		for (let i = 0; i < placed.length; i++) {
			for (let j = i + 1; j < placed.length; j++) {
				if (separate(placed[i], placed[j], o.gap)) moved = true;
			}
		}
		// A label never ends up *on* its own head. Four lanterns on a boom are at one
		// point in plan, so separation stacks their labels — and the one at the bottom
		// of the stack is pushed straight down onto the symbol it names, which is the
		// one collision de-collision must not create.
		for (const label of placed) if (clearOfHead(label, o.offset)) moved = true;
		if (o.bounds) for (const label of placed) confine(label, o.bounds);
		if (!moved) break;
	}

	for (const label of placed) {
		const dx = label.at.x - label.anchor.x;
		const dy = label.at.y - (label.anchor.y - o.offset - label.box.h / 2);
		label.leader = Math.hypot(dx, dy) > o.leaderAfter;
	}
	return placed;
}

/** Push two labels apart if they overlap. True if anything moved. */
function separate(a: PlacedLabel, b: PlacedLabel, gap: Mm): boolean {
	const overlapX = (a.box.w + b.box.w) / 2 + gap - Math.abs(a.at.x - b.at.x);
	const overlapY = (a.box.h + b.box.h) / 2 + gap - Math.abs(a.at.y - b.at.y);
	if (overlapX <= 0 || overlapY <= 0) return false;

	// Along whichever axis needs the smaller push, so a row of heads on a bar spreads
	// sideways and a stack of them spreads upwards — which is what somebody laying the
	// sheet out by hand would do.
	//
	// **A vertical push only ever goes up.** Splitting it — one label up, one down —
	// is the obvious thing and it does not converge: the lower label lands on its own
	// head, `clearOfHead` lifts it back, and the two rules push it round in a circle
	// until the passes run out. Growing the stack upwards away from the heads is both
	// stable and what somebody arranging the sheet would do anyway.
	if (overlapY <= overlapX) {
		const push = overlapY * 1.01;
		// The one already higher goes higher. A tie is broken by argument order, which
		// is the rig's own order, so the result does not depend on floating-point noise.
		move(a.at.y <= b.at.y ? a : b, 0, -push);
	} else {
		const push = (overlapX / 2) * 1.01;
		// A tie in x is broken by which label came first, never by which is which —
		// two heads at exactly the same point must not depend on floating-point noise
		// to decide who goes left.
		const left = a.at.x <= b.at.x ? -1 : 1;
		move(a, push * left, 0);
		move(b, -push * left, 0);
	}
	return true;
}

/** Push a label up until its box clears the head it belongs to. */
function clearOfHead(label: PlacedLabel, offset: Mm): boolean {
	const wanted = label.anchor.y - offset;
	const overlap = label.box.y + label.box.h - wanted;
	if (overlap <= 0) return false;
	move(label, 0, -overlap);
	return true;
}

function move(label: PlacedLabel, dx: Mm, dy: Mm): void {
	label.at.x += dx;
	label.at.y += dy;
	label.box.x += dx;
	label.box.y += dy;
}

/** Keep a label inside the frame it belongs to. */
function confine(label: PlacedLabel, bounds: RectMm): void {
	const left = bounds.x - label.box.x;
	if (left > 0) move(label, left, 0);
	const right = label.box.x + label.box.w - (bounds.x + bounds.w);
	if (right > 0) move(label, -right, 0);
	const top = bounds.y - label.box.y;
	if (top > 0) move(label, 0, top);
	const bottom = label.box.y + label.box.h - (bounds.y + bounds.h);
	if (bottom > 0) move(label, 0, -bottom);
}

/** The text and leader items for a laid-out label. */
export function labelItems(
	label: PlacedLabel,
	size: Mm = DEFAULTS.size,
	leading: number = DEFAULTS.leading
): Array<TextItem | PathItem> {
	const items: Array<TextItem | PathItem> = [];
	if (label.leader) {
		// To the nearest point on the label's own box rather than to its centre, so
		// the line stops at the text instead of running through it.
		const to = nearestOnBox(label.anchor, label.box);
		items.push({
			kind: 'path',
			points: [[label.anchor, to]],
			stroke: stroke(WEIGHT.fine, BLACK)
		});
	}
	label.lines.forEach((line, index) => {
		items.push({
			kind: 'text',
			at: { x: label.at.x, y: label.box.y + index * size * leading },
			text: line,
			size,
			anchor: 'middle',
			baseline: 'top'
		});
	});
	return items;
}

function nearestOnBox(from: Pt, box: RectMm): Pt {
	return {
		x: Math.min(Math.max(from.x, box.x), box.x + box.w),
		y: Math.min(Math.max(from.y, box.y), box.y + box.h)
	};
}
