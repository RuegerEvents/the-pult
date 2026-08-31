/**
 * Users, as far as anything pure is concerned.
 *
 * The colours are the same list the schema hands out, written here too rather than
 * fetched: a browser needs one the moment somebody types a name, and a round trip to
 * ask what colour they should be would be a round trip to learn a constant.
 */

import type { HistoryEntry, PluginDatum, User } from './generated/index.js';

/** The colours a new user is given, in order. Matches `pult-schema`'s list. */
export const USER_COLOURS = ['#4a9eff', '#f59e0b', '#22c55e', '#e879f9', '#f87171', '#2dd4bf'];

export const colourFor = (index: number): string => USER_COLOURS[index % USER_COLOURS.length];

/** The colour to draw a user's changes in, falling back for one nobody knows. */
export function colourOf(users: User[], id: string | null | undefined): string {
	if (!id) return 'var(--text-faint)';
	return users.find((u) => u.id === id)?.colour ?? 'var(--text-dim)';
}

/**
 * What to call a change in the history.
 *
 * The path is the honest description and reads well enough for most of them —
 * `fixtures → Spot 3 → name` is exactly what happened. Ids are shortened because a
 * full uuid in a list is a wall of hex that hides the two words either side of it.
 */
export function describeChange(entry: HistoryEntry, names: Map<string, string> = new Map()): string {
	// An id arrives as a plain string — `PathSegment` flattens on the wire — so it is
	// told apart by shape rather than by a tag. A uuid in a change list is a wall of
	// hex that hides the two words either side of it, so it becomes a name where the
	// panel knows one.
	const parts = entry.path.map((segment) => {
		if (typeof segment === 'number') return String(segment);
		const text = String(segment);
		if (!isUuid(text)) return tidy(text);
		return names.get(text) ?? short(text);
	});
	return parts.join(' → ');
}

/** `__create` and `__delete` are protocol, not English. */
function tidy(segment: string): string {
	if (segment === '__create') return 'added';
	if (segment === '__delete') return 'removed';
	return segment.replace(/_/g, ' ');
}

const short = (id: string): string => id.slice(0, 6);

/**
 * What to call a plugin's stored value in the history.
 *
 * A store row's id is a hash of what it names rather than something an operator
 * ever sees, so without this a saved macro reads `plugin data → a1b2c3 → value`
 * — the one entry in the list nobody can act on. Only a store that declared its
 * writes undoable reaches the history at all, and by then somebody has asked the
 * plugin to save something and deserves to be told what.
 */
export function pluginDatumName(row: Pick<PluginDatum, 'plugin_id' | 'store' | 'key'>): string {
	return `${row.plugin_id} · ${row.store} · ${row.key}`;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const isUuid = (text: string): boolean => UUID.test(text);

/** "a moment ago", for a list nobody wants timestamps in. */
export function ago(iso: string, now = Date.now()): string {
	const seconds = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
	if (seconds < 10) return 'just now';
	if (seconds < 60) return `${seconds}s ago`;
	if (seconds < 3600) return `${Math.round(seconds / 60)}m ago`;
	if (seconds < 86400) return `${Math.round(seconds / 3600)}h ago`;
	return `${Math.round(seconds / 86400)}d ago`;
}
