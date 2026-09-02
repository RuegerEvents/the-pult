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
	RunningFade
} from './generated/index.js';
import { parameterKey } from './patch.js';

/** What the evaluator is given for one parameter. */
export type DrivenBy = {
	/** A plain value the programmer is holding. A held *shape* is not here: the
	 *  station resolves it against its speed master and publishes it as `effect`. */
	programmer?: ParameterValue;
	effect?: RunningEffect;
	fade?: RunningFade;
	home?: ParameterValue;
};

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
	held: Map<string, ProgrammerValue>
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
		if (Object.keys(row).length > 0) rows.set(key, row);
	}
	return rows;
}

/** The whole rig, flattened into the map the evaluator holds. */
export function drivingTheRig(
	fixtures: Fixture[],
	types: FixtureType[],
	entries: ProgrammerValue[]
): Record<string, DrivenBy> {
	const byId = new Map(types.map((t) => [t.id, t]));
	const held = new Map<string, ProgrammerValue>();
	for (const entry of entries) {
		held.set(drivingKey(entry.fixture_id, parameterKey(entry.parameter_kind)), entry);
	}

	const out: Record<string, DrivenBy> = {};
	for (const fixture of fixtures) {
		for (const [key, row] of drivenBy(fixture, byId.get(fixture.fixture_type_id), held)) {
			out[drivingKey(fixture.id, key)] = row;
		}
	}
	return out;
}
