/**
 * The detector's proposal as a tempo, for showing.
 *
 * Deliberately simpler than `pult_audio::grid_from_beats`, which splits a song at every
 * tempo change: what the panel is for here is *showing* the answer, and an operator who
 * wants the rallentando split out adds a segment.
 *
 * What it does **not** simplify is the tempo, because the obvious reading is wrong. The
 * model works at 50 frames a second, so every beat is quantized to 20 ms — at 128 bpm a
 * 468.75 ms beat lands on 460 or 480, whose median is 460, and `60000 / median` is
 * 130.4. Over a minute that walks a whole bar out of step. So the tempo comes from the
 * **span** divided by the number of beats the span holds, with each interval asked how
 * many beats it covers so that a beat the detector missed counts as the two it stands
 * for. The station's own function does exactly this, for the same reason.
 */

import type { Detected, GridSegment } from '$lib/generated/index.js';

export function gridFrom(found: Detected): GridSegment[] {
	const beats = found.beats_ms;
	if (beats.length < 2) return [];
	const gaps = beats.slice(1).map((at, i) => at - beats[i]);
	const median = [...gaps].sort((a, b) => a - b)[Math.floor(gaps.length / 2)];
	if (!median) return [];
	const span = beats[beats.length - 1] - beats[0];
	const held = gaps.reduce((sum, gap) => sum + Math.max(1, Math.round(gap / median)), 0);
	const bpm = span > 0 ? (60_000 * held) / span : 60_000 / median;
	const first = found.downbeats_ms[0] ?? beats[0];
	const bar =
		found.downbeats_ms.length > 1 ? Math.max(1, Math.round((found.downbeats_ms[1] - found.downbeats_ms[0]) / median)) : 4;
	return [{ at_ms: first, bpm: Math.round(bpm * 10) / 10, beats_per_bar: bar }];
}
