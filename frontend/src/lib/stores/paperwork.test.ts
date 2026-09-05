/**
 * The arithmetic behind dragging a block, and the rule for naming a sheet.
 *
 * Both are here rather than in the components that use them because both have a case
 * that is easy to get wrong and impossible to notice by looking: a rectangle dragged
 * through itself, and a duplicate that quietly takes a name something else already has.
 */

import { describe, expect, it } from 'vitest';

import { KEEP_ON_PAPER_MM, MIN_BLOCK_MM, resizedRect, uniqueSheetName } from './paperwork.js';

const BLOCK = { x: 20, y: 20, w: 100, h: 60 };

describe('moving a block', () => {
	it('takes it where the pointer went', () => {
		expect(resizedRect(BLOCK, 'move', 15, -5)).toEqual({ x: 35, y: 15, w: 100, h: 60 });
	});

	it('does not change its size', () => {
		const moved = resizedRect(BLOCK, 'move', 300, 300);
		expect(moved.w).toBe(BLOCK.w);
		expect(moved.h).toBe(BLOCK.h);
	});

	it('leaves a corner on the paper, so it can be grabbed again', () => {
		const A3 = { w: 420, h: 297 };
		const right = resizedRect(BLOCK, 'move', 5000, 0, A3);
		expect(right.x).toBe(A3.w - KEEP_ON_PAPER_MM);
		const left = resizedRect(BLOCK, 'move', -5000, 0, A3);
		expect(left.x + left.w).toBe(KEEP_ON_PAPER_MM);
	});

	it('allows a block to hang over the edge, which laying a sheet out goes through', () => {
		const A3 = { w: 420, h: 297 };
		const out = resizedRect(BLOCK, 'move', 350, 0, A3);
		expect(out.x).toBe(370);
		expect(out.x + out.w).toBeGreaterThan(A3.w);
	});

	it('is unclamped when nothing said how big the paper is', () => {
		expect(resizedRect(BLOCK, 'move', 5000, 0).x).toBe(5020);
	});
});

describe('resizing a block', () => {
	it('moves the right edge and leaves the left one', () => {
		expect(resizedRect(BLOCK, 'e', 20, 0)).toEqual({ x: 20, y: 20, w: 120, h: 60 });
	});

	it('moves the left edge and leaves the right one', () => {
		// The right edge is at 120 before and after: x + w is unchanged.
		const out = resizedRect(BLOCK, 'w', 10, 0);
		expect(out.x).toBe(30);
		expect(out.x + out.w).toBe(BLOCK.x + BLOCK.w);
	});

	it('takes a corner in both directions at once', () => {
		expect(resizedRect(BLOCK, 'se', 10, 10)).toEqual({ x: 20, y: 20, w: 110, h: 70 });
	});

	it('stops rather than turning inside out', () => {
		// Dragging the left edge far past the right one. A negative width draws as
		// nothing and can never be grabbed again to fix it, which is why this is the
		// case worth a test.
		const out = resizedRect(BLOCK, 'w', 500, 0);
		expect(out.w).toBe(MIN_BLOCK_MM);
		expect(out.x + out.w).toBe(BLOCK.x + BLOCK.w);
	});

	it('stops in the other direction too', () => {
		const out = resizedRect(BLOCK, 'n', 500, 500);
		expect(out.h).toBe(MIN_BLOCK_MM);
		expect(out.y + out.h).toBe(BLOCK.y + BLOCK.h);
	});

	it('never shrinks past the minimum from the far edge either', () => {
		expect(resizedRect(BLOCK, 'e', -500, 0).w).toBe(MIN_BLOCK_MM);
		expect(resizedRect(BLOCK, 's', 0, -500).h).toBe(MIN_BLOCK_MM);
	});
});

describe('naming a sheet', () => {
	it('leaves a free name alone', () => {
		expect(uniqueSheetName('Plan', [{ name: 'Rig' }])).toBe('Plan');
	});

	it('numbers a taken one', () => {
		expect(uniqueSheetName('Plan', [{ name: 'Plan' }])).toBe('Plan 2');
	});

	it('keeps counting past the numbers already taken', () => {
		expect(uniqueSheetName('Plan', [{ name: 'Plan' }, { name: 'Plan 2' }])).toBe('Plan 3');
	});

	it('is not confused by a gap in the numbering', () => {
		expect(uniqueSheetName('Plan', [{ name: 'Plan' }, { name: 'Plan 3' }])).toBe('Plan 2');
	});
});
