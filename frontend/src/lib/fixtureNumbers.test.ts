import { describe, expect, it } from 'vitest';
import { byNumber, numberTheRest, parseFixtureNumber, sharedNumbers } from './fixtureNumbers.js';

const f = (id: string, fixture_number: number | null) => ({ id, fixture_number });

describe('fixture numbers', () => {
	it('marks a number two fixtures carry', () => {
		expect(sharedNumbers([f('a', 1), f('b', 2), f('c', 1), f('d', null), f('e', null)])).toEqual(new Set([1]));
	});

	it('numbers the rest in patch order, on from the highest', () => {
		expect(numberTheRest([f('a', null), f('b', 101), f('c', null)])).toEqual([
			{ id: 'a', number: 102 },
			{ id: 'c', number: 103 }
		]);
		expect(numberTheRest([f('a', null), f('b', null)])).toEqual([
			{ id: 'a', number: 1 },
			{ id: 'b', number: 2 }
		]);
	});

	it('lists by number, patch order within one, unnumbered last', () => {
		const ids = byNumber([f('a', null), f('b', 3), f('c', 1), f('d', 3)]).map((x) => x.id);
		expect(ids).toEqual(['c', 'b', 'd', 'a']);
	});

	it('reads a typed number, a cleared one, and refuses the rest', () => {
		expect(parseFixtureNumber(' 12 ')).toBe(12);
		expect(parseFixtureNumber('')).toBeNull();
		expect(parseFixtureNumber('0')).toBeUndefined();
		expect(parseFixtureNumber('1.5')).toBeUndefined();
		expect(parseFixtureNumber('x')).toBeUndefined();
	});
});
