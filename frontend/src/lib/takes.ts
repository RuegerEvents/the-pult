/**
 * Arming a timeline to record, and reading back what is being recorded.
 *
 * A take comes from an input — somebody else's console on a wire — or from the
 * programmer, which is whoever is at this desk. Both are one `record` command on the
 * timeline; this is the one place a picker's choice becomes its arguments and a
 * `TakeStatus` becomes a sentence.
 */

import type { KeyOutline } from './evaluator.js';
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

// ── A take is a row per parameter ─────────────────────────────────────────────
//
// A stored recording never changes; what is edited is the rows pointing into it, one
// per parameter, each with a window (`TimelineTrack::from_ms`/`to_ms`). Cropping is a
// window's end moved, slicing is one row becoming two with the same recording, and
// deleting a part is deleting a slice. What plays is the rows composed by
// `pult_render::compose`, on the station and in the page alike.

/** The row a parameter is drawn on: one per fixture and key. */
export const parameterRow = (fixtureId: string, key: string) => `${fixtureId}/${key}`;

/** What of a recording a row plays: its own parameter, or every one for an old whole take. */
export function rowKeys(track: TimelineTrack, outline: KeyOutline[]): KeyOutline[] {
	if (track.key === null) return outline;
	return outline.filter((k) => k.key === track.key && k.fixture_id === track.fixture_id);
}

/**
 * Where a row is on the timeline: its window, and where it has none, where its
 * recording's points are, nudged by its offset. `null` with no window and no outline.
 */
export function rowSpan(track: TimelineTrack, keys: KeyOutline[] | null): { fromMs: number; toMs: number } | null {
	const natural = keys?.length
		? {
				fromMs: Math.min(...keys.map((k) => k.from_ms)) + track.offset_ms,
				toMs: Math.max(...keys.map((k) => k.to_ms)) + track.offset_ms
			}
		: null;
	const fromMs = track.from_ms ?? natural?.fromMs;
	const toMs = track.to_ms ?? natural?.toMs;
	if (fromMs === undefined || toMs === undefined) return null;
	return { fromMs: Math.max(0, fromMs), toMs: Math.max(fromMs, toMs) };
}

/**
 * The rows with each of `ids` the position falls inside cut in two there: the first
 * half keeps the row, the second is a row of its own on the same recording. `null`
 * where the position is inside none of them, so there is nothing to write.
 */
export function splitRows(
	tracks: TimelineTrack[],
	ids: Set<string>,
	atMs: number,
	spanOf: (track: TimelineTrack) => { fromMs: number; toMs: number } | null,
	newId: () => string
): TimelineTrack[] | null {
	const at = Math.round(atMs);
	let split = false;
	const out = tracks.flatMap((track) => {
		const span = ids.has(track.id) ? spanOf(track) : null;
		if (!span || at <= span.fromMs || at >= span.toMs) return [track];
		split = true;
		return [
			{ ...track, from_ms: track.from_ms ?? span.fromMs, to_ms: at },
			{ ...track, id: newId(), from_ms: at, to_ms: track.to_ms ?? span.toMs }
		];
	});
	return split ? out : null;
}

/** A row with one end of its window moved to `atMs`, never past its other end. */
export function cropped(
	track: TimelineTrack,
	grip: 'front' | 'back',
	atMs: number,
	span: { fromMs: number; toMs: number }
): TimelineTrack {
	const at = Math.round(atMs);
	return grip === 'front'
		? { ...track, from_ms: Math.max(0, Math.min(at, span.toMs - 1)), to_ms: track.to_ms ?? span.toMs }
		: { ...track, from_ms: track.from_ms ?? span.fromMs, to_ms: Math.max(span.fromMs + 1, at) };
}

/**
 * A row moved: its recording and its window together, so a slice dragged along the
 * timeline takes what it plays with it. `offsetMs` is where its offset lands.
 */
export function slid(track: TimelineTrack, offsetMs: number): TimelineTrack {
	const by = offsetMs - track.offset_ms;
	return {
		...track,
		offset_ms: offsetMs,
		from_ms: track.from_ms === null ? null : Math.max(0, track.from_ms + by),
		to_ms: track.to_ms === null ? null : Math.max(0, track.to_ms + by)
	};
}
