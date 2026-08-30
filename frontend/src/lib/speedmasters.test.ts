import { describe, it, expect } from 'vitest';

import {
	beatPhase,
	bpmFromTaps,
	effectiveHz,
	sinceLastGap,
	tidyBpm,
	TAP_RESTART_MS
} from './speedmasters.js';

describe('tapping a tempo', () => {
	it('needs two taps before it will say anything', () => {
		expect(bpmFromTaps([])).toBeNull();
		expect(bpmFromTaps([1000])).toBeNull();
		expect(bpmFromTaps([1000, 1500])).not.toBeNull();
	});

	it('reads half-second taps as 120 bpm', () => {
		expect(bpmFromTaps([0, 500, 1000, 1500])).toBeCloseTo(120, 5);
	});

	it('reads one-second taps as 60 bpm', () => {
		expect(bpmFromTaps([0, 1000, 2000])).toBeCloseTo(60, 5);
	});

	/**
	 * A hand is not a metronome. At 120 bpm a single 40 ms slip is 10 bpm, which is
	 * plainly audible, so the average is taken over the run rather than the last gap.
	 */
	it('averages an unsteady hand rather than following the last tap', () => {
		const wobbly = bpmFromTaps([0, 520, 980, 1510, 2000]);
		expect(wobbly).toBeCloseTo(120, 0);

		const lastGapOnly = 60_000 / (2000 - 1510);
		expect(Math.abs((wobbly ?? 0) - 120)).toBeLessThan(Math.abs(lastGapOnly - 120));
	});

	/**
	 * The gap is the operator stopping, and counting it as an interval would drag
	 * the average down for the next several taps.
	 */
	it('starts again after a long pause', () => {
		const taps = [0, 500, 1000, 1000 + TAP_RESTART_MS + 1, 1000 + TAP_RESTART_MS + 501];
		expect(bpmFromTaps(taps)).toBeCloseTo(120, 5);
		expect(sinceLastGap(taps)).toHaveLength(2);
	});

	it('a pause with nothing after it leaves too little to say', () => {
		expect(bpmFromTaps([0, 500, 1000, 9999])).toBeNull();
	});

	it('rounds to something an operator would read out', () => {
		expect(tidyBpm(127.98765)).toBe(128);
		expect(tidyBpm(119.94)).toBe(119.9);
	});
});

describe('what a master asks of an effect', () => {
	const master = (over: Partial<{ bpm: number; multiplier: number; running: boolean }> = {}) => ({
		bpm: 120,
		multiplier: 1,
		running: true,
		...over
	});

	it('turns beats a minute into cycles a second', () => {
		expect(effectiveHz(master())).toBe(2);
		expect(effectiveHz(master({ bpm: 60 }))).toBe(1);
	});

	it('lets one tempo drive a half-speed sweep and a double-speed chase', () => {
		expect(effectiveHz(master({ multiplier: 0.5 }))).toBe(1);
		expect(effectiveHz(master({ multiplier: 2 }))).toBe(4);
	});

	/**
	 * Stopping a chase should freeze the look, not turn the lights off — so a stopped
	 * master is a rate of zero, which the engine renders as a hold at the phase.
	 */
	it('is zero when stopped, which holds rather than drops', () => {
		expect(effectiveHz(master({ running: false }))).toBe(0);
	});
});

describe('where the beat is', () => {
	const master = { bpm: 120, multiplier: 0.5, running: true, t0: 1000 };

	it('runs a whole cycle between anchors', () => {
		// 120 bpm halved is one cycle a second.
		expect(beatPhase(master, 1000)).toBeCloseTo(0, 5);
		expect(beatPhase(master, 1250)).toBeCloseTo(0.25, 5);
		expect(beatPhase(master, 1500)).toBeCloseTo(0.5, 5);
		expect(beatPhase(master, 2000)).toBeCloseTo(0, 5);
	});

	/**
	 * Two consoles do not agree perfectly about the time, so being asked for a phase
	 * before the anchor is ordinary. The dot belongs at the top of the beat, not at a
	 * negative position.
	 */
	it('wraps rather than going negative before the anchor', () => {
		expect(beatPhase(master, 750)).toBeCloseTo(0.75, 5);
	});

	it('sits still while the master is stopped', () => {
		expect(beatPhase({ ...master, running: false }, 999_999)).toBe(0);
	});
});
