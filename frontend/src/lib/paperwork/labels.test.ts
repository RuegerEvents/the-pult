/**
 * The one place this beats the drawing it was designed against.
 *
 * On that sheet's Fixtures plan, four HydraPanel labels and two Auro Spot labels print
 * exactly on top of one another and cannot be read. These tests are that case: heads
 * closer together than their names are wide, and what has to come out the other side.
 */

import { describe, expect, it } from 'vitest';

import { layOutLabels, type PlacedLabel } from './labels.js';
import type { Metrics } from './font.js';

/**
 * Metrics with a flat advance per character.
 *
 * The real font is loaded in `sheet.test.ts`, where a figure has to mean something.
 * Here the question is whether two boxes end up apart, and a predictable width makes
 * the overlap arithmetic readable.
 */
const FLAT: Metrics = {
	widthOf: (text, size) => text.length * size * 0.5,
	capRatio: 0.71,
	ascentRatio: 1.07,
	descentRatio: 0.29,
	bytes: { regular: new Uint8Array(), bold: new Uint8Array() }
};

function boxesOverlap(a: PlacedLabel, b: PlacedLabel): boolean {
	return (
		a.box.x < b.box.x + b.box.w &&
		b.box.x < a.box.x + a.box.w &&
		a.box.y < b.box.y + b.box.h &&
		b.box.y < a.box.y + a.box.h
	);
}

describe('laying out labels', () => {
	it('leaves a lone label above its head and draws no leader', () => {
		const [label] = layOutLabels([{ anchor: { x: 50, y: 50 }, lines: ['Spikie 1'] }], FLAT);
		expect(label.at.x).toBeCloseTo(50);
		expect(label.at.y).toBeLessThan(50);
		expect(label.leader).toBe(false);
	});

	it('pushes four labels on one point apart until they can be read', () => {
		// The HydraPanel case, exactly: four heads at the same place.
		const placed = layOutLabels(
			[1, 2, 3, 4].map(() => ({ anchor: { x: 50, y: 50 }, lines: ['HydraPanel 1'] })),
			FLAT
		);
		expect(placed).toHaveLength(4);
		for (let i = 0; i < placed.length; i++) {
			for (let j = i + 1; j < placed.length; j++) {
				expect(boxesOverlap(placed[i], placed[j])).toBe(false);
			}
		}
	});

	it('gives a moved label a line back to its head', () => {
		const placed = layOutLabels(
			[1, 2, 3, 4].map(() => ({ anchor: { x: 50, y: 50 }, lines: ['HydraPanel 1'] })),
			FLAT
		);
		expect(placed.some((label) => label.leader)).toBe(true);
	});

	it('spreads a row of heads sideways and a stack upwards', () => {
		// Six lights along a bar: the labels have room above, so the tidy answer is to
		// stagger them vertically rather than to fan them across the drawing.
		const row = layOutLabels(
			[0, 1, 2, 3, 4, 5].map((i) => ({
				anchor: { x: 40 + i * 3, y: 50 },
				lines: ['Titan Tube 1']
			})),
			FLAT
		);
		const spreadY = Math.max(...row.map((l) => l.at.y)) - Math.min(...row.map((l) => l.at.y));
		expect(spreadY).toBeGreaterThan(0);
	});

	it('never leaves a label sitting on the head it names', () => {
		// Four lanterns on a boom are one point in plan. Separation stacks their labels
		// and, before this rule, pushed the bottom one straight down onto the symbol.
		const anchor = { x: 50, y: 50 };
		const placed = layOutLabels(
			[1, 2, 3, 4].map(() => ({ anchor: { ...anchor }, lines: ['Side SL 1', '1/23'] })),
			FLAT
		);
		for (const label of placed) {
			expect(label.box.y + label.box.h).toBeLessThanOrEqual(anchor.y);
		}
	});

	it('is deterministic, because the preview is a promise', () => {
		const requests = [0, 1, 2, 3, 4].map((i) => ({
			anchor: { x: 50 + (i % 2), y: 50 },
			lines: ['Spot', String(i)]
		}));
		const once = JSON.stringify(layOutLabels(requests, FLAT));
		const twice = JSON.stringify(layOutLabels(requests, FLAT));
		expect(once).toEqual(twice);
	});

	it('keeps labels inside the frame they belong to', () => {
		const bounds = { x: 0, y: 0, w: 60, h: 60 };
		const placed = layOutLabels(
			[1, 2, 3].map(() => ({ anchor: { x: 2, y: 2 }, lines: ['A very long fixture name'] })),
			FLAT,
			{ bounds }
		);
		for (const label of placed) {
			expect(label.box.x).toBeGreaterThanOrEqual(bounds.x - 0.001);
			expect(label.box.y).toBeGreaterThanOrEqual(bounds.y - 0.001);
		}
	});

	it('drops an empty line rather than leaving a gap', () => {
		// A fixture with no number should not get a blank middle line pushing its
		// address away from its name.
		const [label] = layOutLabels(
			[{ anchor: { x: 10, y: 10 }, lines: ['Spikie 1', '', '1/271'] }],
			FLAT
		);
		expect(label.lines).toEqual(['Spikie 1', '1/271']);
	});

	it('ignores a request with nothing to say', () => {
		expect(layOutLabels([{ anchor: { x: 0, y: 0 }, lines: ['', ''] }], FLAT)).toEqual([]);
	});
});
