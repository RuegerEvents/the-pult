import { describe as suite, expect, it } from 'vitest';

import {
	describe,
	emptyTest,
	force,
	forcedCount,
	forcedIn,
	isEmpty,
	isSolo,
	percent,
	range,
	release,
	releaseUniverse,
	setSolo,
	shifted
} from './dmxtest.js';

suite('forcing channels', () => {
	it('forces a channel and lets it go again', () => {
		const forced = force(emptyTest(), 3, [5], 255);
		expect(forcedIn(forced, 3)).toEqual({ 5: 255 });
		expect(isEmpty(release(forced, 3, [5]))).toBe(true);
	});

	it('does not change the test it was handed', () => {
		const before = force(emptyTest(), 1, [1], 10);
		force(before, 1, [1, 2], 200);
		release(before, 1, [1]);
		expect(forcedIn(before, 1)).toEqual({ 1: 10 });
	});

	it('keeps universes ascending, whichever order they were forced in', () => {
		let test = force(emptyTest(), 9, [1], 1);
		test = force(test, 2, [1], 1);
		expect(test.universes.map((each) => each.universe)).toEqual([2, 9]);
	});

	it('forces a channel to zero, which is not the same as not forcing it', () => {
		const forced = force(emptyTest(), 1, [4], 0);
		expect(forcedIn(forced, 1)).toEqual({ 4: 0 });
		expect(forcedCount(forced)).toBe(1);
	});

	it('clamps a level into a byte and refuses a channel outside the universe', () => {
		expect(forcedIn(force(emptyTest(), 1, [1, 2], 999), 1)).toEqual({ 1: 255, 2: 255 });
		expect(forcedIn(force(emptyTest(), 1, [1], -4), 1)).toEqual({ 1: 0 });
		expect(isEmpty(force(emptyTest(), 1, [0, 513], 255))).toBe(true);
	});

	it('drops a universe that forces nothing, but keeps a solo one', () => {
		const solo = setSolo(emptyTest(), 4, true);
		expect(isSolo(solo, 4)).toBe(true);
		expect(solo.universes).toHaveLength(1);
		expect(isEmpty(setSolo(solo, 4, false))).toBe(true);
	});

	it('lets go of one universe without touching another', () => {
		let test = force(emptyTest(), 1, [1], 1);
		test = force(test, 2, [1], 1);
		expect(releaseUniverse(test, 1).universes.map((each) => each.universe)).toEqual([2]);
	});
});

suite('selecting channels', () => {
	it('says a selection the way an operator would', () => {
		expect(describe([1])).toBe('1');
		expect(describe([5, 6])).toBe('5, 6');
		expect(describe([5, 6, 7, 8])).toBe('5–8');
		expect(describe([12, 1, 5, 6, 7, 5])).toBe('1, 5–7, 12');
		expect(describe([])).toBe('');
	});

	it('takes a range whichever way round it was dragged', () => {
		expect(range(3, 5)).toEqual([3, 4, 5]);
		expect(range(5, 3)).toEqual([3, 4, 5]);
		expect(range(510, 600)).toEqual([510, 511, 512]);
	});

	it('walks a selection along, and refuses to walk it off the end', () => {
		expect(shifted([4, 5], 1)).toEqual([5, 6]);
		expect(shifted([1], -1)).toBeNull();
		expect(shifted([511, 512], 1)).toBeNull();
	});

	it('reads a level as a percentage', () => {
		expect(percent(0)).toBe('0%');
		expect(percent(255)).toBe('100%');
		expect(percent(128)).toBe('50%');
	});
});
