/**
 * Making a cue.
 *
 * Creating a cue and appending it to a sequence is two writes that have to happen
 * together, and there are now two places that do it — the cue list and the store
 * menu. Written once here so the two cannot disagree about what a new cue's number
 * is, or leave a cue behind that no sequence points at.
 */

import type { Cue, Sequence } from './generated/index.js';
import type { DataRoot } from './ws/data.js';

export type NewCue = {
	name: string;
	captures?: Cue['captures'];
	fadeInMs?: number;
	fadeOutMs?: number;
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
