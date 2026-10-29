/**
 * Fixture numbers: the one on a fixture's label, which is what `fixture 3` means on
 * the command line and the wing (`Fixture::fixture_number`).
 *
 * The station numbers a fixture patched without one (`next_fixture_number`), so what is
 * left for a page is a show from before that, two fixtures sharing a number, and the
 * order the sheet lists them in.
 */

import type { Fixture } from './generated/index.js';

type Numbered = Pick<Fixture, 'id' | 'fixture_number'>;

/** The numbers more than one fixture carries. Allowed — `fixture 5` then means all of them — and marked. */
export function sharedNumbers(fixtures: Numbered[]): Set<number> {
	const seen = new Set<number>();
	const shared = new Set<number>();
	for (const { fixture_number: n } of fixtures) {
		if (n === null) continue;
		if (seen.has(n)) shared.add(n);
		seen.add(n);
	}
	return shared;
}

/**
 * A number for every fixture without one, in patch order, counting on from the highest
 * in the rig — the rule the station numbers a new fixture by, so what this gives and
 * what patching one more afterwards gives do not collide.
 */
export function numberTheRest(fixtures: Numbered[]): { id: string; number: number }[] {
	let next = Math.max(0, ...fixtures.map((f) => f.fixture_number ?? 0));
	return fixtures.filter((f) => f.fixture_number === null).map((f) => ({ id: f.id, number: ++next }));
}

/** In number order, patch order within a number, and the unnumbered last. */
export function byNumber<F extends Numbered>(fixtures: F[]): F[] {
	const place = new Map(fixtures.map((f, i) => [f.id, i]));
	return [...fixtures].sort(
		(a, b) =>
			(a.fixture_number ?? Infinity) - (b.fixture_number ?? Infinity) || place.get(a.id)! - place.get(b.id)!
	);
}

/** A typed number as a fixture number, `null` for clearing it, `undefined` for nonsense. */
export function parseFixtureNumber(text: string): number | null | undefined {
	const trimmed = text.trim();
	if (trimmed === '') return null;
	const n = Number(trimmed);
	return Number.isInteger(n) && n >= 1 && n <= 0xffffffff ? n : undefined;
}
