/**
 * A timeline event in words, and which sequence it is about.
 *
 * One place, because the lane's tooltip, the Inspector's heading, the palette's entry
 * and the event row's name all say it, and four spellings of "Area 2 → Scare" would be
 * four answers to one question.
 */

import type { Cue, Sequence, TimelineAction, VariableOp } from '$lib/generated/index.js';

type Named = { id: string; name: string };

/** The sequence an action Goes, whichever kind of Go it is; `null` for an act that is
 *  not on a sequence (playing a timeline, changing a variable). */
export const sequenceOf = (action: TimelineAction): string | null =>
	'GoToCue' in action
		? action.GoToCue.sequence_id
		: 'GoNext' in action
			? action.GoNext.sequence_id
			: 'Off' in action
				? action.Off.sequence_id
				: null;

/** The timeline an action plays or stops, if that is what it does. */
export const timelineOf = (action: TimelineAction): string | null =>
	'PlayTimeline' in action
		? action.PlayTimeline.timeline_id
		: 'StopTimeline' in action
			? action.StopTimeline.timeline_id
			: null;

const opWords = (op: VariableOp): string => {
	if (op === 'Toggle') return 'toggle';
	if (op === 'Reset') return 'reset';
	if ('Set' in op) return `= ${op.Set}`;
	return op.Add >= 0 ? `+ ${op.Add}` : `− ${-op.Add}`;
};

/** "Area 2 → Scare", "Area 2 → next", "Area 2 off", "▶ Ambience", "■ Ambience", "Inside + 1". */
export function describeAction(
	action: TimelineAction,
	sequences: Sequence[],
	cues: Cue[],
	timelines: Named[] = [],
	variables: Named[] = []
): string {
	const timeline = timelineOf(action);
	if (timeline !== null) {
		const name = timelines.find((t) => t.id === timeline)?.name ?? 'a timeline';
		return 'PlayTimeline' in action ? `▶ ${name}` : `■ ${name}`;
	}
	if ('SetVariable' in action) {
		const name = variables.find((v) => v.id === action.SetVariable.variable_id)?.name ?? 'a variable';
		return `${name} ${opWords(action.SetVariable.op)}`;
	}
	const sequence = sequences.find((s) => s.id === sequenceOf(action))?.name ?? 'a gone sequence';
	if ('GoToCue' in action) {
		const cue = cues.find((c) => c.id === action.GoToCue.cue_id)?.name ?? 'a gone cue';
		return `${sequence} → ${cue}`;
	}
	if ('GoNext' in action) return `${sequence} → next`;
	return `${sequence} off`;
}

/** The cues of a sequence, in its own order. */
export function cuesOf(sequenceId: string | null, sequences: Sequence[], cues: Cue[]): Cue[] {
	const sequence = sequences.find((s) => s.id === sequenceId);
	if (!sequence) return [];
	return sequence.cue_ids.map((id) => cues.find((c) => c.id === id)).filter((c): c is Cue => !!c);
}
