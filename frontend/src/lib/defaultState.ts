/**
 * What a show's default state says, in words: which sequences come up on and at which
 * cue, which masters are not at full, and which timelines run.
 *
 * The station keeps ids (`Show::default_state`, see `infra/default_state.rs`); a menu
 * that said "2 sequences, 1 timeline" left an operator guessing which ones would start
 * when the show came up. Pure, so the wording is tested without a show.
 */

import type { Cue, DefaultState, Sequence, Timeline } from '$lib/generated/index.js';

/** One line per thing the default state does, sequences first. */
export function describeDefaultState(
	state: DefaultState,
	sequences: Sequence[],
	cues: Cue[],
	timelines: Timeline[]
): string[] {
	const lines: string[] = [];
	for (const entry of state.sequences) {
		const sequence = sequences.find((s) => s.id === entry.sequence_id);
		const name = sequence?.name || 'a sequence that is gone';
		const parts: string[] = [];
		if (entry.cue_id) {
			const cue = cues.find((c) => c.id === entry.cue_id);
			parts.push(cue ? `at cue ${cueLabel(cue)}` : 'at a cue that is gone');
		} else {
			parts.push('off');
		}
		if (entry.master !== null && entry.master !== undefined && entry.master < 1) {
			parts.push(`master ${Math.round(entry.master * 100)}%`);
		}
		lines.push(`${name}: ${parts.join(', ')}`);
	}
	for (const id of state.timelines) {
		const timeline = timelines.find((t) => t.id === id);
		lines.push(`${timeline?.name || 'a timeline that is gone'}: plays from the start`);
	}
	return lines;
}

/** A cue as an operator reads it: its number, and its name where it has one. */
function cueLabel(cue: Cue): string {
	return cue.name ? `${cue.number} “${cue.name}”` : String(cue.number);
}
