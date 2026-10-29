/**
 * What a picker of show objects lists for what has been typed into it.
 *
 * A picker is `Pick.svelte`: a sequence, a cue, a timeline, anything with a name,
 * chosen by typing a bit of it rather than scrolling a native `<select>` of eighty.
 * Filed objects come grouped by folder, the folder's path as the heading, the unfiled
 * ones first — `byFolder`'s rule, which the native selects already followed. The
 * search reads the folder path too, so "act 2 scare" finds the scares filed in Act 2.
 */

import { byFolder } from './folders.js';
import type { Folder } from './generated/index.js';

export type Pickable = { id: string; name?: string | null; folder?: string | null };

export type PickGroup<Row> = { path: string; rows: Row[] };

/** Lower case, accents off: "Außen" is found by "aussen" and by "außen". */
export function fold(text: string): string {
	return text
		.normalize('NFD')
		.replace(/[̀-ͯ]/g, '')
		.replace(/ß/g, 'ss')
		.toLowerCase();
}

/** Whether every word typed is somewhere in what the row is called or where it is filed. */
export function matches(query: string, label: string, path: string): boolean {
	const terms = fold(query).split(/\s+/).filter(Boolean);
	if (terms.length === 0) return true;
	const haystack = fold(`${label} ${path}`);
	return terms.every((term) => haystack.includes(term));
}

/** The groups a picker draws for `query`: folders by path, a group gone when nothing in it matches. */
export function pickGroups<Row extends Pickable>(
	rows: Row[],
	folders: Folder[],
	query: string,
	label: (row: Row) => string
): PickGroup<Row>[] {
	return byFolder(rows, folders)
		.map((group) => ({ path: group.path, rows: group.rows.filter((row) => matches(query, label(row), group.path)) }))
		.filter((group) => group.rows.length > 0);
}

/**
 * The same, grouped by something other than a folder: the cues of a show by the
 * sequence each is in. Groups keep the order their first row came in.
 */
export function pickGroupsBy<Row>(
	rows: Row[],
	query: string,
	label: (row: Row) => string,
	groupOf: (row: Row) => string
): PickGroup<Row>[] {
	const groups = new Map<string, Row[]>();
	for (const row of rows) {
		const path = groupOf(row);
		if (!matches(query, label(row), path)) continue;
		groups.set(path, [...(groups.get(path) ?? []), row]);
	}
	return [...groups.entries()].map(([path, rows]) => ({ path, rows }));
}
