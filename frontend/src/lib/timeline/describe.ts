/**
 * A timeline event in words, and which sequence it is about.
 *
 * One place, because the lane's tooltip, the Inspector's heading, the palette's entry
 * and the event row's name all say it, and four spellings of "Area 2 → Scare" would be
 * four answers to one question.
 */

import type { Cue, Sequence, TimelineAction } from '$lib/generated/index.js';

/** The sequence an action Goes, whichever kind of Go it is. */
export const sequenceOf = (action: TimelineAction): string =>
	'GoToCue' in action ? action.GoToCue.sequence_id : 'GoNext' in action ? action.GoNext.sequence_id : action.Off.sequence_id;

/** "Area 2 → Scare", "Area 2 → next", "Area 2 off". */
export function describeAction(action: TimelineAction, sequences: Sequence[], cues: Cue[]): string {
	const sequence = sequences.find((s) => s.id === sequenceOf(action))?.name ?? 'a gone sequence';
	if ('GoToCue' in action) {
		const cue = cues.find((c) => c.id === action.GoToCue.cue_id)?.name ?? 'a gone cue';
		return `${sequence} → ${cue}`;
	}
	if ('GoNext' in action) return `${sequence} → next`;
	return `${sequence} off`;
}

/** The cues of a sequence, in its own order. */
export function cuesOf(sequenceId: string, sequences: Sequence[], cues: Cue[]): Cue[] {
	const sequence = sequences.find((s) => s.id === sequenceId);
	if (!sequence) return [];
	return sequence.cue_ids.map((id) => cues.find((c) => c.id === id)).filter((c): c is Cue => !!c);
}
