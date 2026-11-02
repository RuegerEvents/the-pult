/**
 * CLEAR, in steps, the way MA's works.
 *
 * Each press clears the first thing there is: **the selection, then the programmer.**
 * Decided by what is there rather than by a timer, so the button, Esc and the wing's
 * key all mean the same thing at any speed, and the next press is always readable off
 * the screen: a selection showing means the next Clear drops it.
 *
 * The programmer step is everything unlocked, which is everything this console is
 * putting on stage over playback — the programmer *is* output here, so MA's separate
 * "clear active" and "clear all" are one step. A *locked* value is parked on purpose
 * and Clear leaves it, as it always has; the Held panel's Clear is how one goes.
 *
 * This was the other way round until 2026-10-25: the programmer first, the selection
 * on a quick second press. Changed at the gig for the operator's MA habit, where the
 * first Clear after setting a look drops the selection and the look stays up.
 */

import { derived, get, type Readable } from 'svelte/store';

import { clear as clearProgrammer, entries } from './programmer.js';
import { clearSelection, selection } from './selection.js';

export type ClearStep = 'selection' | 'programmer' | null;

/** What the next Clear will clear, or `null` when there is nothing to. */
export const nextClear: Readable<ClearStep> = derived(
	[selection, entries],
	([$selection, $entries]) => {
		if ($selection.length > 0) return 'selection';
		if ($entries.some((entry) => !entry.locked)) return 'programmer';
		return null;
	}
);

/** One press of Clear. Answers what it cleared. */
export async function clearStep(): Promise<ClearStep> {
	const step = get(nextClear);
	if (step === 'selection') clearSelection();
	else if (step === 'programmer') await clearProgrammer({ keepLocked: true });
	return step;
}
