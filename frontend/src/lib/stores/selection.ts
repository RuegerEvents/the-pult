/**
 * Which fixtures are selected, as a question rather than an answer.
 *
 * Selection is the operator's, not the show's: `CLAUDE.md` puts frontend-only UI
 * state in a store rather than in the schema, and two people at two consoles are
 * plainly allowed to have hold of different fixtures.
 *
 * What changed is underneath. The source of truth is a `SelectionQuery` evaluated
 * against the current rig, so "every mover downstage" stays true when somebody
 * patches a sixth one — which is what the spec asks for and why: a list of ids is a
 * photograph of a rig that has since been rebuilt.
 *
 * Everything reading this still sees `selection` as an ordered list of ids and
 * `selected` as a set, because that is what a panel wants and nothing about the
 * change is any of their business.
 *
 * One thing fell out of it: nothing needs pruning any more. A deleted fixture stops
 * matching, so it leaves the selection on the next evaluation. The old
 * `pruneSelection` existed only because a list could go stale, and a query cannot.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import {
	EMPTY_QUERY,
	evaluate,
	idsQuery,
	isManualList,
	type Order,
	type SelectionQuery,
	type Term
} from '$lib/selection.js';
import { collection } from './show.js';

const fixtures = collection('fixtures');

/** The question. Everything below is derived from this and the rig. */
export const query = writable<SelectionQuery>(EMPTY_QUERY);

/**
 * The order an operator dragged the panel into.
 *
 * Kept beside the query rather than in it: it is an answer about particular
 * fixtures, and the query is a question that outlives them. Only read when the
 * query's order is `Manual`.
 *
 * Written only by a drag or a freeze, which produce a *complete* order. Picking by
 * hand does not touch it: the arrival order already falls out of the clause the
 * clicks build, and a partial hand order would push whatever it did know about to
 * the front — so shift-clicking one more light onto a geometric selection would jump
 * it to the head of the chase.
 */
const handOrder = writable<string[]>([]);

/** The fixtures the query picks out, in the order it asks for. */
export const selection: Readable<string[]> = derived(
	[query, fixtures, handOrder],
	([$query, $fixtures, $handOrder]) => evaluate($query, $fixtures, $handOrder)
);

/** Membership, for a component that only needs to ask about one fixture. */
export const selected = derived(selection, ($selection) => new Set($selection));

/** How many, and what the query says, for the panel's heading. */
export const isQuery = derived(query, ($query) => !isManualList($query));

// ── Picking by hand ───────────────────────────────────────────────────────────

/**
 * Just this one.
 *
 * Replaces the query outright rather than adding to it: a plain click means "forget
 * what I was doing", and an operator who clicks one light and gets it plus a
 * geometric query they had forgotten about would not thank us.
 */
export function select(id: string) {
	handOrder.set([]);
	query.set(idsQuery([id]));
}

/**
 * Add or remove one, for a shift-click.
 *
 * Tacked onto a trailing hand-picked clause rather than rewriting the query, so a
 * geometric selection can be adjusted by hand without losing the geometry. Removing
 * a fixture the geometry picked adds a `Drop`, because there is no clause to take it
 * out of.
 */
export function toggle(id: string) {
	const current = get(query);
	const present = get(selection).includes(id);

	if (present) {
		query.set({ ...current, clauses: [...current.clauses, { combine: 'Drop', term: { kind: 'Ids', ids: [id] } }] });
		return;
	}

	// Extend the trailing manual clause if there is one, rather than growing a chain
	// of single-id clauses as somebody shift-clicks their way across a truss.
	const last = current.clauses[current.clauses.length - 1];
	if (last?.combine === 'Add' && last.term.kind === 'Ids') {
		const clauses = [...current.clauses];
		clauses[clauses.length - 1] = {
			combine: 'Add',
			term: { kind: 'Ids', ids: [...last.term.ids, id] }
		};
		query.set({ ...current, clauses });
	} else {
		query.set({ ...current, clauses: [...current.clauses, { combine: 'Add', term: { kind: 'Ids', ids: [id] } }] });
	}
}

export const clearSelection = () => {
	handOrder.set([]);
	query.set(EMPTY_QUERY);
};

/** Drop one fixture, for the × beside it in the selection panel. */
export function remove(id: string) {
	if (get(selection).includes(id)) toggle(id);
}

export const isSelected = (id: string) => get(selection).includes(id);

// ── Asking a different question ───────────────────────────────────────────────

/** Replace the query outright, for the selection panel's editor. */
export function setQuery(next: SelectionQuery) {
	query.set(next);
}

/** Add a clause to whatever is already selected. */
export function addClause(combine: 'Add' | 'Keep' | 'Drop', term: Term) {
	query.update((q) => ({ ...q, clauses: [...q.clauses, { combine, term }] }));
}

export function removeClause(index: number) {
	query.update((q) => ({ ...q, clauses: q.clauses.filter((_, i) => i !== index) }));
}

export function setOrder(order: Order) {
	query.update((q) => ({ ...q, order }));
}

/**
 * Turn whatever is selected right now into a plain list.
 *
 * The way out of a query that is nearly right: freeze the answer and edit it by
 * hand. Also what the drag-to-reorder needs, because reordering a question is not a
 * thing you can do.
 */
export function freeze() {
	const ids = get(selection);
	handOrder.set(ids);
	query.set(idsQuery(ids));
}

/**
 * Move one fixture to another place in the order.
 *
 * The spec asks for the selection to be reorderable by drag and drop, and the order
 * is not decoration: it is what an effect spreads along, so "the third fixture" has
 * to be something the operator decides rather than something the patch decided.
 *
 * Dragging freezes a geometric query first. A hand-made order is an answer about
 * particular fixtures, and there is nothing coherent to do with one when the
 * question is "whatever is downstage".
 *
 * Out-of-range indices leave the selection alone rather than throwing — a drop that
 * lands outside the list is a cancelled drag, not an error.
 */
export function reorder(from: number, to: number) {
	const ids = get(selection);
	if (from === to || from < 0 || to < 0 || from >= ids.length || to >= ids.length) return;

	const next = [...ids];
	const [moved] = next.splice(from, 1);
	next.splice(to, 0, moved);

	handOrder.set(next);
	query.update((q) => ({ ...q, order: { kind: 'Manual' } }));
}
