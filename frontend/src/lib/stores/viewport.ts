/**
 * Whether this screen is a phone.
 *
 * A tiled workspace is a desk's arrangement: four tiles on a 390-pixel screen are
 * four panels nobody can use. Under `COMPACT_WIDTH` the workspace shows one panel at
 * a time from the same layout (`CompactWorkspace.svelte`), and the top bar wraps and
 * drops its words for its icons. The same width is the `@media` breakpoint the
 * stylesheets use, so the two cannot disagree about which screen this is.
 */

import { browser } from '$app/environment';
import { readable, writable } from 'svelte/store';

/** In CSS pixels. A phone held upright is well under it; a tablet is over it. */
export const COMPACT_WIDTH = 720;

export const compact = readable(false, (set) => {
	if (!browser || typeof window.matchMedia !== 'function') return;
	const query = window.matchMedia(`(max-width: ${COMPACT_WIDTH}px)`);
	set(query.matches);
	const changed = (e: MediaQueryListEvent) => set(e.matches);
	query.addEventListener('change', changed);
	return () => query.removeEventListener('change', changed);
});

/**
 * The one panel a compact workspace is showing, or `null` for its first.
 *
 * Its own store rather than the desk's `maximized`, which is a moment of needing
 * room: a phone turned sideways past the breakpoint should come back to its tiles,
 * not to whatever the phone happened to be looking at.
 */
export const compactPanel = writable<string | null>(null);
