/**
 * Tempo, tapped.
 *
 * A speed master is a number several effects follow, and the way an operator sets
 * one is by tapping along with the band. Everything here is pure so it can be tested
 * without a screen; the panel does the writing.
 */

import type { SpeedMaster } from './generated/index.js';

/**
 * A gap long enough that the operator has stopped tapping and started again.
 *
 * Two seconds is 30 bpm. Slower than that is not a tempo anybody taps, and treating
 * a pause as an interval would drag the average down for the next four taps.
 */
export const TAP_RESTART_MS = 2_000;

/** How many intervals the average is taken over. */
export const TAP_WINDOW = 8;
const TAP_MINIMUM = 2;

/**
 * The tempo a run of tap times implies, or null while there is not enough to say.
 *
 * Averaged over the last few intervals rather than taken from the last one, because
 * a hand is not a metronome and a single 40 ms slip is 10 bpm at speed. Anything
 * before a gap of [`TAP_RESTART_MS`] is dropped: the operator was tapping something
 * else, or was not tapping at all.
 */
export function bpmFromTaps(taps: number[]): number | null {
	const run = sinceLastGap(taps);
	if (run.length < TAP_MINIMUM) return null;

	const intervals: number[] = [];
	for (let i = 1; i < run.length; i++) intervals.push(run[i] - run[i - 1]);

	const recent = intervals.slice(-TAP_WINDOW);
	const mean = recent.reduce((a, b) => a + b, 0) / recent.length;
	if (!(mean > 0)) return null;
	return 60_000 / mean;
}

/**
 * The taps since the operator last stopped.
 *
 * Exported because the panel shows how many are in the run: tapping four times and
 * seeing "4" is the difference between a tempo you trust and a number that appeared.
 */
export function sinceLastGap(taps: number[]): number[] {
	let start = 0;
	for (let i = 1; i < taps.length; i++) {
		if (taps[i] - taps[i - 1] > TAP_RESTART_MS) start = i;
	}
	return taps.slice(start);
}

/** Round to a tenth. A bpm shown to six decimal places is a bpm nobody trusts. */
export function tidyBpm(bpm: number): number {
	return Math.round(bpm * 10) / 10;
}

/**
 * The rate an effect on this master runs at, before its own multiplier.
 *
 * Beats are per minute and effects are per second, and the master's multiplier is
 * what lets one tempo drive a half-speed sweep and a double-speed chase at once.
 * A stopped master is zero, which holds every effect on it at its phase rather than
 * dropping it — stopping a chase should freeze the look, not turn the lights off.
 */
export function effectiveHz(master: Pick<SpeedMaster, 'bpm' | 'multiplier' | 'running'>): number {
	if (!master.running) return 0;
	return (master.bpm / 60) * master.multiplier;
}

/**
 * Where in its beat the master is at `nowMs`, 0..1.
 *
 * The same arithmetic the engine and every node use, for the same reason: the dot on
 * the panel has to be on the beat the lights are on, not on the beat this browser
 * would have worked out for itself.
 */
export function beatPhase(
	master: Pick<SpeedMaster, 'bpm' | 'multiplier' | 'running' | 't0'>,
	nowMs: number
): number {
	const hz = effectiveHz(master);
	if (hz <= 0) return 0;
	const cycles = ((nowMs - master.t0) / 1000) * hz;
	// Wrapped rather than truncated: a master whose anchor is in the future — which
	// clock skew between two consoles makes ordinary — belongs at the top of the
	// beat, not at a negative position.
	return ((cycles % 1) + 1) % 1;
}

/** The multipliers the panel offers. Halving and doubling is most of what is wanted. */
export const MULTIPLIERS = [0.25, 0.5, 1, 2, 4] as const;
