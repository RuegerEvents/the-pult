/**
 * What the cue sheet and the fixture sheet are pointing at.
 *
 * Three small pieces of this browser's own state, and none of it is the show's: which
 * cue is being *looked at* (as against taken), which preset is being edited, and which
 * cue sheet the Space bar means.
 *
 * Looking is deliberately not taking. Clicking a cue in a list to see what is in it is
 * the commonest thing anybody does with a cue list, and on a console where a click
 * took the cue it would be the commonest way to put the wrong look on stage. So a
 * click sets this, and a double-click or the Go column takes.
 */

import { get, writable } from 'svelte/store';

/** The cue whose contents the fixture sheet is showing, or `null` for the rig. */
export const cueInView = writable<string | null>(null);

/** The preset whose values the fixture sheet is showing. Set in package E. */
export const presetInView = writable<string | null>(null);

/**
 * The sequence whose cue sheet was touched last, which is what Space means.
 *
 * Space is Go on *a* cue sheet, and a console with three of them open has to answer
 * which. Last-touched rather than a mode or a selected sequence: it is the answer an
 * operator already has in their head, and the one that needs no control of its own.
 * Nothing focused means no Go and a toast — never a guess, because a Go into the
 * wrong sequence is a look on stage nobody asked for.
 */
export const focusedCueSheet = writable<string | null>(null);

/** Look at a cue, and stop looking at a preset — the sheet shows one thing at a time. */
export function viewCue(cueId: string | null): void {
	presetInView.set(null);
	cueInView.set(cueId === get(cueInView) ? null : cueId);
}

/** And the other way round. */
export function viewPreset(presetId: string | null): void {
	cueInView.set(null);
	presetInView.set(presetId === get(presetInView) ? null : presetId);
}
