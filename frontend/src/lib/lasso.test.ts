import { describe, expect, it } from 'vitest';

import { boxOf, intersects } from './lasso.js';

describe('the lasso', () => {
	it('makes the same box whichever way it was dragged', () => {
		expect(boxOf(10, 20, 0, 5)).toEqual({ left: 0, top: 5, right: 10, bottom: 20 });
		expect(boxOf(0, 5, 10, 20)).toEqual(boxOf(10, 20, 0, 5));
	});

	it('touches what it overlaps, edges included, and nothing beside it', () => {
		const lasso = boxOf(0, 0, 100, 30);
		expect(intersects(lasso, { left: 10, top: 25, right: 200, bottom: 50 })).toBe(true);
		expect(intersects(lasso, { left: 10, top: 30, right: 200, bottom: 50 })).toBe(true);
		expect(intersects(lasso, { left: 10, top: 31, right: 200, bottom: 50 })).toBe(false);
	});
});
