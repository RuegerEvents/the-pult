/**
 * Pinching the timeline: a trackpad's pinch, which a browser hands over as a wheel with
 * the control key held, and two fingers on a touch screen.
 *
 * Both zoom about where the fingers are rather than the middle of the panel — what is
 * under them stays under them — and two fingers on glass also pan, since the point
 * between them moving is the timeline moving. Pure, so the arithmetic is tested and
 * `Lanes.svelte` only routes events here.
 */

import { clampView, type View } from '$lib/waveform.js';

/** How strongly a wheel delta zooms: about 1.28× for one notch of a mouse wheel. */
const PER_PIXEL = 0.0025;
/** A trackpad pinch's deltas are a few pixels each and many per gesture. */
const PER_PINCH_PIXEL = 0.01;

/**
 * The factor a wheel event zooms the span by: above 1 widens (zooms out). Continuous,
 * so a pinch made of forty small events lands where one made of four would; a
 * `deltaMode` of lines is a mouse that counts notches, read as forty pixels each.
 */
export function wheelZoomFactor(deltaY: number, deltaMode: number, pinch: boolean): number {
	const pixels = deltaMode === 1 ? deltaY * 40 : deltaMode === 2 ? deltaY * 800 : deltaY;
	const factor = Math.exp(pixels * (pinch ? PER_PINCH_PIXEL : PER_PIXEL));
	// Never more than doubling or halving in one event, however wild the device.
	return Math.min(2, Math.max(0.5, factor));
}

/** Where two fingers came down, and the view they came down on. */
export type PinchStart = {
	view: View;
	/** The time under the point between the fingers, as it was. */
	atMs: number;
	/** How far apart the fingers were, in pixels. */
	distance: number;
};

export function pinchStart(view: View, width: number, a: { x: number }, b: { x: number }): PinchStart {
	const mid = (a.x + b.x) / 2;
	return {
		view,
		atMs: view.startMs + (mid / width) * view.spanMs,
		distance: Math.max(1, Math.abs(a.x - b.x))
	};
}

/**
 * The view while two fingers are down: the span scaled by how far they have spread, and
 * the time that was between them kept between them wherever they have moved to.
 */
export function pinchView(
	start: PinchStart,
	width: number,
	durationMs: number,
	a: { x: number },
	b: { x: number }
): View {
	const distance = Math.max(1, Math.abs(a.x - b.x));
	const spanMs = (start.view.spanMs * start.distance) / distance;
	const mid = (a.x + b.x) / 2;
	return clampView({ startMs: start.atMs - (mid / width) * spanMs, spanMs }, durationMs);
}
