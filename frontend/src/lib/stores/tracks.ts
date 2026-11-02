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

import { composeTrack, forgetTrack, loadEvaluator, loadTrack, playTrack, stopTrack, type TrackPart } from '../evaluator.js';
import type { Timeline, TimelineFlash } from '../generated/index.js';
import { collection } from './show.js';

/** Where the console serves an asset from. Same origin, like everything else. */
const assetUrl = (sha: string) => `/assets/${sha}`;

/** What one running timeline's recordings come to: its rows, composed under one id. */
type Rolling = {
	id: string;
	parts: TrackPart[];
	flashes: TimelineFlash[];
	anchorMs: number;
	positionMs: number;
	rate: number;
	levels: boolean;
};

/**
 * Which recordings a set of timelines is playing: per running timeline, its enabled
 * rows, composed in the wasm exactly as the station's `playing_tracks` composes them —
 * each row its window of its parameter, nudged by its own offset, with the timeline's
 * flashes over them — and played on the timeline's own transport.
 */
export function rolling(timelines: Timeline[]): Rolling[] {
	const out: Rolling[] = [];
	for (const timeline of timelines) {
		if (!timeline.running) continue;
		const parts: TrackPart[] = timeline.tracks
			.filter((track) => track.enabled)
			.map((track) => ({
				sha: track.asset,
				fixture_id: track.fixture_id,
				key: track.key,
				from_ms: track.from_ms,
				to_ms: track.to_ms,
				offset_ms: track.offset_ms
			}));
		const flashes = timeline.flashes ?? [];
		if (parts.length === 0 && flashes.length === 0) continue;
		// A timeline recording a new pass leaves the sequences' levels to their faders,
		// as the station's `playing_tracks` does: the fader is what is being recorded.
		const recording = !!timeline.recording && 'Programmer' in timeline.recording;
		out.push({
			id: `timeline:${timeline.id}`,
			parts,
			flashes,
			anchorMs: timeline.anchor_ms,
			positionMs: timeline.position_at_anchor_ms,
			rate: timeline.rate,
			levels: !recording
		});
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
	/** Shas whose bytes have arrived. */
	const ready = new Set<string>();
	/** Composed ids the evaluator has been told are playing, with the rows each was composed from. */
	const playing = new Map<string, string>();
	/** Every sha the show has mentioned, so one that goes can be let go of. */
	const known = new Set<string>();

	let stop: (() => void) | undefined;
	let latest: Timeline[] = [];

	function apply() {
		const timelines = latest;
		const wanted = rolling(timelines);
		const mentioned = new Set(timelines.flatMap((timeline) => timeline.tracks.map((track) => track.asset)));

		for (const sha of known) {
			if (mentioned.has(sha)) continue;
			// Deleted from the show, or the whole timeline went. Forgotten rather than
			// left behind, because a browser that stays open through an afternoon of
			// takes would otherwise hold every one of them.
			forgetTrack(sha);
			loaded.delete(sha);
			ready.delete(sha);
			known.delete(sha);
		}
		for (const sha of mentioned) known.add(sha);

		for (const id of playing.keys()) {
			if (!wanted.some((track) => track.id === id)) {
				stopTrack(id);
				forgetTrack(id);
				playing.delete(id);
			}
		}

		for (const track of wanted) {
			for (const part of track.parts) fetchOnce(part.sha);
			// Composed again only when the rows or the bytes under them changed: a crop
			// dragged is a new composition per frame, and a cue taken is none.
			const signature = JSON.stringify([track.parts, track.flashes, track.parts.map((p) => ready.has(p.sha))]);
			if (playing.get(track.id) !== signature) {
				composeTrack(track.id, track.parts, track.flashes);
				playing.set(track.id, signature);
			}
			playTrack(track.id, track.anchorMs, track.positionMs, track.rate, track.levels);
		}
		set(ready.size);
	}

	function fetchOnce(sha: string) {
		if (loaded.has(sha)) return;
		// Marked before the fetch, so a second update arriving mid-download does not
		// start a second one for the same bytes.
		loaded.add(sha);
		void fetch(assetUrl(sha))
			.then((response) => {
				if (!response.ok) throw new Error(`${response.status}`);
				return response.arrayBuffer();
			})
			.then((bytes) => {
				if (!live) return;
				loadTrack(sha, new Uint8Array(bytes));
				ready.add(sha);
				apply();
			})
			.catch((e) => {
				// A station that has not got the file yet. Forgotten so the next change
				// to the timelines tries again, rather than leaving the take silently
				// absent for the rest of the show.
				loaded.delete(sha);
				console.warn(`[pult] the recording ${sha.slice(0, 8)} could not be read`, e);
			});
	}

	loadEvaluator().then((instance) => {
		if (!live || !instance) return;
		stop = collection('timelines').subscribe(($timelines) => {
			latest = $timelines as Timeline[];
			apply();
		});
	});

	return () => {
		live = false;
		stop?.();
		for (const id of playing.keys()) stopTrack(id);
		playing.clear();
	};
});
