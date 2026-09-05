/**
 * A {@link Drawing}, as PDF: the file half of "rendered twice".
 *
 * The only renderer with arithmetic in it, and all of the arithmetic is one flip. PDF
 * measures in points from the *bottom* left with y upwards; the drawing model measures
 * in millimetres from the top left with y downwards. So every y becomes
 * `(height - y) · mm`, and nothing else about a page changes between the preview and
 * the file.
 *
 * There is no SVG-to-PDF conversion anywhere in this feature, which is the reason the
 * drawing model exists: a converter is a third implementation of the page, with its own
 * opinions about dashes, joins and text placement, and it would be the thing that made
 * the file differ from the preview.
 */

import fontkit from '@pdf-lib/fontkit';
import { degrees, PDFDocument, rgb, type PDFFont, type PDFPage } from 'pdf-lib';

import type { DrawItem, Drawing, Mm } from './drawing.js';
import { textOrigin, type Metrics } from './font.js';

/** Points per millimetre. */
export const MM = 72 / 25.4;

function colour(hex: string) {
	const value = hex.replace('#', '');
	const full =
		value.length === 3
			? value
					.split('')
					.map((c) => c + c)
					.join('')
			: value;
	const int = parseInt(full, 16);
	return rgb(((int >> 16) & 255) / 255, ((int >> 8) & 255) / 255, (int & 255) / 255);
}

interface Faces {
	regular: PDFFont;
	bold: PDFFont;
}

function place(item: DrawItem, page: PDFPage, height: Mm, faces: Faces, metrics: Metrics) {
	const up = (y: Mm) => (height - y) * MM;

	if (item.kind === 'path') {
		for (const line of item.points) {
			if (line.length < 2) continue;
			// In the drawing's own coordinates, scaled to points but still y-down.
			// `drawSvgPath` maps a path point to `(x + px, y - py)`, so putting the
			// origin at the top of the page is what turns y-down into y-up — one flip,
			// done by the library, rather than a second one done here on top of it.
			const path =
				line.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x * MM} ${p.y * MM}`).join(' ') +
				(item.fill ? ' Z' : '');
			page.drawSvgPath(path, {
				x: 0,
				y: height * MM,
				scale: 1,
				borderColor: item.stroke ? colour(item.stroke.color) : undefined,
				borderWidth: item.stroke ? item.stroke.width * MM : 0,
				borderDashArray: item.stroke?.dash?.map((d) => d * MM),
				color: item.fill ? colour(item.fill) : undefined
			});
		}
		return;
	}

	if (item.kind === 'image') {
		// Embedding is async, so images are pre-embedded by `toPdf` and this is never
		// reached with one. Kept total rather than throwing: a missing picture should
		// leave a hole in a sheet, not fail an export somebody is waiting on.
		return;
	}

	const font = item.bold ? faces.bold : faces.regular;
	// The same origin `svg.ts` draws at, from the same function — see `textOrigin`.
	const { x, y } = textOrigin(item, metrics);

	page.drawText(item.text, {
		x: x * MM,
		y: up(y),
		size: item.size * MM,
		font,
		color: colour(item.color ?? '#000000'),
		rotate: item.rotate ? degrees(-item.rotate) : undefined
	});
}

/**
 * Write a set of pages as one PDF.
 *
 * One file with every sheet in it rather than one file per sheet: a sheet set is a
 * document, and six downloads is six things to attach to an email.
 */
export async function toPdf(
	drawings: Drawing[],
	metrics: Metrics,
	meta: { title: string; author: string } = { title: 'Paperwork', author: '' }
): Promise<Uint8Array> {
	const doc = await PDFDocument.create();
	doc.registerFontkit(fontkit);
	doc.setTitle(meta.title);
	if (meta.author) doc.setAuthor(meta.author);
	doc.setProducer('the-pult');
	doc.setCreator('the-pult');

	// Subset, so a sheet with forty words in it does not carry 290 KB of glyphs it
	// never draws.
	const faces: Faces = {
		regular: await doc.embedFont(metrics.bytes.regular, { subset: true }),
		bold: await doc.embedFont(metrics.bytes.bold, { subset: true })
	};

	for (const drawing of drawings) {
		const page = doc.addPage([drawing.width * MM, drawing.height * MM]);
		for (const item of drawing.items) {
			if (item.kind === 'image') {
				const png = await doc.embedPng(item.data);
				page.drawImage(png, {
					x: item.rect.x * MM,
					y: (drawing.height - item.rect.y - item.rect.h) * MM,
					width: item.rect.w * MM,
					height: item.rect.h * MM
				});
				continue;
			}
			place(item, page, drawing.height, faces, metrics);
		}
	}

	return doc.save();
}
