/**
 * What the panel picker offers, worked out without a component.
 *
 * Every panel this console can draw, in its category's column, including the ones
 * already open — listed as "go to", because a picker that hides an open panel is a
 * picker that cannot find it. Setup errands are a column of their own and open Setup
 * at that section rather than taking a tile.
 */

import { fuzzyFilter } from '$lib/fuzzy.js';
import { CATEGORY_ORDER, type Category } from './categories.js';
import { panelHome, type PanelMeta } from './panels.js';

export type PickerEntry = {
	id: string;
	title: string;
	category: Category;
	/** What choosing it does. */
	action: 'open' | 'goto' | 'setup';
};

export type PickerColumn = { category: Category; entries: PickerEntry[] };

/** The entry for one panel, given what is already on screen. */
export function pickerEntry(id: string, meta: PanelMeta, open: Set<string>): PickerEntry {
	const home = panelHome(meta);
	// A panel that is both an errand and a work surface is offered as a work surface;
	// Setup is one click away from the top bar for the errand.
	const action: PickerEntry['action'] = open.has(id)
		? 'goto'
		: home === 'setup'
			? 'setup'
			: 'open';
	return { id, title: meta.title, category: meta.category, action };
}

/** The columns, in category order, with empty ones left out. */
export function pickerColumns(
	panels: Record<string, PanelMeta>,
	open: Set<string>,
	query = ''
): PickerColumn[] {
	const all = Object.entries(panels).map(([id, meta]) => pickerEntry(id, meta, open));
	const matched = fuzzyFilter(query, all, (e) => e.title);
	return CATEGORY_ORDER.map((category) => ({
		category,
		entries: matched.filter((e) => e.category === category)
	})).filter((column) => column.entries.length > 0);
}

/** The entries in reading order (column by column), for arrow keys and Enter. */
export const flatEntries = (columns: PickerColumn[]): PickerEntry[] =>
	columns.flatMap((c) => c.entries);
