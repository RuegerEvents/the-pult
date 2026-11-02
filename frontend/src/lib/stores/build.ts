/**
 * A stale page reloads itself — and says so first, and waits for a dialog.
 *
 * `isStale` in `$lib/build.ts` is the rule; this is the act. A short countdown in a
 * toast rather than a reload out of nowhere, because the operator at the tablet should
 * know why the screen went white. And never under an open dialog: a Store being filled
 * in or a take being reviewed is somebody's half-made decision, and the reload waits
 * until the last dialog closes, saying so.
 *
 * The stamp it reloaded for is kept in `sessionStorage`, so a page that comes back
 * still different — a cache in the way — does not reload again for the same build.
 */

import { browser, dev, version } from '$app/environment';
import { get } from 'svelte/store';

import { isStale } from '$lib/build.js';
import { addToast } from '$lib/toasts.js';
import { dialogCount } from './dialogs.js';

const RELOADED_FOR = 'pult.reloadedFor';
/** How long the toast stands before the reload. */
const COUNTDOWN_MS = 3000;

let pending = false;

/** The station serves this build; reload if it is not this page's. */
export function checkBuild(station: string | null | undefined): void {
	if (!browser || pending) return;
	const reloadedFor = sessionStorage.getItem(RELOADED_FOR);
	if (!isStale(version, station, dev, reloadedFor)) return;
	pending = true;
	const reload = () => {
		sessionStorage.setItem(RELOADED_FOR, station ?? '');
		location.reload();
	};
	if (get(dialogCount) > 0) {
		addToast('The station has a new version of the console. Reloading once this dialog is closed.', 'info');
		const stop = dialogCount.subscribe((open) => {
			if (open > 0) return;
			queueMicrotask(() => stop());
			setTimeout(reload, COUNTDOWN_MS);
		});
		return;
	}
	addToast('The station has a new version of the console. Reloading…', 'info');
	setTimeout(reload, COUNTDOWN_MS);
}
