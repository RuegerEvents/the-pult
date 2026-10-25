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

import { addToast } from '../toasts.js';
import { showData } from './show.js';

/** The cue whose contents the fixture sheet is showing, or `null` for the rig. */
export const cueInView = writable<string | null>(null);

/** The preset whose values the fixture sheet is showing. Set in package E. */
export const presetInView = writable<string | null>(null);

/**
 * Which sequence the Cues panel is showing. A store rather than the panel's own
 * state, so an executor in Playback can open its sequence there with one click.
 */
export const sequenceInView = writable<string | null>(null);

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

/**
 * Go on the cue sheet touched last — Space, and the wing's GO — or back one cue.
 *
 * With none focused there is **no Go and a toast**, never a guess. A console with
 * three cue sheets open has to answer which one is meant, and answering it by
 * picking one is a look on stage nobody asked for.
 */
export async function goOnFocusedSheet(back = false): Promise<void> {
	const sequenceId = get(focusedCueSheet);
	if (!sequenceId) {
		addToast(`${back ? 'Go back' : 'Go'} is on a cue sheet — click one first`);
		return;
	}
	const sequence = showData().sequences.byId(sequenceId);
	try {
		if (back) await sequence.goBack({ at: Date.now() });
		else await sequence.goNext({ at: Date.now() });
	} catch (e) {
		addToast(`${e}`);
	}
}
