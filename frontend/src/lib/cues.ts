/**
 * Making a cue.
 *
 * Creating a cue and appending it to a sequence is two writes that have to happen
 * together, and there are now two places that do it — the cue list and the store
 * menu. Written once here so the two cannot disagree about what a new cue's number
 * is, or leave a cue behind that no sequence points at.
 */

import type { Cue, Sequence } from './generated/index.js';
import { parameterKey } from './patch.js';
import type { DataRoot } from './ws/data.js';

export type NewCue = {
	name: string;
	captures?: Cue['captures'];
	fadeInMs?: number;
	fadeOutMs?: number;
	/** The shape of its fades. Left out means the show's default per parameter. */
	easing?: Cue['easing'];
	followMode?: Cue['follow_mode'];
	/** Where in the list it goes. Appended when this is not given. */
	number?: number;
	/** The cue it goes after, for an insert. */
	after?: string;
};

/**
 * Add a cue to a sequence, and hand back what was made.
 *
 * Appended by default. Given an `after`, it is inserted directly behind that cue with
 * a number between it and the next — which is what fractional cue numbers have always
 * been for, and what nothing used until now.
 */
export async function createCue(
	data: DataRoot,
	sequence: Sequence,
	cues: Cue[],
	{
		name,
		captures = [],
		fadeInMs = DEFAULT_FADE_MS,
		fadeOutMs = DEFAULT_FADE_MS,
		easing = null,
		followMode = 'Manual',
		number,
		after
	}: NewCue
): Promise<Cue> {
	const order = orderedCues(sequence, cues);
	const at = after ? order.findIndex((c) => c.id === after) : -1;

	const cue: Cue = {
		id: crypto.randomUUID(),
		name,
		number: number ?? (at >= 0 ? insertNumber(order[at].number, order[at + 1]?.number) : order.length + 1),
		captures,
		follow_mode: followMode,
		fade_in_ms: fadeInMs,
		fade_out_ms: fadeOutMs,
		easing,
		is_active: false
	};
	await data.cues.create(cue);

	const ids = [...sequence.cue_ids];
	ids.splice(at >= 0 ? at + 1 : ids.length, 0, cue.id);
	await data.sequences.byId(sequence.id).cue_ids.set(ids);
	return cue;
}

/**
 * Half a second, which is the fade an unfamiliar console should give you.
 *
 * Long enough that a Go is not a jump, short enough that it does not feel like the
 * desk hesitating. Every cue made without being asked gets this, and every one of
 * them can be changed afterwards.
 */
export const DEFAULT_FADE_MS = 500;

/** A sequence's cues, in the order the sequence lists them. */
export function orderedCues(sequence: Sequence, cues: Cue[]): Cue[] {
	const byId = new Map(cues.map((c) => [c.id, c]));
	return sequence.cue_ids.map((id) => byId.get(id)).filter((c): c is Cue => !!c);
}

/**
 * A number between two cues, or after the last one.
 *
 * The midpoint, so 1 and 2 gives 1.5 and 1.5 and 2 gives 1.75 — which is how a cue
 * list survives being inserted into repeatedly without renumbering everything below
 * it, and why `Cue.number` was a float from the start.
 *
 * With nothing after it, the next whole number: appending to a list that ends at 4.75
 * should give 5, not 5.75.
 */
export function insertNumber(before: number, after?: number): number {
	if (after === undefined) return Math.floor(before) + 1;
	return before + (after - before) / 2;
}

/**
 * Move one cue to a different place in the list.
 *
 * The ids are the order — `cue_ids` is `ordered` in the schema — so this rewrites
 * that and leaves the numbers alone. Renumbering on every drag would make a cue an
 * operator calls "cue 5" stop being cue 5 because somebody moved cue 2.
 */
export function reorderCueIds(ids: string[], from: number, to: number): string[] {
	if (from === to || from < 0 || from >= ids.length) return ids;
	const next = [...ids];
	const [moved] = next.splice(from, 1);
	next.splice(Math.max(0, Math.min(next.length, to)), 0, moved);
	return next;
}

