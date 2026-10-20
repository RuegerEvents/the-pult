import { describe, expect, it } from 'vitest';
import { pinchStart, pinchView, wheelZoomFactor } from './pinch.js';

const view = { startMs: 10_000, spanMs: 20_000 };
const width = 1000;
const duration = 120_000;

describe('a trackpad pinch', () => {
	it('zooms in for a pinch together and out for a pinch apart, gently', () => {
		expect(wheelZoomFactor(-4, 0, true)).toBeLessThan(1);
		expect(wheelZoomFactor(4, 0, true)).toBeGreaterThan(1);
		expect(wheelZoomFactor(4, 0, true)).toBeLessThan(1.1);
	});

	it('lands in the same place in forty small steps as in four large ones', () => {
		const small = Array.from({ length: 40 }, () => wheelZoomFactor(1, 0, true)).reduce((a, b) => a * b, 1);
		const large = Array.from({ length: 4 }, () => wheelZoomFactor(10, 0, true)).reduce((a, b) => a * b, 1);
		expect(small).toBeCloseTo(large, 6);
	});

	it('reads a notched mouse wheel as a notch and never jumps more than twice', () => {
		expect(wheelZoomFactor(1, 1, false)).toBeGreaterThan(1.05);
		expect(wheelZoomFactor(10_000, 0, false)).toBe(2);
		expect(wheelZoomFactor(-10_000, 0, false)).toBe(0.5);
	});
});

describe('two fingers on glass', () => {
	it('keeps the time between the fingers between them while they spread', () => {
		const start = pinchStart(view, width, { x: 400 }, { x: 600 });
		expect(start.atMs).toBe(20_000);
		const next = pinchView(start, width, duration, { x: 300 }, { x: 700 });
		expect(next.spanMs).toBe(10_000);
		// The point between the fingers is still at x = 500, and still 20 s.
		expect(next.startMs + (500 / width) * next.spanMs).toBeCloseTo(20_000, 6);
	});

	it('pans when both fingers move together', () => {
		const start = pinchStart(view, width, { x: 400 }, { x: 600 });
		const next = pinchView(start, width, duration, { x: 500 }, { x: 700 });
		expect(next.spanMs).toBe(20_000);
		// 20 s is now under x = 600: the timeline moved right with the fingers.
		expect(next.startMs + (600 / width) * next.spanMs).toBeCloseTo(20_000, 6);
	});

	it('survives fingers that meet', () => {
		const start = pinchStart(view, width, { x: 500 }, { x: 500 });
		const next = pinchView(start, width, duration, { x: 500 }, { x: 500 });
		expect(Number.isFinite(next.spanMs)).toBe(true);
	});
});
