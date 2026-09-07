/**
 * The waveform of a song, fetched once.
 *
 * The shape `stores/tracks.ts` has, and for the same reasons: an asset is
 * content-addressed, so it is fetched by its sha and never fetched again, and a fetch
 * that fails is *forgotten* rather than remembered as a failure — a station that has
 * not got the file yet is not a station without it, and the next update tries again.
 *
 * What is different is that this is a picture rather than data the evaluator needs, so
 * a page with no timeline panel open fetches nothing at all: the panel asks.
 */

import { decodePeaks, type Peaks } from '../waveform.js';

/** Where the console serves an asset from. Same origin, like everything else. */
const assetUrl = (sha: string) => `/assets/${sha}`;

const held = new Map<string, Peaks>();
const asking = new Set<string>();

/**
 * The reduced waveform behind an asset sha, or `null` while it is on its way and for
 * anything that is not one.
 *
 * Synchronous on the second call, which is what lets a canvas ask on every draw
 * without a subscription: the panel calls this and draws whatever comes back, and
 * `onReady` is only there to say "draw again now".
 */
export function peaksFor(sha: string, onReady?: () => void): Peaks | null {
	const found = held.get(sha);
	if (found) return found;
	if (asking.has(sha)) return null;
	asking.add(sha);

	void fetch(assetUrl(sha))
		.then((response) => {
			if (!response.ok) throw new Error(`${response.status}`);
			return response.arrayBuffer();
		})
		.then((bytes) => {
			const peaks = decodePeaks(bytes);
			if (peaks) held.set(sha, peaks);
			asking.delete(sha);
			onReady?.();
		})
		.catch(() => {
			// Forgotten rather than remembered as a failure: the station that computes
			// the waveform is the one playing the audio, and it may not have got round
			// to it yet — a panel that gave up would show a blank canvas for the rest
			// of the show.
			asking.delete(sha);
		});
	return null;
}
