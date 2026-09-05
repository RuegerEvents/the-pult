/**
 * A sheet, as geometry, before anybody decides what to write it with.
 *
 * This is the one representation of a page in this console, and everything about
 * paperwork hangs off the fact that it is rendered **twice** — to SVG for the preview
 * a person approves, and to PDF operators for the file they send. Two renderers over
 * one model rather than two page-builders, for the reason `pult-render` is compiled
 * twice rather than translated: the visible form of the two drifting apart is a
 * drawing that prints differently from the one somebody signed off, and nobody would
 * find that until it was on paper in a room.
 *
 * # Millimetres, from the top-left, y downwards
 *
 * Paper units all the way through. A viewport at 1:50 is a window of a known size onto
 * the rig, and once the scale is resolved every coordinate in here is a physical
 * distance on the page — which is what makes "is this drawing to scale" a question
 * with an answer rather than a hope. SVG shares this convention; PDF does not, and
 * `pdf.ts` is where the flip happens, once.
 *
 * # Clipping is here and not in either renderer
 *
 * A viewport shows part of the rig, so lines have to be cut at its frame. That is done
 * to the model, by {@link clipToRect}, rather than by an SVG `clipPath` and a PDF
 * graphics-state clip — which would be two implementations of the same cut, in two
 * languages, agreeing until they did not. It also means the corpus can assert it.
 */

/** A length on the page, in millimetres. */
export type Mm = number;

/** A point on the page. */
export interface Pt {
	x: Mm;
	y: Mm;
}

/** A rectangle on the page, from its top-left corner. */
export interface RectMm {
	x: Mm;
	y: Mm;
	w: Mm;
	h: Mm;
}

/** How a line is drawn. */
export interface Stroke {
	/** Millimetres. A drawing's line weights are 0.13, 0.25, 0.35, 0.5, 0.7. */
	width: Mm;
	color: string;
	/** Dash pattern in millimetres, e.g. `[2, 1]`. Omitted for a solid line. */
	dash?: number[];
}

/** Where a piece of text sits relative to its anchor point. */
export type TextAnchor = 'start' | 'middle' | 'end';

/**
 * Vertical placement.
 *
 * `top` measures from the cap line and is what a table row and a label line want —
 * laying text out from a baseline means every caller carrying the font's ascent
 * around.
 */
export type TextBaseline = 'top' | 'middle' | 'baseline';

export interface TextItem {
	kind: 'text';
	at: Pt;
	text: string;
	/**
	 * The font's em size, in millimetres — the same number a CSS `font-size` or a PDF
	 * `Tf` takes, so neither renderer has to convert. A drawing's nominal text height
	 * is its *cap* height, which for this face is about 0.71 of this; `font.ts` is
	 * where that conversion lives, for the one caller that wants it.
	 */
	size: Mm;
	bold?: boolean;
	anchor?: TextAnchor;
	baseline?: TextBaseline;
	color?: string;
	/** Degrees clockwise about `at`. Used by a rotated dimension or a side label. */
	rotate?: number;
}

export interface PathItem {
	kind: 'path';
	/**
	 * One or more polylines. Several in one item so a truss's whole outline is one
	 * fill rather than a dozen, which matters: an A3 plan of a festival rig is tens of
	 * thousands of segments and an item each would be tens of thousands of PDF
	 * operators.
	 */
	points: Pt[][];
	stroke?: Stroke;
	/** A fill colour closes each polyline. `#fff` is what paints out what is behind. */
	fill?: string;
}

export interface ImageItem {
	kind: 'image';
	rect: RectMm;
	/** PNG bytes. The only raster format either renderer takes. */
	data: Uint8Array;
}

export type DrawItem = TextItem | PathItem | ImageItem;

/** A whole page. */
export interface Drawing {
	width: Mm;
	height: Mm;
	items: DrawItem[];
}

/** Line weights, in millimetres, and what each of them is for. */
export const WEIGHT = {
	/** Hairlines: a hidden edge, a grid, a leader. */
	fine: 0.13,
	/** Text underlines, table rules, a fixture's own outline. */
	thin: 0.25,
	/** The default. */
	medium: 0.35,
	/** Structure: truss, decks, anything load-bearing. */
	heavy: 0.5,
	/** A sheet's own border. */
	border: 0.7
} as const;

export const BLACK = '#000000';
export const WHITE = '#ffffff';

/** A stroke, said shortly. */
export function stroke(width: Mm = WEIGHT.medium, color = BLACK, dash?: number[]): Stroke {
	return dash ? { width, color, dash } : { width, color };
}

/** An empty page of a given size. */
export function page(width: Mm, height: Mm): Drawing {
	return { width, height, items: [] };
}

/** The four corners of a rectangle, as a closed polyline. */
export function rectPath(r: RectMm): Pt[] {
	return [
		{ x: r.x, y: r.y },
		{ x: r.x + r.w, y: r.y },
		{ x: r.x + r.w, y: r.y + r.h },
		{ x: r.x, y: r.y + r.h },
		{ x: r.x, y: r.y }
	];
}

/** Whether a point is inside a rectangle, edges included. */
export function inside(p: Pt, r: RectMm): boolean {
	return p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;
}

// ── Clipping ─────────────────────────────────────────────────────────────────

/**
 * Cut a polyline to a rectangle, keeping the parts inside it.
 *
 * Liang–Barsky per segment, which gives back a list of runs rather than one line: a
 * truss crossing a viewport, leaving it and coming back is two visible pieces, and
 * joining them up would draw a line through the middle of the frame that is not in the
 * rig.
 */
