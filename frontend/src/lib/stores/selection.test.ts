import { describe, it, expect, beforeEach } from 'vitest';
import { get, writable } from 'svelte/store';

import type { DataRoot } from '$lib/ws/data.js';
import type { Fixture } from '$lib/generated/index.js';
import { initShowStores } from './show.js';
import {
	addClause,
	clearSelection,
	freeze,
	query,
	remove,
	reorder,
	select,
	selection,
	setOrder,
	toggle
} from './selection.js';

/**
 * A rig for the store to select out of.
 *
 * The selection is derived from a query *and the fixtures*, so a test without a rig
 * would be testing that nothing matches nothing. That is itself the property which
 * retired `pruneSelection` — a fixture that has left the rig cannot stay selected —
 * so the rig has to be real for any of this to mean anything.
 */
const rig = writable<Fixture[]>([]);

const fixture = (id: string, x: number, typeId = 'par'): Fixture => ({
	id,
	name: id.toUpperCase(),
	fixture_type_id: typeId,
	address: { Dmx: { universe: 1, address: 1 } },
	position: { Point: { x, y: 5, z: 0 } },
	live_values: {},
	live_effects: {},
	live_fades: {}
});

initShowStores({
	fixtures: { subscribeDeep: (cb: (v: Fixture[]) => void) => rig.subscribe(cb) }
} as unknown as DataRoot);

beforeEach(() => {
	rig.set([fixture('a', 0), fixture('b', 1), fixture('c', 2), fixture('d', 3, 'mover')]);
	clearSelection();
});

describe('picking by hand', () => {
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

	it('drops one with the × beside it', () => {
		toggle('a');
		toggle('b');
		remove('a');
		expect(get(selection)).toEqual(['b']);
	});

	it('reorders by dragging', () => {
		toggle('a');
		toggle('b');
		toggle('c');
		reorder(2, 0);
		expect(get(selection)).toEqual(['c', 'a', 'b']);
	});

	it('ignores a drag that lands outside the list', () => {
		toggle('a');
		toggle('b');
		reorder(0, 9);
		expect(get(selection)).toEqual(['a', 'b']);
	});

	/**
	 * Shift-clicking across a truss should not grow a chain of one-id clauses; the
	 * trailing hand-picked clause is extended instead.
	 */
	it('keeps hand-picking to a single clause', () => {
		toggle('a');
		toggle('b');
		toggle('c');
		expect(get(query).clauses).toHaveLength(1);
	});
});

describe('selecting by asking a question', () => {
	it('picks out everything of a type, in rig order', () => {
		addClause('Add', { kind: 'OfType', typeId: 'par' });
		expect(get(selection)).toEqual(['a', 'b', 'c']);
	});

	/**
	 * The whole point, and the reason the spec asks for it: a rig that changes under
	 * a selection is the normal case at a festival, and the selection should still
	 * mean what it said.
	 */
	it('picks up a fixture patched after the question was asked', () => {
		addClause('Add', { kind: 'OfType', typeId: 'par' });
		expect(get(selection)).toHaveLength(3);

		rig.update((f) => [...f, fixture('e', 4)]);
		expect(get(selection)).toEqual(['a', 'b', 'c', 'e']);
	});

	/**
	 * And the other direction, which is what retired `pruneSelection`: a deleted
	 * fixture stops matching, so it leaves without anything having to notice.
	 */
	it('lets go of a fixture that has left the rig', () => {
		addClause('Add', { kind: 'Everything' });
		expect(get(selection)).toHaveLength(4);

		rig.update((f) => f.filter((x) => x.id !== 'b'));
		expect(get(selection)).toEqual(['a', 'c', 'd']);
	});

	it('lets go of a hand-picked fixture that has left too', () => {
		toggle('a');
		toggle('b');
		rig.update((f) => f.filter((x) => x.id !== 'a'));
		expect(get(selection)).toEqual(['b']);
	});

	it('adjusts a question by hand without losing the question', () => {
		addClause('Add', { kind: 'OfType', typeId: 'par' });
		toggle('d');
		expect(get(selection)).toEqual(['a', 'b', 'c', 'd']);

		// The geometry is still there, so a new par still arrives.
		rig.update((f) => [...f, fixture('e', 4)]);
		expect(get(selection)).toContain('e');
	});

	it('removes a fixture the question picked, by dropping it', () => {
		addClause('Add', { kind: 'OfType', typeId: 'par' });
		toggle('b');
		expect(get(selection)).toEqual(['a', 'c']);
	});

	it('orders along an axis', () => {
		addClause('Add', { kind: 'Everything' });
		setOrder({ kind: 'ByAxis', axis: 'x', descending: true });
		expect(get(selection)).toEqual(['d', 'c', 'b', 'a']);
	});
});

describe('freezing a question into a list', () => {
	/**
	 * The way out of a query that is nearly right. Reordering one is not a thing you
	 * can coherently do — a hand-made order is an answer about particular fixtures,
	 * and the question outlives them — so dragging freezes first.
	 */
	it('keeps what is selected and stops following the rig', () => {
		addClause('Add', { kind: 'OfType', typeId: 'par' });
		freeze();
		expect(get(selection)).toEqual(['a', 'b', 'c']);

		rig.update((f) => [...f, fixture('e', 4)]);
		expect(get(selection)).toEqual(['a', 'b', 'c']);
	});

	it('dragging a query freezes it', () => {
		addClause('Add', { kind: 'Everything' });
		setOrder({ kind: 'ByAxis', axis: 'x' });
		reorder(3, 0);
		expect(get(selection)).toEqual(['d', 'a', 'b', 'c']);
		expect(get(query).order).toEqual({ kind: 'Manual' });
	});
});
