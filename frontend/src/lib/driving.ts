/**
 * What is driving each parameter of the rig, as the evaluator wants it.
 *
 * The console keeps *what is driving* a parameter — the fade or shape anchored in
 * console time, the programmer over it, the home value beneath — and nobody keeps the
 * answer. This turns the rows a browser already has into the four layers the evaluator
 * takes, and it is all this file does: there is no arithmetic here, deliberately.
 * Turning those layers into a number happens in one place, in Rust, compiled for both
 * the station and the page.
 */

import type {
	Fixture,
	FixtureType,
	ParameterValue,
	ProgrammerValue,
	RunningEffect,
	RunningFade,
	Sequence
} from './generated/index.js';
import { fadeGroup } from './fade.js';
import { parameterKey } from './patch.js';

/** What the evaluator is given for one parameter. */
export type DrivenBy = {
	/** A plain value the programmer is holding. A held *shape* is not here: the
	 *  station resolves it against its speed master and publishes it as `effect`. */
	programmer?: ParameterValue;
	effect?: RunningEffect;
	fade?: RunningFade;
	home?: ParameterValue;
	/** The playing sequence's fader, where it dims this parameter. */
	master?: number;
	/** The sequence whose cue is driving this intensity, so a level a playing take
	 *  recorded for it is put over `master` by the evaluator, frame by frame. */
	sequence?: string;
};

/** Which sequence each cue belongs to, for every sequence at any level. */
export type CueSequences = Map<string, string>;

export function cueSequences(sequences: Sequence[]): CueSequences {
	const out: CueSequences = new Map();
	for (const sequence of sequences) for (const cue of sequence.cue_ids) out.set(cue, sequence.id);
	return out;
}

/** The cue whose playback layer wins this parameter: an effect over a fade. */
function winningCue(effect: RunningEffect | undefined, fade: RunningFade | undefined): string | undefined {
	if (effect) return effect.source === 'Programmer' ? undefined : effect.source.Cue;
	return fade?.cue_id;
}

/**
 * The sequences' playback faders, by the cues they hold — only those below full.
 *
 * `Masters` in `crates/pult-schema/src/types/sequence.rs` is the station's, and this
 * is the page's copy of the same small rule; the arithmetic it feeds is the
 * evaluator's and is not repeated anywhere.
 */
export type Masters = Map<string, number>;

export function mastersOf(sequences: Sequence[]): Masters {
	const out: Masters = new Map();
	for (const sequence of sequences) {
		const level = sequence.master;
		if (level === null || level === undefined || level >= 1) continue;
		for (const cue of sequence.cue_ids) out.set(cue, Math.min(1, Math.max(0, level)));
	}
	return out;
}

/**
 * The fader over this parameter, if one dims it: an intensity, played by a sequence
 * below full, through whichever playback layer wins — an effect over a fade.
 */
export function masterOver(
	masters: Masters,
	key: string,
	effect: RunningEffect | undefined,
	fade: RunningFade | undefined
): number | undefined {
	if (masters.size === 0 || fadeGroup(key) !== 'Intensity') return undefined;
	const cue = winningCue(effect, fade);
	return cue === undefined ? undefined : masters.get(cue);
}

/** How a parameter is named everywhere the evaluator is concerned. */
export const drivingKey = (fixtureId: string, key: string) => `${fixtureId}/${key}`;

/**
 * Where a parameter rests when nothing is driving it: this fixture's own override, or
 * what its type declares.
 *
 * The same resolution `home_value_by_key` makes on the station, and it has to stay the
 * same — a browser that invented its own answer would draw an untouched house light
 * dark while the station had it up.
 */
export function homeValue(
	fixture: Fixture,
	type: FixtureType | undefined,
	key: string
): ParameterValue | undefined {
	const own = fixture.home_values[key];
	if (own !== undefined) return own;
	return type?.parameters.find((p) => parameterKey(p.kind) === key)?.default_value;
}

/**
 * Everything acting on one fixture, keyed by parameter key.
 *
 * Only the parameters something can say anything about: a key with no fade, no shape,
 * no hold and no home is a key nothing has ever driven and nothing can place, and
 * listing it would only make the evaluator answer "nothing" more often.
 */
export function drivenBy(
	fixture: Fixture,
	type: FixtureType | undefined,
	held: Map<string, ProgrammerValue>,
	masters: Masters = new Map(),
	sequenceOfCue: CueSequences = new Map()
): Map<string, DrivenBy> {
	const rows = new Map<string, DrivenBy>();
	const keys = new Set<string>([
		...Object.keys(fixture.live_effects),
		...Object.keys(fixture.live_fades),
		...Object.keys(fixture.home_values),
		...(type?.parameters ?? []).map((p) => parameterKey(p.kind))
	]);

	for (const key of keys) {
		const entry = held.get(drivingKey(fixture.id, key));
		const row: DrivenBy = {};
		// An entry carrying a shape asserts the shape, never a value: grabbing a fader
		// and putting a sine on it are the same act of taking hold of one parameter.
		if (entry && !entry.effect) row.programmer = entry.value;
		const effect = fixture.live_effects[key];
		if (effect) row.effect = effect;
		const fade = fixture.live_fades[key];
		if (fade) row.fade = fade;
		const home = homeValue(fixture, type, key);
		if (home !== undefined) row.home = home;
		const master = masterOver(masters, key, effect, fade);
		if (master !== undefined) row.master = master;
		if (sequenceOfCue.size > 0 && fadeGroup(key) === 'Intensity') {
			const cue = winningCue(effect, fade);
			const sequence = cue === undefined ? undefined : sequenceOfCue.get(cue);
			if (sequence !== undefined) row.sequence = sequence;
		}
		if (Object.keys(row).length > 0) rows.set(key, row);
	}
	return rows;
}

/** The whole rig, flattened into the map the evaluator holds. */
export function drivingTheRig(
	fixtures: Fixture[],
	types: FixtureType[],
	entries: ProgrammerValue[],
	sequences: Sequence[] = []
): Record<string, DrivenBy> {
	const masters = mastersOf(sequences);
	const sequenceOfCue = cueSequences(sequences);
	const byId = new Map(types.map((t) => [t.id, t]));
	const held = new Map<string, ProgrammerValue>();
	for (const entry of entries) {
		held.set(drivingKey(entry.fixture_id, parameterKey(entry.parameter_kind)), entry);
	}

	const out: Record<string, DrivenBy> = {};
	for (const fixture of fixtures) {
		for (const [key, row] of drivenBy(fixture, byId.get(fixture.fixture_type_id), held, masters, sequenceOfCue)) {
			out[drivingKey(fixture.id, key)] = row;
		}
	}
	return out;
}
