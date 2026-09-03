import { describe, expect, it } from 'vitest';

import { FrameMeter, SLOW_FPS, STALL_MS, fpsOf, struggling } from './stats.js';
import type { BrowserFrames } from './generated/index.js';

/** A clock the test moves by hand, so no window is measured against a real one. */
function aClock() {
	let at = 0;
	return { now: () => at, advance: (ms: number) => (at += ms) };
}

/** Sixty frames of 16.67 ms, which is a page keeping up. */
function comfortable(): BrowserFrames {
	return {
		mean_ms: 16.7,
		max_ms: 18,
		evaluating_mean_ms: 0.4,
		evaluating_max_ms: 1.1,
		parameters: 240,
		frames: 120,
		window_ms: 2000
	};
}

describe('FrameMeter', () => {
	it('a window with no frames in it is absent rather than zero', () => {
		const clock = aClock();
		const meter = new FrameMeter(clock.now);
		clock.advance(2000);
		// A page with no light on it, or a tab the browser stopped serving. Zero
		// would read as "instant"; nothing is what actually happened.
		expect(meter.close()).toBeNull();
	});

	it('averages the frames it was given and keeps the worst of them', () => {
		const clock = aClock();
		const meter = new FrameMeter(clock.now);

		meter.frame(16, 0.5, 100);
		meter.frame(16, 0.5, 100);
		meter.frame(200, 4, 100);
		clock.advance(2000);

		const sample = meter.close();
		expect(sample).not.toBeNull();
		expect(sample!.frames).toBe(3);
		expect(sample!.mean_ms).toBeCloseTo(77.33, 1);
		// The mean is what a page averaged; the worst is what an operator saw.
		expect(sample!.max_ms).toBe(200);
		expect(sample!.evaluating_max_ms).toBe(4);
		expect(sample!.window_ms).toBe(2000);
	});

	it('closing starts the next window rather than accumulating for ever', () => {
		const clock = aClock();
		const meter = new FrameMeter(clock.now);

		meter.frame(200, 8, 50);
		clock.advance(2000);
		meter.close();

		meter.frame(16, 0.5, 50);
		clock.advance(2000);
		const sample = meter.close()!;
		expect(sample.frames).toBe(1);
		expect(sample.max_ms).toBe(16);
	});
});

describe('what counts as struggling', () => {
	it('a page keeping up says nothing', () => {
		expect(struggling(comfortable())).toBeNull();
	});

	it('a window that drew nothing cannot be judged', () => {
		expect(struggling(null)).toBeNull();
	});

	/**
	 * The rule that would otherwise fire on every page that has just opened: five
	 * frames in the last fifth of a second is a frame rate of five, and a browser
	 * waking up is not a browser in trouble.
	 */
	it('a handful of frames is not enough to accuse a page over', () => {
		const waking: BrowserFrames = { ...comfortable(), frames: 4, window_ms: 2000 };
		expect(struggling(waking)).toBeNull();
	});

	it('a sustained low frame rate is worth telling the other consoles about', () => {
		const slow: BrowserFrames = { ...comfortable(), frames: 24, window_ms: 2000 };
		expect(fpsOf(slow)).toBeLessThan(SLOW_FPS);
		expect(struggling(slow)).toMatch(/12\.0 fps over 240 parameters/);
	});

	/**
	 * The fault a mean hides. One 300 ms frame inside a window averaging 17 ms is a
	 * visible jerk and an unremarkable average, so the worst frame gets its own rule.
	 */
	it('a single stall is caught even when the average is comfortable', () => {
		const stalled: BrowserFrames = { ...comfortable(), max_ms: STALL_MS + 200 };
		expect(struggling(stalled)).toMatch(/stalled for 300 ms/);
	});
});
