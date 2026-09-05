/**
 * A {@link Drawing}, as SVG: the preview half of "rendered twice".
 *
 * SVG shares the drawing model's convention — millimetres, y downwards, origin
 * top-left — so this renderer converts nothing at all. That is on purpose and it is
 * why the model is in those units: the renderer with the arithmetic in it is the one
 * that can be wrong, and here there is none.
 *
 * The one thing it does have to do is measure text the way `pdf.ts` does, which it
 * gets for free by not measuring any: `anchor` becomes `text-anchor` and `baseline`
 * becomes `dominant-baseline`, both of which the browser resolves against the same
 * embedded face the PDF carries.
 */

import { FONT_FAMILY, fontFaceCss, textOrigin, type Metrics } from './font.js';
import type { DrawItem, Drawing, Mm, Pt } from './drawing.js';

/** Four decimal places of a millimetre is a tenth of a micrometre. */
function n(value: number): string {
	return Number.isFinite(value) ? String(Math.round(value * 1e4) / 1e4) : '0';
}

function points(line: Pt[]): string {
	return line.map((p) => `${n(p.x)},${n(p.y)}`).join(' ');
}

function escape(text: string): string {
	return text
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;');
}

/** Bytes to a data URI, for the one item kind that carries any. */
function pngDataUri(data: Uint8Array): string {
	let binary = '';
	// In chunks: `String.fromCharCode(...bytes)` on a 20 MB render blows the argument
	// limit, which is a crash on exactly the sheets somebody cares most about.
	const chunk = 0x8000;
	for (let i = 0; i < data.length; i += chunk) {
		binary += String.fromCharCode(...data.subarray(i, i + chunk));
	}
	return `data:image/png;base64,${btoa(binary)}`;
}

function item(it: DrawItem, metrics: Metrics): string {
	if (it.kind === 'path') {
		const shapes = it.points
			.map((line) => {
				const tag = it.fill ? 'polygon' : 'polyline';
				return `<${tag} points="${points(line)}"/>`;
			})
			.join('');
		const fill = it.fill ? `fill="${it.fill}"` : 'fill="none"';
		const s = it.stroke
			? `stroke="${it.stroke.color}" stroke-width="${n(it.stroke.width)}"` +
				(it.stroke.dash ? ` stroke-dasharray="${it.stroke.dash.map(n).join(' ')}"` : '')
			: 'stroke="none"';
		return `<g ${fill} ${s} stroke-linejoin="round" stroke-linecap="round">${shapes}</g>`;
	}

	if (it.kind === 'image') {
		const { x, y, w, h } = it.rect;
		return `<image x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" preserveAspectRatio="none" href="${pngDataUri(it.data)}"/>`;
	}

	// Anchor and baseline are resolved by `textOrigin`, not by `text-anchor` and
	// `dominant-baseline`: the PDF has no equivalent of either, so letting the browser
	// decide here would be a second opinion about where a word goes.
	const { x, y } = textOrigin(it, metrics);
	const transform = it.rotate ? ` transform="rotate(${n(it.rotate)} ${n(x)} ${n(y)})"` : '';
	return (
		`<text x="${n(x)}" y="${n(y)}" font-size="${n(it.size)}" ` +
		`text-anchor="start" fill="${it.color ?? '#000000'}"` +
		(it.bold ? ' font-weight="700"' : '') +
		`${transform}>${escape(it.text)}</text>`
	);
}

/**
 * The whole page, as one SVG string.
 *
 * `width` and `height` are stated in millimetres and the viewBox is the same numbers,
 * so a browser printing this at 100% puts it on the paper at its real size — which is
 * what makes the preview a preview rather than a picture of one.
 */
export function toSvg(drawing: Drawing, metrics: Metrics): string {
	const body = drawing.items.map((it) => item(it, metrics)).join('\n');
	return [
		`<svg xmlns="http://www.w3.org/2000/svg" width="${n(drawing.width)}mm" height="${n(drawing.height)}mm" viewBox="0 0 ${n(drawing.width)} ${n(drawing.height)}">`,
		`<style>${fontFaceCss()} text{font-family:'${FONT_FAMILY}';}</style>`,
		`<rect x="0" y="0" width="${n(drawing.width)}" height="${n(drawing.height)}" fill="#ffffff"/>`,
		body,
		'</svg>'
	].join('\n');
}

/** How wide the page is on screen at a given zoom, in CSS pixels. */
export function pixelWidth(drawing: Drawing, zoom: number): Mm {
	return (drawing.width / 25.4) * 96 * zoom;
}
