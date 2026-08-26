import { describe, it, expect, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { clearSelection, pruneSelection, select, selection, toggle } from './selection.js';

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
