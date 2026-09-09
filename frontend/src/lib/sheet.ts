/**
 * What is driving one cell of the fixture sheet, and what colour that makes it.
 *
 * The sheet's whole claim is that a colour tells you *where a value came from*, and
 * this console can make that claim honestly where other desks make it with a flag:
 * nothing here is stored or guessed, it is read off the same four layers the
 * evaluator is handed. `driving.ts` assembles them; this decides which one is on top.
 *
 * Pure, and tested, because the alternative is a table of colours built inside a
 * component where nobody can ask it a question.
 */

import type { Cue, ParameterCapture } from './generated/index.js';
import type { DrivenBy } from './driving.js';

/**
 * The layer a value came from, in the order they beat each other.
 *
 * `programmer > track > effect > fade > home` is the stack, and the two fade cases
 * are split because the sheet's job in cue mode is to say which cue a value belongs
 * to: `cue` is the cue being looked at asserting it, `tracked` is an earlier one
 * still holding it.
 */
export type Source = 'programmer' | 'track' | 'effect' | 'cue' | 'tracked' | 'home' | 'none';

/** The `cue_id` a fade carries when no cue put it there — a release, or a send home. */
export const NO_CUE = '00000000-0000-0000-0000-000000000000';

/**
 * Which layer is on top for one parameter.
 *
 * `track` is passed in rather than read off `DrivenBy`, because a recording is not one
 * of the four things a fixture row carries: it is an asset the evaluator holds, and
 * the only thing a page can say about it is whether a take is rolling over this key.
 *
 * `shownCue` is the cue the sheet is looking at, or `null` when it is looking at the
 * rig. With one, a fade from that cue reads as *hard* and a fade from any other cue
 * reads as *tracked* — which is the distinction an operator is asking about when they
 * click a cue. Without one, every cue's fade is simply a cue's.
 *
 * A fade with no cue behind it is a release on its way home, and reads as `home`: it
 * is going exactly where `home` is, and colouring it as a cue's would name a cue that
 * is no longer asserting anything.
 */
export function source(
	driven: DrivenBy | undefined,
	track: boolean,
	shownCue: string | null
): Source {
	if (!driven) return track ? 'track' : 'none';
	if (driven.programmer !== undefined) return 'programmer';
	if (track) return 'track';
	if (driven.effect) return 'effect';
	if (driven.fade) {
		if (driven.fade.cue_id === NO_CUE) return 'home';
		if (shownCue === null) return 'cue';
		return driven.fade.cue_id === shownCue ? 'cue' : 'tracked';
	}
	if (driven.home !== undefined) return 'home';
	return 'none';
}

/**
 * And the same question asked of a cue rather than of the rig: a capture that
 * survived tracking is *hard* in the cue being looked at, or *tracked* from an
 * earlier one.
 */
export const trackedSource = (from: Cue, shownCue: string): Source =>
	from.id === shownCue ? 'cue' : 'tracked';

/**
 * What each source is called where a legend has to say so.
 *
 * MA-near on purpose: an operator who has stood behind a grandMA reads amber as the
 * programmer and cyan as tracked without being told, and inventing a private palette
 * to be different would cost exactly that.
 */
export const SOURCE_LABELS: Record<Source, string> = {
	programmer: 'Programmer',
	track: 'Recording',
	effect: 'Effect',
	cue: 'This cue',
	tracked: 'Tracked',
	home: 'Home',
	none: 'Nothing'
};

/** The rows a cue mode shows, keyed the way a sheet cell asks for them. */
export function cellsOfCue(
	tracked: { cue: Cue; capture: ParameterCapture }[],
	key: (capture: ParameterCapture) => string
): Map<string, { cue: Cue; capture: ParameterCapture }> {
	const out = new Map<string, { cue: Cue; capture: ParameterCapture }>();
	for (const entry of tracked) out.set(`${entry.capture.fixture_id}/${key(entry.capture)}`, entry);
	return out;
}
