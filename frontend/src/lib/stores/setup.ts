/**
 * Setup: a mode, not a window in the workspace.
 *
 * Which panels are errands rather than work surfaces is `layout/panels.ts`'s
 * `home`; this is only whether the dialog is open and which section it is on. Both
 * are this browser's — two operators at two screens can have different sections
 * open, and a setup dialog left up must not travel to the next tab.
 *
 * The section is remembered in `sessionStorage` rather than `localStorage`: coming
 * back to Patch during one call is helpful, and opening tomorrow's console on
 * whatever was last configured is not.
 */

import { browser } from '$app/environment';
import { get, writable } from 'svelte/store';

const STORAGE_KEY = 'pult.setup.section';

/** The section on screen, or `null` when setup is shut. */
export const setupSection = writable<string | null>(null);

/** The section to open on when nothing else says. */
export const DEFAULT_SECTION = 'patch';

function remembered(): string {
	if (!browser) return DEFAULT_SECTION;
	try {
		return sessionStorage.getItem(STORAGE_KEY) ?? DEFAULT_SECTION;
	} catch {
		return DEFAULT_SECTION;
	}
}

/** Open setup, on a named section or on the one this tab was last looking at. */
export function openSetup(section?: string): void {
	setupSection.set(section ?? remembered());
	if (browser && section) {
		try {
			sessionStorage.setItem(STORAGE_KEY, section);
		} catch {
			// A tab with storage off still opens; it just starts on Patch each time.
		}
	}
}

export function showSection(section: string): void {
	openSetup(section);
}

export function closeSetup(): void {
	setupSection.set(null);
}

export function toggleSetup(): void {
	if (get(setupSection) === null) openSetup();
	else closeSetup();
}
