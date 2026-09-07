/**
 * Recordings, in the page.
 *
 * A track is the fourth thing that can drive a parameter, and the only one that is
 * *data*: a fade is six numbers and a recording is however many change points somebody
 * sent while the tape was rolling. So it is an asset, fetched once by its sha and
 * handed to the evaluator as bytes — the same bytes the station reads, through the same
 * parser compiled twice, which is the rule the rest of the evaluator already lives by.
 *
 * Nothing here is arithmetic. Which recordings are running and where each playhead is
 * comes off the `timelines` rows; sampling one is `pult-render`'s and happens inside
 * the wasm at evaluate time, so a page and a lamp cannot disagree about what a take was
 * asserting at a moment.
 *
 * Two rules worth keeping.
 *
 * **A sha is fetched once and never dropped.** A show carries as many takes as somebody
 * recorded, and the alternative — letting go of one when its timeline stops — is a
 * first bar that is occasionally silent while a file is downloaded again.
 *
 * **A stopped timeline hands over no track at all**, rather than one with a frozen
 * playhead. A playing recording makes the evaluator say "this never settles", which is
 * right while it is rolling and would keep a settled rig redrawing for ever afterwards.
 */

import { readable, type Readable } from 'svelte/store';

import { forgetTrack, loadEvaluator, loadTrack, playTrack, stopTrack } from '../evaluator.js';
import type { Timeline } from '../generated/index.js';
import { collection } from './show.js';

/** Where the console serves an asset from. Same origin, like everything else. */
const assetUrl = (sha: string) => `/assets/${sha}`;

/** What one enabled track of one running timeline comes to. */
type Rolling = { sha: string; anchorMs: number; positionMs: number; rate: number };

/**
 * Which recordings a set of timelines is playing.
 *
 * A track's own `offset_ms` moves the playhead the *other* way: a take that arrived
 * 40 ms late is played 40 ms earlier. The same arithmetic the station does in
 * `playing_tracks`, and it has to be the same or the screen leads the lamps.
 */
export function rolling(timelines: Timeline[]): Rolling[] {
	const out: Rolling[] = [];
	for (const timeline of timelines) {
		if (!timeline.running) continue;
		for (const track of timeline.tracks) {
			if (!track.enabled) continue;
			out.push({
				sha: track.asset,
				anchorMs: timeline.anchor_ms,
				positionMs: Math.max(0, timeline.position_at_anchor_ms - track.offset_ms),
				rate: timeline.rate
			});
		}
	}
	return out;
}

/**
 * Keep the evaluator's recordings in step with the show.
 *
 * A store rather than a function so that it is bound to a subscriber's lifetime the
 * way the frame loop is: `stores/output.ts` holds it for as long as anything is being
 * drawn, and a page with no rig on it fetches nothing.
 *
 * The value is a count of loaded recordings, which is only there so the store has one:
 * everything this does happens inside the wasm.
 */
export const tracks: Readable<number> = readable(0, (set) => {
	let live = true;
	/** Shas whose bytes are in the evaluator, or on their way there. */
	const loaded = new Set<string>();
	/** Shas the evaluator has been told are playing. */
	const playing = new Set<string>();
	/** Every sha the show has mentioned, so one that goes can be let go of. */
	const known = new Set<string>();

	let stop: (() => void) | undefined;

	loadEvaluator().then((instance) => {
		if (!live || !instance) return;

		stop = collection('timelines').subscribe(($timelines) => {
			const timelines = $timelines as Timeline[];
			const wanted = rolling(timelines);
			const mentioned = new Set(
				timelines.flatMap((timeline) => timeline.tracks.map((track) => track.asset))
			);

			for (const sha of known) {
				if (mentioned.has(sha)) continue;
				// Deleted from the show, or the whole timeline went. Forgotten rather
				// than left behind, because a browser that stays open through an
				// afternoon of takes would otherwise hold every one of them.
				forgetTrack(sha);
				loaded.delete(sha);
				playing.delete(sha);
				known.delete(sha);
			}
			for (const sha of mentioned) known.add(sha);

			for (const sha of playing) {
				if (!wanted.some((track) => track.sha === sha)) {
					stopTrack(sha);
					playing.delete(sha);
				}
			}

			for (const track of wanted) {
				if (loaded.has(track.sha)) {
					playTrack(track.sha, track.anchorMs, track.positionMs, track.rate);
					playing.add(track.sha);
					continue;
				}
				// Marked before the fetch, so a second update arriving mid-download does
				// not start a second one for the same bytes.
				loaded.add(track.sha);
				void fetch(assetUrl(track.sha))
					.then((response) => {
						if (!response.ok) throw new Error(`${response.status}`);
						return response.arrayBuffer();
					})
					.then((bytes) => {
						if (!live) return;
						loadTrack(track.sha, new Uint8Array(bytes));
						playTrack(track.sha, track.anchorMs, track.positionMs, track.rate);
						playing.add(track.sha);
						set(loaded.size);
					})
					.catch((e) => {
						// A station that has not got the file yet. Forgotten so the next
						// change to the timelines tries again, rather than leaving the
						// take silently absent for the rest of the show.
						loaded.delete(track.sha);
						console.warn(`[pult] the recording ${track.sha.slice(0, 8)} could not be read`, e);
					});
			}
			set(loaded.size);
		});
	});

	return () => {
		live = false;
		stop?.();
		for (const sha of playing) stopTrack(sha);
		playing.clear();
	};
});
