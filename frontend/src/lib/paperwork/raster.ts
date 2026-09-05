/**
 * A picture viewport: the rig renderer, off screen, at print resolution.
 *
 * A drafting viewport is drawn by `project.ts` into the sheet's own vector model. A
 * *picture* one cannot be — a lit beam, haze, bloom and tone mapping are a shader's
 * answer and there is no vector form of them — so it is rendered by the same WebGL
 * renderer the rig panel uses and placed on the sheet as an image.
 *
 * # It cannot borrow the panel's renderer
 *
 * `Rig3D` owns a renderer per panel, deliberately, because two rig tiles can be open at
 * once. Capturing from an open one would mean the paperwork depended on somebody having
 * a rig panel open, at the right size, pointed the right way. So the export mounts its
 * own, hidden, at exactly the pixel size the sheet wants, and takes it down afterwards.
 *
 * # What "300 dpi" actually gets you
 *
 * A 190 mm viewport at 300 dpi is 2244 pixels across, which is fine. The same viewport
 * on A1 at 600 would not be, and neither is any of this on a machine whose GPU will not
 * allocate the buffer. So the request is a *ceiling*: {@link pixelsFor} reduces it to
 * what the context will actually give, and the sheet prints the dpi it got rather than
 * the dpi it asked for. A picture that quietly came out at 96 dpi is a sheet that looks
 * wrong for a reason nobody can see.
 */

import type { Mm, RectMm } from './drawing.js';

/** Millimetres to inches. */
const PER_INCH = 25.4;

/** The largest texture this machine will give, or a safe guess if it will not say. */
export function maxTextureSize(): number {
	try {
		const canvas = document.createElement('canvas');
		const gl = canvas.getContext('webgl2') ?? canvas.getContext('webgl');
		if (!gl) return 4096;
		return (gl as WebGLRenderingContext).getParameter(
			(gl as WebGLRenderingContext).MAX_TEXTURE_SIZE
		) as number;
	} catch {
		return 4096;
	}
}

/**
 * How many pixels to render a viewport at, and the dpi that actually comes to.
 *
 * Both are returned because the second is printed on the sheet. A caller that only
 * asked for the pixels would have no way to be honest about what it got.
 */
export function pixelsFor(
	rect: RectMm,
	wantedDpi: number,
	limit = maxTextureSize()
): { width: number; height: number; dpi: number } {
	const at = (dpi: number) => ({
		width: Math.max(1, Math.round((rect.w / PER_INCH) * dpi)),
		height: Math.max(1, Math.round((rect.h / PER_INCH) * dpi))
	});
	let dpi = Math.max(24, Math.round(wantedDpi));
	let size = at(dpi);
	while ((size.width > limit || size.height > limit) && dpi > 24) {
		// Halve rather than step: the loop has to terminate quickly on a machine whose
		// limit is 2048 and a sheet that asked for A1 at 600.
		dpi = Math.max(24, Math.floor(dpi / 2));
		size = at(dpi);
	}
	return { ...size, dpi };
}

/** The image data a picture viewport ends up as. */
export interface Raster {
	data: Uint8Array;
	dpi: number;
}

/**
 * A canvas as PNG bytes.
 *
 * PNG rather than JPEG because a drawing is line art over flat colour, which is exactly
 * what JPEG is worst at — and a shaded truss with ringing round every edge looks like a
 * bad scan of a good drawing.
 */
export async function canvasToPng(canvas: HTMLCanvasElement): Promise<Uint8Array> {
	const blob = await new Promise<Blob | null>((resolve) =>
		canvas.toBlob((b) => resolve(b), 'image/png')
	);
	if (!blob) throw new Error('the view could not be captured');
	return new Uint8Array(await blob.arrayBuffer());
}

/** How wide a viewport is on screen, in CSS pixels, for a given render size. */
export function cssSize(rect: RectMm, pixels: { width: number; height: number }) {
	void rect;
	return { width: pixels.width, height: pixels.height };
}

/** A millimetre length as inches, for anything printing a dpi. */
export function inches(mm: Mm): number {
	return mm / PER_INCH;
}
