/**
 * One font, measured once, used by the preview and by the file.
 *
 * The preview is SVG in a browser and the output is PDF, and if those two lay text out
 * with different metrics then what somebody approves on screen is not what prints.
 * That is not a cosmetic difference: label de-collision decides where a fixture's name
 * goes by *how wide it is*, and a column's width is the widest cell in it. Measured
 * against the browser's idea of Helvetica, a sheet laid out on a Mac and printed from a
 * Linux tablet would place its labels differently.
 *
 * So the console ships a font — Open Sans, under the SIL Open Font Licence, in
 * `static/fonts/` — measures every string against **its** metrics through fontkit, and
 * hands the same bytes to `pdf-lib` to embed. The SVG asks for the same face by name
 * through an `@font-face` pointing at the same file. One face, one set of advances,
 * two renderers.
 *
 * # There is no fallback, deliberately
 *
 * A metric fallback — average character widths, or the browser's own measurement — is a
 * second answer to the one question this module exists to have a single answer to. So
 * layout *waits* for the font, and a sheet cannot be drawn before it arrives. That is
 * the same rule the export follows for meshes: paperwork that is nearly right is worse
 * than paperwork that is not ready yet.
 */

import fontkit from '@pdf-lib/fontkit';

/** Where the two faces live, served by SvelteKit's static directory in dev and by
 * `rust-embed` in a release build — the same path in both. */
export const FONT_URLS = {
	regular: '/fonts/OpenSans-Regular.ttf',
	bold: '/fonts/OpenSans-Bold.ttf'
} as const;

/** The family name the SVG asks for. */
export const FONT_FAMILY = 'Pult Paperwork';

/** What a laid-out page needs to know about its type. */
export interface Metrics {
	/** The width of a string at a given em size, in the same unit as the size. */
	widthOf(text: string, size: number, bold?: boolean): number;
	/** Cap height as a fraction of the em, for the one caller stating a drawing height. */
	capRatio: number;
	/** Ascent as a fraction of the em, which is what `baseline: 'top'` measures from. */
	ascentRatio: number;
	/** Descent as a positive fraction of the em. */
	descentRatio: number;
	/** The bytes, for `pdf-lib` to embed. */
	bytes: { regular: Uint8Array; bold: Uint8Array };
}

interface Face {
	unitsPerEm: number;
	ascent: number;
	descent: number;
	capHeight: number;
	layout(text: string): { advanceWidth: number };
}

function build(regularBytes: Uint8Array, boldBytes: Uint8Array): Metrics {
	const regular = fontkit.create(regularBytes as never) as unknown as Face;
	const bold = fontkit.create(boldBytes as never) as unknown as Face;

	return {
		widthOf(text: string, size: number, isBold = false): number {
			if (!text) return 0;
			const face = isBold ? bold : regular;
			return (face.layout(text).advanceWidth / face.unitsPerEm) * size;
		},
		capRatio: regular.capHeight / regular.unitsPerEm,
		ascentRatio: regular.ascent / regular.unitsPerEm,
		descentRatio: Math.abs(regular.descent) / regular.unitsPerEm,
		bytes: { regular: regularBytes, bold: boldBytes }
	};
}

let pending: Promise<Metrics> | null = null;

/**
 * The metrics, loading the faces on first ask and reusing them after.
 *
 * `fetchBytes` exists for the tests, which read the same two files off the disk rather
 * than over a network that is not there — the point being that they measure against the
 * *same faces the browser does*, so a corpus figure means something.
 */
export function metrics(
	fetchBytes: (url: string) => Promise<Uint8Array> = defaultFetch
): Promise<Metrics> {
	if (!pending) {
		pending = Promise.all([fetchBytes(FONT_URLS.regular), fetchBytes(FONT_URLS.bold)])
			.then(([regular, bold]) => build(regular, bold))
			.catch((error) => {
				// Clear the cache so a page that failed once because the network hiccuped
				// can try again, rather than being permanently unable to draw.
				pending = null;
				throw error;
			});
	}
	return pending;
}

/** Forget the loaded faces. For tests, and for nothing else. */
export function forgetFont(): void {
	pending = null;
}

async function defaultFetch(url: string): Promise<Uint8Array> {
	const response = await fetch(url);
	if (!response.ok) throw new Error(`${url} answered ${response.status}`);
	return new Uint8Array(await response.arrayBuffer());
}

/**
 * The `@font-face` the preview needs, as a stylesheet fragment.
 *
 * Inline in the SVG rather than in the app's CSS, so a preview that is copied out of
 * the panel — or opened on its own — still carries its own type.
 */
export function fontFaceCss(): string {
	return `@font-face{font-family:'${FONT_FAMILY}';font-weight:400;src:url('${FONT_URLS.regular}') format('truetype');}
@font-face{font-family:'${FONT_FAMILY}';font-weight:700;src:url('${FONT_URLS.bold}') format('truetype');}`;
}

// ── Where a piece of text actually goes ──────────────────────────────────────

/**
 * The baseline a text item sits on, and the left edge it starts at.
 *
 * **One implementation, called by both renderers.** This was written twice — once in
 * `svg.ts` and once in `pdf.ts` — for about an hour, which is exactly long enough to
 * notice that it is the same defect the rest of this feature is arranged against: two
 * places deciding where a word goes will agree until one of them is changed, and the
 * symptom is a file that does not match the preview it was approved from.
 *
 * The anchor is resolved here as well, so that `end`-anchored text lands on the same
 * millimetre in both — SVG could do it with `text-anchor` and PDF cannot, and letting
 * each do it its own way is the same trap one level down.
 */
export function textOrigin(
	item: {
		at: { x: number; y: number };
		text: string;
		size: number;
		bold?: boolean;
		anchor?: 'start' | 'middle' | 'end';
		baseline?: 'top' | 'middle' | 'baseline';
	},
	metrics: Metrics
): { x: number; y: number } {
	let x = item.at.x;
	if (item.anchor === 'middle') x -= metrics.widthOf(item.text, item.size, item.bold) / 2;
	else if (item.anchor === 'end') x -= metrics.widthOf(item.text, item.size, item.bold);

	let y = item.at.y;
	// `top` is the default because it is what a table row and a label line want: laying
	// text out from a baseline means every caller carrying the ascent around.
	if (item.baseline === 'top' || item.baseline === undefined) y += metrics.ascentRatio * item.size;
	else if (item.baseline === 'middle')
		y += (metrics.ascentRatio - metrics.descentRatio) * 0.5 * item.size;
	return { x, y };
}
