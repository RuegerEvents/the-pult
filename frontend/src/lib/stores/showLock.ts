/**
 * The Show lock: one switch, in the top bar, that takes every edit handle away.
 *
 * There is no edit mode. A name is renamed where it is written, a row is dragged
 * where it sits, a delete is a button on the thing — and a delete is one Ctrl-Z, so
 * the cost of a mis-hit while programming is a keystroke. What that leaves is show
 * night: a tablet on a truss, a phone at the maze entrance, a desk with the house in.
 * The lock is for then. Locked, a console still *acts* — Go, recall, play, a fader,
 * a variable's +1 — and changes nothing about the show's structure: no rename, no
 * reorder, no delete, no drag to assign, and the context menu offers only acts.
 *
 * # Per browser, and why
 *
 * Kept in this browser's `localStorage`, not in the show. A SYNCED lock would let one
 * tablet lock the desk where somebody is putting a cue right mid-show, and let that
 * somebody unlock every tablet in the building by fixing it — both worse than a lock
 * nobody else can see. A lock is about the hands at *this* screen, which is also why
 * it survives a reload: a tablet locked for the night must not come back unlocked
 * after its browser restarts in the cold.
 *
 * Every panel reads `canEdit`, never the lock itself, so "may I show an edit handle"
 * is one answer for the whole page.
 */

import { browser } from '$app/environment';
import { derived, writable, type Readable } from 'svelte/store';

const STORAGE_KEY = 'pult.showLock';

function stored(): boolean {
	if (!browser) return false;
	try {
		return localStorage.getItem(STORAGE_KEY) === 'locked';
	} catch {
		return false;
	}
}

/** Whether this browser is locked for the show. */
export const showLocked = writable<boolean>(stored());

showLocked.subscribe((locked) => {
	if (!browser) return;
	try {
		if (locked) localStorage.setItem(STORAGE_KEY, 'locked');
		else localStorage.removeItem(STORAGE_KEY);
	} catch {
		// A browser with storage off still locks; it just forgets on reload.
	}
});

/** Whether edit handles may be drawn. The one thing a panel reads. */
export const canEdit: Readable<boolean> = derived(showLocked, (locked) => !locked);

export function toggleShowLock(): void {
	showLocked.update((locked) => !locked);
}
