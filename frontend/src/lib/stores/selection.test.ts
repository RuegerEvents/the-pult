import { describe, it, expect, beforeEach } from 'vitest';
import { get, writable } from 'svelte/store';

import type { DataRoot } from '$lib/ws/data.js';
import type { Fixture } from '$lib/generated/index.js';
import { initShowStores } from './show.js';
import { evaluate, idsQuery } from '$lib/selection.js';
import {
	addClause,
	applySelectionEffect,
	asSavedQuery,
	clearSelection,
	freeze,
	query,
	recall,
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
	sensed_values: {},
	live_effects: {},
	live_fades: {},
	home_values: {}
});

initShowStores(
	{ fixtures: { subscribeDeep: (cb: (v: Fixture[]) => void) => rig.subscribe(cb) } } as unknown as DataRoot,
	// The socket is only reached for undo and identify, neither of which a
	// selection test touches.
	{} as never
);

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
		// The dragged order goes into the query, not only into the store beside it, so
		// that saving this selection as a group carries the order to a station that
		// never saw the drag.
		expect(get(query).order).toEqual({ kind: 'Manual', order: ['d', 'a', 'b', 'c'] });
	});
});

describe('what a plugin surface can ask for', () => {
	beforeEach(() => {
		clearSelection();
	});

	it('takes a list of fixtures', () => {
		expect(applySelectionEffect({ selection: { fixtureIds: ['b', 'a'] } })).toBe(true);
		expect(get(selection)).toEqual(['a', 'b']);
	});

	/**
	 * The point of a query effect: `group 1` typed into the command line leaves the
	 * same live selection that recalling the group in the panel does, so a fixture
	 * patched a moment later joins it.
	 */
	it('takes a question, and the question keeps up with the rig', () => {
		applySelectionEffect({
			selection: {
				query: {
					clauses: [{ combine: 'Add', term: { kind: 'OfType', typeId: 'par' } }],
					order: { kind: 'ByName' }
				}
			}
		});
		const before = get(selection);
		rig.update((f) => [...f, fixture('e', 4)]);
		expect(get(selection).length).toBe(before.length + 1);
	});

	it('prefers the question when handed both', () => {
		applySelectionEffect({
			selection: {
				query: idsQuery(['a']),
				fixtureIds: ['a', 'b', 'c']
			}
		});
		expect(get(selection)).toEqual(['a']);
	});

	/**
	 * A recalled group carries its own order, and this browser's last drag must not
	 * quietly overrule it.
	 */
	it('drops the hand order, so a recalled group keeps the order it was saved in', () => {
		addClause('Add', { kind: 'Everything' });
		reorder(3, 0);
		expect(get(selection)[0]).toBe('d');

		applySelectionEffect({
			selection: {
				query: {
					clauses: [{ combine: 'Add', term: { kind: 'Everything' } }],
					order: { kind: 'Manual', order: ['c', 'b'] }
				}
			}
		});
		expect(get(selection)).toEqual(['c', 'b', 'a', 'd']);
	});

	it('says when it was asked for nothing', () => {
		expect(applySelectionEffect(null)).toBe(false);
		expect(applySelectionEffect({})).toBe(false);
	});
});

/**
 * What a group carries when it leaves this browser.
 *
 * The hand order is a store, and a station resolving the group has no store — so a
 * group saved in a dragged order has to carry that order in its query, or it comes
 * back in patch order everywhere but here.
 */
describe('saving the selection as a group', () => {
	beforeEach(() => {
		clearSelection();
	});

	it('bakes the dragged order into the query', () => {
		addClause('Add', { kind: 'Everything' });
		reorder(3, 0);
		expect(get(selection)).toEqual(['d', 'a', 'b', 'c']);

		const saved = asSavedQuery();
		expect(saved.order).toEqual({ kind: 'Manual', order: ['d', 'a', 'b', 'c'] });

		// And it resolves that way with nothing but the query — which is what a
		// station, and every other console, will have.
		expect(evaluate(saved, get(rig))).toEqual(['d', 'a', 'b', 'c']);
	});

	it('leaves a geometric order alone, which already means the same everywhere', () => {
		addClause('Add', { kind: 'Everything' });
		setOrder({ kind: 'ByName' });
		expect(asSavedQuery().order).toEqual({ kind: 'ByName' });
	});

	/** A group saved and then recalled is the same selection, not a rearranged one. */
	it('round-trips through recall', () => {
		addClause('Add', { kind: 'Everything' });
		reorder(3, 0);
		const saved = asSavedQuery();

		clearSelection();
		recall(saved);
		expect(get(selection)).toEqual(['d', 'a', 'b', 'c']);
	});
});
