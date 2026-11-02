/**
 * Takes, outlined for drawing on a lane.
 *
 * `stores/tracks.ts` loads a take into the evaluator only while its timeline runs,
 * because that is when it drives anything. A lane draws takes whether or not anything
 * is running, so it asks for an outline instead: the bytes fetched once by sha and read
 * through `pult_render::outline` in the wasm — the same parse the station plays from,
 * so there is no second reader of the format here either.
 *
 * Kept for as long as the page is up, like the evaluator's own copy: a show carries as
 * many takes as somebody recorded, they are small, and an outline that went and came
 * back would be a lane that flickers empty while it is fetched again.
 */

import { writable, type Readable } from 'svelte/store';

import { loadEvaluator, trackOutline, type KeyOutline } from '$lib/evaluator.js';

/** An outline, or `'loading'`, or `'failed'` (a station that has not got the file yet). */
export type OutlineState = KeyOutline[] | 'loading' | 'failed';

const store = writable<Record<string, OutlineState>>({});

export const outlines: Readable<Record<string, OutlineState>> = { subscribe: store.subscribe };

/** Ask for a take's outline. Repeated asks for one sha cost nothing. */
export function wantOutline(sha: string): void {
	let asked = false;
	store.update((all) => {
		if (all[sha] && all[sha] !== 'failed') return all;
		asked = true;
		return { ...all, [sha]: 'loading' };
	});
	if (!asked) return;
	void (async () => {
		try {
			await loadEvaluator();
			const response = await fetch(`/assets/${sha}`);
			if (!response.ok) throw new Error(`${response.status}`);
			const outline = trackOutline(new Uint8Array(await response.arrayBuffer()));
			store.update((all) => ({ ...all, [sha]: outline ?? 'failed' }));
		} catch {
			// Marked failed rather than left loading, so the next ask tries again.
			store.update((all) => ({ ...all, [sha]: 'failed' }));
		}
	})();
}
