import { describe, it, expect, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { clearSelection, pruneSelection, remove, reorder, select, selection, toggle } from './selection.js';

beforeEach(() => clearSelection());

describe('selection', () => {
	it('replaces what was held when one fixture is picked', () => {
		select('a');
		select('b');
		expect(get(selection)).toEqual(['b']);
	});

	it('adds and removes with a toggle', () => {
		toggle('a');
		toggle('b');
		expect(get(selection)).toEqual(['a', 'b']);
		toggle('a');
		expect(get(selection)).toEqual(['b']);
	});

	it('keeps the order things were added in', () => {
		toggle('c');
		toggle('a');
		toggle('b');
		expect(get(selection)).toEqual(['c', 'a', 'b']);
	});

	it('drops fixtures that have left the rig', () => {
		toggle('a');
		toggle('b');
		pruneSelection(['b']);
		expect(get(selection)).toEqual(['b']);
	});

	it('leaves the selection alone when nothing has gone', () => {
		toggle('a');
		const before = get(selection);
		pruneSelection(['a', 'b']);
		// The same array, not an equal one: a new one would re-render every fixture
		// on the plan each time a fixture list is delivered, which is every frame
		// of every fade.
		expect(get(selection)).toBe(before);
	});
});

describe('ordering the selection', () => {
	it('moves one fixture to another place in the order', () => {
		toggle('a');
		toggle('b');
		toggle('c');
		reorder(2, 0);
		expect(get(selection)).toEqual(['c', 'a', 'b']);
	});

	it('moves one down as readily as up', () => {
		toggle('a');
		toggle('b');
		toggle('c');
		reorder(0, 2);
		expect(get(selection)).toEqual(['b', 'c', 'a']);
	});

	it('leaves the order alone for a drop that landed nowhere', () => {
		toggle('a');
		toggle('b');
		const before = get(selection);
		reorder(0, 5);
		reorder(-1, 0);
		reorder(1, 1);
		expect(get(selection)).toBe(before);
	});

	it('drops one fixture and keeps the rest in order', () => {
		toggle('a');
		toggle('b');
		toggle('c');
		remove('b');
		expect(get(selection)).toEqual(['a', 'c']);
	});

	it('leaves the selection alone when asked to drop something not in it', () => {
		toggle('a');
		const before = get(selection);
		remove('z');
		expect(get(selection)).toBe(before);
	});
});