export function clipPolyline(points: Pt[], r: RectMm): Pt[][] {
	const out: Pt[][] = [];
	let run: Pt[] = [];

	for (let i = 0; i + 1 < points.length; i++) {
		const seg = clipSegment(points[i], points[i + 1], r);
		if (!seg) {
			if (run.length > 1) out.push(run);
			run = [];
			continue;
		}
		const [a, b] = seg;
		if (run.length === 0) {
			run.push(a, b);
		} else {
			// The segment was clipped at its start, so it does not continue the run.
			const last = run[run.length - 1];
			if (Math.abs(last.x - a.x) > 1e-9 || Math.abs(last.y - a.y) > 1e-9) {
				if (run.length > 1) out.push(run);
				run = [a, b];
			} else {
				run.push(b);
			}
		}
	}
	if (run.length > 1) out.push(run);
	return out;
}

function clipSegment(a: Pt, b: Pt, r: RectMm): [Pt, Pt] | null {
	const dx = b.x - a.x;
	const dy = b.y - a.y;
	let t0 = 0;
	let t1 = 1;
	const p = [-dx, dx, -dy, dy];
	const q = [a.x - r.x, r.x + r.w - a.x, a.y - r.y, r.y + r.h - a.y];

	for (let i = 0; i < 4; i++) {
		if (p[i] === 0) {
			// Parallel to this edge and outside it: nothing of the segment survives.
			if (q[i] < 0) return null;
			continue;
		}
		const t = q[i] / p[i];
		if (p[i] < 0) {
			if (t > t1) return null;
			if (t > t0) t0 = t;
		} else {
			if (t < t0) return null;
			if (t < t1) t1 = t;
		}
	}
	return [
		{ x: a.x + t0 * dx, y: a.y + t0 * dy },
		{ x: a.x + t1 * dx, y: a.y + t1 * dy }
	];
}

/**
 * Cut a filled polygon to a rectangle.
 *
 * Sutherland–Hodgman against the four edges. Unlike the polyline case this must give
 * back one closed shape rather than runs, because a fill has an inside: a truss half
 * out of the frame is still solid where it is visible, and cutting it into open runs
 * would paint a wedge across the sheet.
 */
export function clipPolygon(points: Pt[], r: RectMm): Pt[] {
	const edges: Array<(p: Pt) => boolean> = [
		(p) => p.x >= r.x,
		(p) => p.x <= r.x + r.w,
		(p) => p.y >= r.y,
		(p) => p.y <= r.y + r.h
	];
	const cuts: Array<(a: Pt, b: Pt) => Pt> = [
		(a, b) => atX(a, b, r.x),
		(a, b) => atX(a, b, r.x + r.w),
		(a, b) => atY(a, b, r.y),
		(a, b) => atY(a, b, r.y + r.h)
	];

	let poly = points.slice();
	// A closed polyline arrives with its first point repeated; the algorithm wants the
	// ring without it.
	if (poly.length > 1 && same(poly[0], poly[poly.length - 1])) poly = poly.slice(0, -1);

	for (let e = 0; e < 4 && poly.length > 0; e++) {
		const next: Pt[] = [];
		for (let i = 0; i < poly.length; i++) {
			const current = poly[i];
			const previous = poly[(i + poly.length - 1) % poly.length];
			const currentIn = edges[e](current);
			const previousIn = edges[e](previous);
			if (currentIn) {
				if (!previousIn) next.push(cuts[e](previous, current));
				next.push(current);
			} else if (previousIn) {
				next.push(cuts[e](previous, current));
			}
		}
		poly = next;
	}
	if (poly.length < 3) return [];
	return [...poly, poly[0]];
}

function same(a: Pt, b: Pt): boolean {
	return Math.abs(a.x - b.x) < 1e-9 && Math.abs(a.y - b.y) < 1e-9;
}

function atX(a: Pt, b: Pt, x: Mm): Pt {
	const t = (x - a.x) / (b.x - a.x || 1e-12);
	return { x, y: a.y + t * (b.y - a.y) };
}

function atY(a: Pt, b: Pt, y: Mm): Pt {
	const t = (y - a.y) / (b.y - a.y || 1e-12);
	return { x: a.x + t * (b.x - a.x), y };
}

/**
 * Cut one item to a rectangle, dropping it entirely where nothing survives.
 *
 * Text is kept or dropped whole, by its anchor point: half a label is worse than no
 * label, and a viewport whose frame ran through the middle of a fixture's name would
 * read as a rendering fault rather than as a boundary.
 */
export function clipToRect(item: DrawItem, r: RectMm): DrawItem | null {
	if (item.kind === 'text') return inside(item.at, r) ? item : null;
	if (item.kind === 'image') {
		const x = Math.max(item.rect.x, r.x);
		const y = Math.max(item.rect.y, r.y);
		const right = Math.min(item.rect.x + item.rect.w, r.x + r.w);
		const bottom = Math.min(item.rect.y + item.rect.h, r.y + r.h);
		// An image is placed rather than cut: a raster viewport is rendered to exactly
		// its own frame, so anything else here is a caller's mistake and cropping the
		// pixels would hide it.
		return right > x && bottom > y ? item : null;
	}

	const points: Pt[][] = [];
	for (const line of item.points) {
		if (item.fill) {
			const cut = clipPolygon(line, r);
			if (cut.length > 2) points.push(cut);
		} else {
			points.push(...clipPolyline(line, r));
		}
	}
	return points.length ? { ...item, points } : null;
}

/** Cut a whole list of items to a rectangle. */
export function clipAll(items: DrawItem[], r: RectMm): DrawItem[] {
	const out: DrawItem[] = [];
	for (const item of items) {
		const cut = clipToRect(item, r);
		if (cut) out.push(cut);
	}
	return out;
}
