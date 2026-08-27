/**
 * Which fixtures are selected.
 *
 * Selection is the operator's, not the show's: `CLAUDE.md` puts frontend-only UI
 * state in a store rather than in the schema, and two people at two consoles are
 * plainly allowed to have hold of different fixtures.
 *
 * The spec eventually wants a selection generated from geometric functions over the
 * rig rather than kept as a list of ids. This is the list of ids that comes first —
 * the plan view is the first place in the console where picking a fixture out of the
 * rig by looking at it is even possible.
 */

import { writable, derived, get } from 'svelte/store';

export const selection = writable<string[]>([]);

/** Membership, for a component that only needs to ask about one fixture. */
export const selected = derived(selection, ($selection) => new Set($selection));

export function select(id: string) {
	selection.set([id]);
}

/** Add or remove one, for a shift-click. Order is kept: it is the fixture order. */
export function toggle(id: string) {
	selection.update((ids) => (ids.includes(id) ? ids.filter((i) => i !== id) : [...ids, id]));
}

export const clearSelection = () => selection.set([]);

/** Drop one fixture, for the × beside it in the selection panel. */
export function remove(id: string) {
	selection.update((ids) => (ids.includes(id) ? ids.filter((i) => i !== id) : ids));
}

/**
 * Move one fixture to another place in the order.
 *
 * The spec asks for the selection to be reorderable by drag and drop, and the order
 * is not decoration: it is what an effect spreads along, so "the third fixture" has
 * to be something the operator decides rather than something the patch decided.
 *
 * Out-of-range indices leave the selection alone rather than throwing — a drop that
 * lands outside the list is a cancelled drag, not an error.
 */
export function reorder(from: number, to: number) {
	selection.update((ids) => {
		if (from === to || from < 0 || to < 0 || from >= ids.length || to >= ids.length) return ids;
		const next = [...ids];
		const [moved] = next.splice(from, 1);
		next.splice(to, 0, moved);
		return next;
	});
}

export const isSelected = (id: string) => get(selection).includes(id);

/** Drop anything that is no longer in the rig, after a fixture is deleted. */
export function pruneSelection(present: Iterable<string>) {
	const alive = new Set(present);
	selection.update((ids) => {
		const kept = ids.filter((id) => alive.has(id));
		return kept.length === ids.length ? ids : kept;
	});
}
