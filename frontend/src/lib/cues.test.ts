import { describe, it, expect } from 'vitest';

import { insertNumber, reorderCueIds } from './cues.js';

describe('numbering an inserted cue', () => {
	/**
	 * The midpoint, which is what fractional cue numbers have always been for. A list
	 * can be inserted into repeatedly without renumbering everything below it, so a
	 * cue an operator calls "cue 5" stays cue 5.
	 */
	it('lands between the two cues it goes between', () => {
		expect(insertNumber(1, 2)).toBe(1.5);
		expect(insertNumber(1.5, 2)).toBe(1.75);
		expect(insertNumber(1.75, 2)).toBe(1.875);
	});

	it('works on a list that does not start at one', () => {
		expect(insertNumber(10, 20)).toBe(15);
		expect(insertNumber(2.25, 2.5)).toBe(2.375);
	});

	/** Appending to a list that ends at 4.75 should give 5, not 5.75. */
	it('takes the next whole number when there is nothing after it', () => {
		expect(insertNumber(3)).toBe(4);
		expect(insertNumber(4.75)).toBe(5);
		expect(insertNumber(1.001)).toBe(2);
	});
});

describe('dragging a cue somewhere else', () => {
	const ids = ['a', 'b', 'c', 'd'];

	it('moves one down the list', () => {
		expect(reorderCueIds(ids, 0, 2)).toEqual(['b', 'c', 'a', 'd']);
	});

	it('moves one up the list', () => {
		expect(reorderCueIds(ids, 3, 1)).toEqual(['a', 'd', 'b', 'c']);
	});

	it('leaves the list alone when nothing moved', () => {
		expect(reorderCueIds(ids, 2, 2)).toEqual(ids);
	});

	it('clamps a drop past either end rather than losing the cue', () => {
		expect(reorderCueIds(ids, 0, 99)).toEqual(['b', 'c', 'd', 'a']);
		expect(reorderCueIds(ids, 3, -5)).toEqual(['d', 'a', 'b', 'c']);
	});

	it('ignores a drag from outside the list', () => {
		expect(reorderCueIds(ids, 9, 0)).toEqual(ids);
	});

	/**
	 * The ids are the order — `cue_ids` is `ordered` in the schema — so a drag
	 * rewrites that and leaves `Cue.number` alone. Renumbering on every drag would
	 * make a cue stop being the number an operator calls it because somebody moved a
	 * different one.
	 */
	it('never changes how many cues there are', () => {
		for (let from = 0; from < ids.length; from++) {
			for (let to = 0; to < ids.length; to++) {
				expect(reorderCueIds(ids, from, to)).toHaveLength(ids.length);
				expect(new Set(reorderCueIds(ids, from, to))).toEqual(new Set(ids));
			}
		}
	});
});
