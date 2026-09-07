/**
 * The waveform's arithmetic, and the one format in this console with two readers.
 *
 * The codec test is the important one: `pult_audio::peaks` writes these bytes and this
 * file reads them, so a change to either without the other is a panel drawing noise.
 * It is checked against a buffer built here to the format's own description rather than
 * against a fixture, so the test says what the format *is* rather than what one file
 * happens to contain.
 */

import { describe, expect, it } from 'vitest';

import {
	clampView,
	clock,
	decodePeaks,
	gridLines,
	MIN_SPAN_MS,
	msOf,
	parsePosition,
	snap,
	xOf,
	zoomAt
} from './waveform.js';

/** A peaks file, written the way the station writes one. */
function peaksFile(bins: [number, number][], sampleRate = 48_000, samplesPerBin = 480) {
	const buffer = new ArrayBuffer(18 + bins.length * 4);
	const view = new DataView(buffer);
	view.setUint8(0, 0x50); // P
	view.setUint8(1, 0x4c); // L
	view.setUint8(2, 0x50); // P
	view.setUint8(3, 0x4b); // K
	view.setUint16(4, 1, true);
	view.setUint32(6, sampleRate, true);
	view.setUint32(10, samplesPerBin, true);
	view.setUint32(14, bins.length, true);
	bins.forEach(([low, high], i) => {
		view.setInt16(18 + i * 4, Math.round(low * 32767), true);
		view.setInt16(18 + i * 4 + 2, Math.round(high * 32767), true);
	});
	return buffer;
}

describe('the peaks codec', () => {
	it('reads what the station wrote', () => {
		const peaks = decodePeaks(peaksFile([[-1, 1], [-0.5, 0.25]]));
		expect(peaks).not.toBeNull();
		expect(peaks!.sampleRate).toBe(48_000);
		expect(peaks!.samplesPerBin).toBe(480);
		expect(peaks!.bins.length).toBe(4);
		expect(peaks!.bins[0]).toBeCloseTo(-1, 3);
		expect(peaks!.bins[1]).toBeCloseTo(1, 3);
		expect(peaks!.bins[3]).toBeCloseTo(0.25, 3);
	});

	it('works out the duration from the bins and the rate', () => {
		// 480 samples a bin at 48 kHz is a hundredth of a second, so 300 bins is 3 s.
		const peaks = decodePeaks(peaksFile(Array(300).fill([0, 0])));
		expect(peaks!.durationMs).toBe(3_000);
	});

	it('refuses anything that is not a peaks file, rather than drawing noise', () => {
		expect(decodePeaks(new ArrayBuffer(4))).toBeNull();
		expect(decodePeaks(new TextEncoder().encode('not this at all!!!!!!').buffer)).toBeNull();

		const wrongVersion = peaksFile([[0, 0]]);
		new DataView(wrongVersion).setUint16(4, 99, true);
		expect(decodePeaks(wrongVersion)).toBeNull();

		const truncated = peaksFile(Array(10).fill([0, 0])).slice(0, 30);
		expect(decodePeaks(truncated)).toBeNull();
	});
});

describe('what a view may be', () => {
	it('never lets the right edge run past the end of the song', () => {
		const view = clampView({ startMs: 900_000, spanMs: 10_000 }, 60_000);
		expect(view.startMs).toBe(50_000);
		expect(view.spanMs).toBe(10_000);
	});

	it('never zooms in past two seconds or out past the whole song', () => {
		expect(clampView({ startMs: 0, spanMs: 10 }, 60_000).spanMs).toBe(MIN_SPAN_MS);
		expect(clampView({ startMs: 0, spanMs: 999_999 }, 60_000).spanMs).toBe(60_000);
	});

	it('holds a short song open at the minimum span rather than collapsing it', () => {
		const view = clampView({ startMs: 0, spanMs: 500 }, 400);
		expect(view.spanMs).toBe(MIN_SPAN_MS);
		expect(view.startMs).toBe(0);
	});
});

