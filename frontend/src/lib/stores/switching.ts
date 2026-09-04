/**
 * The switch this page is in the middle of, if any.
 *
 * Kept in `sessionStorage` rather than in memory, deliberately: a switch ends in a
 * reload, and the reloaded page has to come up already saying "Opening Festival…"
 * rather than "Connecting to the console". Session rather than local storage
 * because a switch belongs to this tab — the tablet has its own, learned from the
 * station's close frame — and must not leak into a window opened next week.
 *
 * The rules — what a close code means, when a switch is overdue — are in
 * `$lib/switching.ts` and tested there. This is only the keeping.
 */

import { browser } from '$app/environment';
import { writable } from 'svelte/store';

import { plausible, type Switch } from '$lib/switching.js';

const STORAGE_KEY = 'pult.switching';

function readBack(): Switch | null {
	if (!browser) return null;
	try {
		const raw = sessionStorage.getItem(STORAGE_KEY);
		if (!raw) return null;
		const parsed: unknown = JSON.parse(raw);
		return plausible(parsed, Date.now()) ? parsed : null;
	} catch {
		return null;
	}
}

function keep(current: Switch | null) {
	if (!browser) return;
	try {
		if (current) sessionStorage.setItem(STORAGE_KEY, JSON.stringify(current));
		else sessionStorage.removeItem(STORAGE_KEY);
	} catch {
		// A browser with storage turned off still switches; the reload just shows
		// the connecting screen for a moment, which is what it used to do.
	}
}

/** The switch under way, or `null`. */
export const switching = writable<Switch | null>(readBack());

/**
 * Say that a switch has begun. Called *before* the station is asked, so the cover
 * is up before the socket goes, and by the client when a close frame says so.
 *
 * A switch already under way is not replaced: the first word is the one the
 * operator chose, and the station's close frame that follows says the same thing
 * in its own words.
 */
export function beginSwitch(doing: string): void {
	switching.update((current) => {
		if (current) return current;
		const next = { doing, since: Date.now() };
		keep(next);
		return next;
	});
}

/** The switch is over: the page is looking at whatever the console now has open. */
export function endSwitch(): void {
	switching.set(null);
	keep(null);
}
