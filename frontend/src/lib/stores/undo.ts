/**
 * Taking a change back.
 *
 * There is no stack here. The backend works out what to undo from the oplog, which
 * is what makes undo per *person* rather than per browser: an operator's desktop and
 * the tablet in their hand are the same user, and either can take back what the
 * other did. A stack in one browser could not do that.
 *
 * So this is thin on purpose — a request, a toast, and a nudge to anything showing
 * the history.
 */

import { writable } from 'svelte/store';

import type { HistoryEntry } from '$lib/generated/index.js';
import { addToast } from '$lib/toasts.js';
import { showClient } from './show.js';

/**
 * Bumped whenever the history changes, so a panel showing it knows to re-read.
 *
 * The oplog is not a replicated collection with a subscription of its own — it is
 * infrastructure the frontend asks about — so there is nothing to watch. A counter
 * is cruder than a subscription and honest about being one.
 */
export const historyVersion = writable(0);

let busy = false;

/** Take back this user's last change. */
export const undo = () => take(false);

/** Put back this user's last undo. */
export const redo = () => take(true);

async function take(redoing: boolean): Promise<void> {
	// No check that anybody is signed in: there is always somebody. A show is given
	// a default user when it is loaded, so the first change on a fresh console is
	// attributed and can be taken back.
	//
	// A held Ctrl-Z repeats faster than the round trip, and two undos in flight
	// would both read the log before either had written to it and take back the same
	// change twice.
	if (busy) return;
	busy = true;
	try {
		const undone = await showClient().undo(redoing);
		if (!undone) {
			addToast(redoing ? 'Nothing to put back.' : 'Nothing to take back.');
			return;
		}
		// Said out loud only when a press moved more than the operator was looking
		// at. Taking back one rename is its own confirmation — the name changes on
		// screen — but taking back a fan across twenty heads is worth a sentence.
		if (undone.changed > 1) {
			addToast(`${redoing ? 'Put back' : 'Took back'} ${undone.changed} changes.`);
		}
		historyVersion.update((n) => n + 1);
	} catch {
		addToast(redoing ? 'That would not redo.' : 'That would not undo.');
	} finally {
		busy = false;
	}
}

/** The recent history, newest first. */
export async function readHistory(limit = 100): Promise<HistoryEntry[]> {
	try {
		return await showClient().history(limit);
	} catch {
		return [];
	}
}

/**
 * Whether a keystroke means undo, redo, or neither.
 *
 * Pure so it can be tested without a keyboard. Ctrl/Cmd-Z undoes and either
 * Ctrl-Shift-Z or Ctrl-Y redoes, which covers what people's hands already do on
 * both platforms.
 *
 * A keystroke inside a text field is not a shortcut: somebody mid-way through
 * typing a cue name means the browser's own undo, and taking their fixture back
 * instead would be a nasty surprise.
 */
export function shortcutFor(
	event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'shiftKey'>,
	inTextField: boolean
): 'undo' | 'redo' | null {
	if (inTextField) return null;
	if (!event.ctrlKey && !event.metaKey) return null;
	const key = event.key.toLowerCase();
	if (key === 'y') return 'redo';
	if (key !== 'z') return null;
	return event.shiftKey ? 'redo' : 'undo';
}

/** Whether the event landed somewhere a person is typing. */
export function isTextField(target: EventTarget | null): boolean {
	const el = target as HTMLElement | null;
	if (!el) return false;
	const tag = el.tagName;
	return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable === true;
}