describe('pixels and milliseconds', () => {
	const view = { startMs: 10_000, spanMs: 20_000 };

	it('round trips', () => {
		expect(xOf(10_000, view, 1_000)).toBe(0);
		expect(xOf(30_000, view, 1_000)).toBe(1_000);
		expect(msOf(500, view, 1_000)).toBe(20_000);
		expect(msOf(xOf(17_345, view, 800), view, 800)).toBe(17_345);
	});

	/**
	 * The rule that makes a wheel feel like a map: what is under the pointer stays
	 * under the pointer. Zooming about the centre instead walks the thing being looked
	 * at off the screen.
	 */
	it('zooms about the point it was given', () => {
		const zoomed = zoomAt(view, 120_000, 20_000, 0.5);
		expect(zoomed.spanMs).toBe(10_000);
		expect(zoomed.startMs).toBe(15_000);
		expect(msOf(xOf(20_000, zoomed, 1_000), zoomed, 1_000)).toBe(20_000);
	});
});

describe('snapping', () => {
	const view = { startMs: 0, spanMs: 10_000 };
	const targets = [
		{ atMs: 1_000, what: 'beat' as const },
		{ atMs: 2_000, what: 'bar' as const },
		{ atMs: 2_040, what: 'marker' as const }
	];

	it('takes nothing when nothing is near', () => {
		expect(snap(6_000, targets, view, 1_000, 6)).toBe(6_000);
	});

	it('takes what is near', () => {
		expect(snap(1_020, targets, view, 1_000, 6)).toBe(1_000);
	});

	/** A marker beats a bar beats a beat, so dragging near a chorus mark lands on it. */
	it('prefers a marker to a bar and a bar to a beat', () => {
		expect(snap(2_010, targets, view, 1_000, 20)).toBe(2_040);
		const withoutMarker = targets.slice(0, 2);
		expect(snap(2_010, withoutMarker, view, 1_000, 20)).toBe(2_000);
	});

	/** "Near" is a distance on the screen, not in the song. */
	it('is measured in pixels, so zooming out snaps to more', () => {
		const wide = { startMs: 0, spanMs: 600_000 };
		expect(snap(3_000, targets, wide, 1_000, 6)).toBe(2_040);
		expect(snap(3_000, targets, view, 1_000, 6)).toBe(3_000);
	});
});

describe('the grid', () => {
	const grid = [{ at_ms: 0, bpm: 120, beats_per_bar: 4 }];

	it('puts a beat every half second at 120 and a bar every four', () => {
		const lines = gridLines(grid, 0, 4_000);
		expect(lines.map((l) => l.atMs)).toEqual([0, 500, 1_000, 1_500, 2_000, 2_500, 3_000, 3_500]);
		expect(lines.filter((l) => l.what === 'bar').map((l) => l.atMs)).toEqual([0, 2_000]);
	});

	it('starts at the visible edge rather than walking the whole song', () => {
		const lines = gridLines(grid, 60_000, 61_000);
		expect(lines[0].atMs).toBe(60_000);
		expect(lines.length).toBe(2);
	});

	it('stops a segment where the next one starts', () => {
		const two = [
			{ at_ms: 0, bpm: 120, beats_per_bar: 4 },
			{ at_ms: 1_000, bpm: 60, beats_per_bar: 4 }
		];
		const lines = gridLines(two, 0, 3_000);
		expect(lines.map((l) => l.atMs)).toEqual([0, 500, 1_000, 2_000]);
	});

	/** More lines than pixels is not a picture. */
	it('is bounded, so a whole act across the panel does not draw ten thousand lines', () => {
		expect(gridLines(grid, 0, 3_600_000, 100).length).toBe(100);
	});

	it('says nothing about a segment with no tempo', () => {
		expect(gridLines([{ at_ms: 0, bpm: 0, beats_per_bar: 4 }], 0, 10_000)).toEqual([]);
	});
});

describe('reading a position', () => {
	it('writes and reads a stopwatch', () => {
		expect(clock(64_200)).toBe('1:04.20');
		expect(clock(-5)).toBe('0:00.00');
		expect(parsePosition('1:04.2')).toBe(64_200);
		expect(parsePosition('4000')).toBe(4_000);
		expect(parsePosition('')).toBeNull();
		expect(parsePosition('what')).toBeNull();
	});
});
