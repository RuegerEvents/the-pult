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
};

/**
 * Add a cue to the end of a sequence, and hand back what was made.
 *
 * The number is the position in the list rather than anything clever. Cue numbers
 * are fractional so that a cue can be inserted between two others, but nothing
 * inserts yet, and counting is the honest thing to do until something does.
 */
export async function createCue(
	data: DataRoot,
	sequence: Sequence,
	{ name, captures = [], fadeInMs = 500, fadeOutMs = 500 }: NewCue
): Promise<Cue> {
	const cue: Cue = {
		id: crypto.randomUUID(),
		name,
		number: sequence.cue_ids.length + 1,
		captures,
		follow_mode: 'Manual',
		fade_in_ms: fadeInMs,
		fade_out_ms: fadeOutMs,
		is_active: false
	};
	await data.cues.create(cue);
	await data.sequences.byId(sequence.id).cue_ids.set([...sequence.cue_ids, cue.id]);
	return cue;
}
