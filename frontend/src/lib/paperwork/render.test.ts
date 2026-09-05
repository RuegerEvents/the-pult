/**
 * The two renderings of one page, checked against each other.
 *
 * The claim this feature rests on is that the SVG somebody approves and the PDF they
 * send are the same drawing. That is structurally true for geometry — both take the
 * model's own millimetres, and the PDF's single flip is one line — and it is *made*
 * true for text by `textOrigin`, which both call. These tests are what keeps it true:
 * they measure against the real shipped faces, so a figure in here is a figure about
 * the type the console actually draws with.
 */

import { readFile } from 'node:fs/promises';
import { PDFDocument } from 'pdf-lib';
import { describe, expect, it } from 'vitest';

import { WEIGHT, page, rectPath, stroke, type Drawing } from './drawing.js';
import { FONT_URLS, forgetFont, metrics, textOrigin, type Metrics } from './font.js';
import { MM, toPdf } from './pdf.js';
import { toSvg } from './svg.js';

/** The same two files the browser fetches, off the disk. */
async function realMetrics(): Promise<Metrics> {
	forgetFont();
	return metrics(async (url) => new Uint8Array(await readFile(`static${url}`)));
}

function aPage(): Drawing {
	const drawing = page(420, 297);
	drawing.items.push(
		{
			kind: 'path',
			points: [rectPath({ x: 10, y: 10, w: 400, h: 277 })],
			stroke: stroke(WEIGHT.border)
		},
		{ kind: 'text', at: { x: 20, y: 20 }, text: 'Fixtures', size: 3.4, bold: true },
		{
			kind: 'text',
			at: { x: 400, y: 20 },
			text: '1:50',
			size: 2.6,
			anchor: 'end'
		}
	);
	return drawing;
}

describe('the shipped font', () => {
	it('measures a string the same way twice', async () => {
		const m = await realMetrics();
		expect(m.widthOf('Spikie 1', 2)).toBeCloseTo(m.widthOf('Spikie 1', 2));
	});

	it('has plausible vertical proportions', async () => {
		const m = await realMetrics();
		expect(m.ascentRatio).toBeGreaterThan(0.7);
		expect(m.ascentRatio).toBeLessThan(1.3);
		expect(m.capRatio).toBeGreaterThan(0.6);
		expect(m.capRatio).toBeLessThan(0.8);
	});

	it('scales linearly with the size, which is what a scaled drawing assumes', async () => {
		const m = await realMetrics();
		expect(m.widthOf('HydraPanel 1', 4)).toBeCloseTo(m.widthOf('HydraPanel 1', 2) * 2, 6);
	});

	it('measures nothing as nothing', async () => {
		const m = await realMetrics();
		expect(m.widthOf('', 3)).toBe(0);
	});
});

describe('both renderings of one page', () => {
	it('puts text at the same place, resolved once', async () => {
		const m = await realMetrics();
		const item = {
			at: { x: 400, y: 20 },
			text: '1:50',
			size: 2.6,
			anchor: 'end' as const,
			baseline: 'top' as const
		};
		const origin = textOrigin(item, m);
		// End-anchored, so the origin is a whole string's width to the left.
		expect(origin.x).toBeCloseTo(400 - m.widthOf('1:50', 2.6));

		// And that is the number the SVG carries: the renderer resolves the anchor
		// itself rather than leaving it to `text-anchor`.
		const svg = toSvg(aPage(), m);
		expect(svg).toContain(`x="${Math.round(origin.x * 1e4) / 1e4}"`);
		expect(svg).toContain('text-anchor="start"');
	});

	it('writes an SVG whose page is stated in millimetres', async () => {
		const m = await realMetrics();
		const svg = toSvg(aPage(), m);
		expect(svg).toContain('width="420mm"');
		expect(svg).toContain('height="297mm"');
		expect(svg).toContain('viewBox="0 0 420 297"');
	});

	it('writes a PDF of the same page at the same physical size', async () => {
		const m = await realMetrics();
		const bytes = await toPdf([aPage()], m, { title: 'Fixtures', author: '' });
		expect(bytes.length).toBeGreaterThan(1000);
		// A3 landscape is 1190.55 × 841.89 pt.
		expect(420 * MM).toBeCloseTo(1190.55, 1);
		expect(297 * MM).toBeCloseTo(841.89, 1);
		const header = new TextDecoder().decode(bytes.subarray(0, 8));
		expect(header.startsWith('%PDF-')).toBe(true);
	});

	it('puts every sheet of a set in one file, at the right size', async () => {
		const m = await realMetrics();
		const six = await toPdf([aPage(), aPage(), aPage(), aPage(), aPage(), aPage()], m);
		// Read back rather than searched for as text: the page tree is compressed, and
		// a regex over the bytes would pass on a file no reader could open.
		const back = await PDFDocument.load(six);
		expect(back.getPageCount()).toBe(6);
		const { width, height } = back.getPage(0).getSize();
		expect(width).toBeCloseTo(1190.55, 1);
		expect(height).toBeCloseTo(841.89, 1);
		expect(back.getTitle()).toBe('Paperwork');
	});
});

describe('escaping', () => {
	it('does not let a fixture name close the SVG', async () => {
		const m = await realMetrics();
		const drawing = page(100, 100);
		drawing.items.push({
			kind: 'text',
			at: { x: 5, y: 5 },
			text: '</text><script>alert(1)</script>',
			size: 2
		});
		const svg = toSvg(drawing, m);
		expect(svg).not.toContain('<script>');
		expect(svg).toContain('&lt;/text&gt;');
	});
});
