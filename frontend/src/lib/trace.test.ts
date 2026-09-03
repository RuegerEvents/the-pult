import { describe, expect, it } from 'vitest';

import { Trace, Traces, sparkline } from './trace.js';

describe('Trace', () => {
	it('keeps readings oldest first and drops the oldest past its length', () => {
		const trace = new Trace(3);
		[1, 2, 3, 4].forEach((v, i) => trace.push(v, i));
		expect(trace.points).toEqual([2, 3, 4]);
	});

	/**
	 * The failure this exists to prevent: a station that has gone quiet keeps being
	 * rendered with the last figure it published, and a trace that took every render
	 * as a reading would draw a flat line — which reads as a machine steadily working
	 * rather than one that has stopped talking.
	 */
	it('the same window seen twice is one reading', () => {
		const trace = new Trace();
		trace.push(4, 1000);
		trace.push(4, 1000);
		trace.push(4, 1000);
		expect(trace.points).toEqual([4]);
	});

	it('and the same value in a new window is a new reading', () => {
		const trace = new Trace();
		trace.push(4, 1000);
		trace.push(4, 2000);
		expect(trace.points).toEqual([4, 4]);
	});
});

describe('Traces', () => {
	it('holds one line per key', () => {
		const traces = new Traces();
		traces.push('house', 4, 1);
		traces.push('roof', 9, 1);
		expect(traces.points('house')).toEqual([4]);
		expect(traces.points('roof')).toEqual([9]);
	});

	/// A connector taken out of the show, or a tab that closed, must not leave a line.
	it('forgets what is no longer there', () => {
		const traces = new Traces();
		traces.push('house', 4, 1);
		traces.push('roof', 9, 1);

		traces.keep(['house']);

		expect(traces.points('house')).toEqual([4]);
		expect(traces.points('roof')).toEqual([]);
	});
});

describe('sparkline', () => {
	it('one reading is not a trend and draws nothing', () => {
		expect(sparkline([5], 60, 16)).toBeNull();
		expect(sparkline([], 60, 16)).toBeNull();
	});

	/**
	 * Scaled from zero rather than from the lowest point: these are costs and rates,
	 * and a frame time bouncing between 4.0 and 4.1 ms has to read as flat rather than
	 * as an alarming sawtooth.
	 */
	it('is flat when the figures barely move, because the floor is zero', () => {
		const d = sparkline([4.0, 4.1, 4.0], 60, 16)!;
		const ys = [...d.matchAll(/[ML][\d.]+,([\d.]+)/g)].map((m) => Number(m[1]));
		expect(Math.max(...ys) - Math.min(...ys)).toBeLessThan(1);
	});

	it('spans the box from the bottom to the highest reading', () => {
		const d = sparkline([0, 10], 60, 16)!;
		expect(d).toBe('M0.0,16.0 L60.0,0.0');
	});

	/// A given ceiling is what lets two browsers' lines be compared with each other.
	it('a ceiling pins the top, and a reading past it draws along it', () => {
		const d = sparkline([30, 60, 120], 60, 16, 60)!;
		expect(d).toBe('M0.0,8.0 L30.0,0.0 L60.0,0.0');
	});
});
