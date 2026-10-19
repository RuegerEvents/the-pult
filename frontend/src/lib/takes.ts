/**
 * Arming a timeline to record, and reading back what is being recorded.
 *
 * A take comes from an input — somebody else's console on a wire — or from the
 * programmer, which is whoever is at this desk. Both are one `record` command on the
 * timeline; this is the one place a picker's choice becomes its arguments and a
 * `TakeStatus` becomes a sentence.
 */

import type { InputConfig, RecordSource, TakeStatus, TimelineTrack } from './generated/index.js';

/** The value a source picker holds: `''` for nothing, `'programmer'`, or an input id. */
export type ArmChoice = string;

export const PROGRAMMER = 'programmer';

/** What a picker should show for what the timeline is armed with. */
export function armChoice(recording: RecordSource | null): ArmChoice {
	if (!recording) return '';
	if ('Programmer' in recording) return PROGRAMMER;
	return recording.Input;
}

/** Whether a programmer recording gives the keys back when it is stored. */
export function releases(recording: RecordSource | null): boolean {
	return recording !== null && 'Programmer' in recording ? recording.Programmer.release : true;
}

/** The arguments of `timelines[id].record(...)` for a choice. */
export function armArgs(
	choice: ArmChoice,
	release: boolean
): { inputId?: string | null; programmer?: boolean; release?: boolean } {
	if (choice === PROGRAMMER) return { programmer: true, release };
	return { inputId: choice || null };
}

function plural(n: number, one: string, many: string): string {
	return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** One line about a take in progress, for the timeline panel. */
export function takeSummary(status: TakeStatus): string {
	const parts = [
		`Recording ${plural(status.keys, 'key', 'keys')}`,
		plural(status.points, 'change point', 'change points')
	];
	if (status.passes > 0) parts.push(`${plural(status.passes, 'pass', 'passes')} stored`);
	return parts.join(' · ');
}

/** Where a stored take came from, in words. */
export function trackSource(track: TimelineTrack, inputs: InputConfig[]): string {
	if (track.input_id === null) return 'programmer';
	return inputs.find((input) => input.id === track.input_id)?.name ?? 'an input';
}