/**
 * The latest capture of every key over a run of cues.
 *
 * The browser's copy of `pult_schema::types::cue::tracked_through`, and it is here
 * for the reason `selection.ts` and `scene.ts` are: a cue clicked in a list has to
 * colour a sheet in the same frame, and a round trip inside that is a sheet that
 * flickers. The two are held together by `testdata/tracking.json`, which this file's
 * test and `crates/pult-schema/tests/tracking_corpus.rs` both read.
 *
 * `through` is the cue ids in order, up to and including the one being looked at. A
 * cue the show no longer has is **skipped rather than refused**: a sequence naming a
 * deleted cue is a show mid-edit, not a reason to answer nothing.
 *
 * The order is part of the answer — keys in the order they were first captured,
 * whichever cue later overrode them — because a sheet whose rows moved between two
 * identical questions is a sheet nobody can read.
 */
export function trackedThrough(
	through: string[],
	byId: (id: string) => Cue | undefined
): { cue: Cue; capture: Cue['captures'][number] }[] {
	const order: string[] = [];
	const latest = new Map<string, { cue: Cue; capture: Cue['captures'][number] }>();
	for (const id of through) {
		const cue = byId(id);
		if (!cue) continue;
		for (const capture of cue.captures) {
			const key = `${capture.fixture_id}/${parameterKey(capture.parameter_kind)}`;
			if (!latest.has(key)) order.push(key);
			latest.set(key, { cue, capture });
		}
	}
	return order.map((key) => latest.get(key)!);
}

/** The cue ids a sequence lists up to and including one of them, for `trackedThrough`. */
export function cueIdsThrough(sequence: Sequence, cueId: string): string[] {
	const at = sequence.cue_ids.indexOf(cueId);
	return at < 0 ? [] : sequence.cue_ids.slice(0, at + 1);
}

/**
 * What the *next* cue has to say so that a store into this one does not track forward.
 *
 * Storing into cue 3 in a tracking playback changes cue 3 and, silently, every cue
 * after it that does not capture the same key — which is right when a designer is
 * changing the look and wrong when they are fixing one moment. Every other desk calls
 * the second one *cue only*, and this is it: for each key the store changes, if the
 * next cue does not capture that key itself, the value it was **tracking** before the
 * store is written into it, so the change stops at this cue's edge.
 *
 * Three things fall out of the rule and are worth stating.
 *
 * The compensating capture carries the value and nothing else — no fade time, no
 * delay, no curve. Those are copied from nowhere on purpose: the value is being put
 * into a different cue, and it should move the way *that* cue moves.
 *
 * A key nothing was tracking before needs no compensation. There was nothing to
 * preserve, so the next cue goes on saying nothing about it and whatever is beneath
 * shows through, exactly as it did.
 *
 * And the last cue of a sequence has no next, so cue only does nothing there — which
 * is not a special case but the same rule with nothing to write into.
 */
export function cueOnlyCompensation(
	next: Cue,
	changed: string[],
	before: Map<string, Cue['captures'][number]>
): Cue['captures'] | null {
	const already = new Set(
		next.captures.map((c) => `${c.fixture_id}/${parameterKey(c.parameter_kind)}`)
	);
	const added: Cue['captures'] = [];
	for (const key of changed) {
		if (already.has(key)) continue;
		const tracked = before.get(key);
		if (!tracked) continue;
		added.push({
			fixture_id: tracked.fixture_id,
			parameter_kind: tracked.parameter_kind,
			value: tracked.value,
			// A stored effect drops its anchor, the same rule `storeCaptures` follows:
			// the cue's own `went_at` is what it is measured from on every Go.
			effect: tracked.effect ? { ...tracked.effect, t0: null } : null,
			// The reference comes with it: a compensating value that dropped the palette
			// would be the one cue in the show whose "warm" stopped following warm.
			preset: tracked.preset ?? null,
			fade_in_ms: 0,
			fade_out_ms: 0,
			delay_in_ms: 0,
			easing: null
		});
		already.add(key);
	}
	return added.length === 0 ? null : [...next.captures, ...added];
}
