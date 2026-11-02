/**
 * Fuzzy matching, for the panel picker and the command palette.
 *
 * A subsequence match, scored so that what a person meant comes first: the letters
 * of "tl" are in "Timeline" and in "Stations log", and the one where they start
 * words wins. One implementation for both places that type-to-find, so the picker and
 * ⌘K never disagree about whether "rig" finds the 3D Rig.
 */

/**
 * How well `query` matches `text`, or `null` if it does not. Higher is better.
 *
 * Every query character must appear in order. Points for a character that starts a
 * word, for one that follows the previous match directly, and for the whole query
 * being a prefix; a little off for every character skipped, so a short match beats
 * a scattered one.
 */
export function fuzzyScore(query: string, text: string): number | null {
	const q = query.trim().toLowerCase();
	if (q === '') return 0;
	const t = text.toLowerCase();
	if (t.startsWith(q)) return 1000 - t.length;
	const at = t.indexOf(q);
	if (at >= 0) return 800 - at - t.length / 10;

	let score = 0;
	let from = 0;
	let previous = -2;
	for (const char of q) {
		if (char === ' ') continue;
		const found = t.indexOf(char, from);
		if (found < 0) return null;
		const startsWord = found === 0 || /[\s\-_/.:(]/.test(t[found - 1]);
		score += startsWord ? 10 : 1;
		if (found === previous + 1) score += 5;
		score -= (found - from) * 0.5;
		previous = found;
		from = found + 1;
	}
	return score;
}

/** Keep what matches, best first; ties keep their original order. */
export function fuzzyFilter<T>(query: string, items: T[], text: (item: T) => string): T[] {
	if (query.trim() === '') return items;
	return items
		.map((item, index) => ({ item, index, score: fuzzyScore(query, text(item)) }))
		.filter((x): x is { item: T; index: number; score: number } => x.score !== null)
		.sort((a, b) => b.score - a.score || a.index - b.index)
		.map((x) => x.item);
}
