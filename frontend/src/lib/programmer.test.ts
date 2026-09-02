import { describe, it, expect } from 'vitest';
import type {
	Cue,
	EffectSpec,
	Fixture,
	FixtureType,
	ParameterCapture,
	ParameterValue,
	ProgrammerValue
} from './generated/index.js';
import {
	asFloat,
	commonValue,
	editableParameters,
	entriesFromCue,
	entryId,
	hexToRgb,
	nudge,
	rgbToHex,
	storeCaptures,
	withFloat
} from './programmer.js';
import { readingOf } from './stores/output.js';

const fixture = (over: Partial<Fixture> = {}): Fixture => ({
	id: 'f',
	name: 'Spot',
	fixture_type_id: 'mover',
	address: { Dmx: { universe: 1, address: 1 } },
	position: null,
	sensed_values: {},
	live_effects: {},
	live_fades: {},
	home_values: {},
	...over
});

const mover: FixtureType = {
	id: 'mover',
	name: 'Mover',
	manufacturer: 'Generic',
	channel_count: 6,
	parameters: [
		{ kind: 'Intensity', direction: 'Output', binding: { Dmx: { channel: 1 } }, default_value: { type: 'Float', value: 0 } },
		{ kind: 'ColorRgb', direction: 'Output', binding: { Dmx: { channel: 2 } }, default_value: { type: 'Color', value: { r: 1, g: 1, b: 1 } } },
		{ kind: 'Pan', direction: 'Output', binding: { Dmx: { channel: 5 } }, default_value: { type: 'Float', value: 0.5 } }
	]
};

const dimmer: FixtureType = {
	...mover,
	id: 'dimmer',
	parameters: [mover.parameters[0]]
};

const sensor: FixtureType = {
	...mover,
	id: 'sensor',
	parameters: [
		{ kind: { Contact: 0 }, direction: 'Input', binding: { Port: { index: 0 } }, default_value: { type: 'Bool', value: false } }
	]
};

describe('entry ids', () => {
	it('gives the same id for the same parameter every time', () => {
		expect(entryId('a', 'Intensity')).toBe(entryId('a', 'Intensity'));
	});

	it('gives different ids to different parameters and different fixtures', () => {
		expect(entryId('a', 'Intensity')).not.toBe(entryId('a', 'Pan'));
		expect(entryId('a', 'Intensity')).not.toBe(entryId('b', 'Intensity'));
	});

	it('produces something the backend will accept as a uuid', () => {
		expect(entryId('a', 'Intensity')).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
		);
	});

	it('does not confuse a fixture and a key that run together', () => {
		// "ab" + "/" + "c" and "a" + "/" + "bc" are different parameters.
		expect(entryId('ab', 'c')).not.toBe(entryId('a', 'bc'));
	});

	it('agrees with the command-line plugin by pinned example', () => {
		// These exact pairs are asserted to these exact ids in the Rust
		// reimplementation too (plugins/command-line/core/src/ids.rs). The two
		// derivations must agree or two writers of one fader get two rows —
		// a change that moves either side breaks one suite or the other.
		expect(entryId('2f6b535b-9a71-4c39-9d95-6d6ab2f0f639', 'Intensity')).toBe(
			'5f13b718-4585-810f-9f90-15d7509267f4'
		);
		expect(entryId('00000000-0000-0000-0000-000000000000', 'ColorRgb')).toBe(
			'3ad6b4b5-4891-8a54-ae06-93999b3641bd'
		);
	});
});

describe('what a selection can be given', () => {
	it('lists a type’s output parameters in the order it lists them', () => {
		const rows = editableParameters([mover], [fixture()]);
		expect(rows.map((r) => r.key)).toEqual(['Intensity', 'ColorRgb', 'Pan']);
	});

	it('unions across a mixed selection and says which are not shared', () => {
		const rows = editableParameters(
			[mover, dimmer],
			[fixture(), fixture({ id: 'g', fixture_type_id: 'dimmer' })]
		);
		expect(rows.map((r) => r.key)).toEqual(['Intensity', 'ColorRgb', 'Pan']);
		expect(rows.find((r) => r.key === 'Intensity')!.mixed).toBe(false);
		expect(rows.find((r) => r.key === 'Pan')!.mixed).toBe(true);
	});

	it('leaves out parameters the show reads rather than writes', () => {
		expect(editableParameters([sensor], [fixture({ fixture_type_id: 'sensor' })])).toEqual([]);
	});

	it('has nothing to offer for a fixture whose type has gone', () => {
		expect(editableParameters([], [fixture()])).toEqual([]);
	});

	it('has nothing to offer for an empty selection', () => {
		expect(editableParameters([mover], [])).toEqual([]);
	});
});

describe('reading a selection back', () => {
	const level = (v: number): ParameterValue => ({ type: 'Float', value: v });

	const two = [fixture(), fixture({ id: 'g' })];

	it('reports the value when the selection agrees', () => {
		const showing = readingOf({ 'f/Intensity': level(0.5), 'g/Intensity': level(0.5) });
		expect(commonValue(two, 'Intensity', showing)).toEqual({ value: level(0.5), mixed: false });
	});

	it('says so when it does not', () => {
		const showing = readingOf({ 'f/Intensity': level(0.5), 'g/Intensity': level(0.9) });
		expect(commonValue(two, 'Intensity', showing).mixed).toBe(true);
	});

	it('ignores fixtures nothing can say anything about', () => {
		const showing = readingOf({ 'f/Intensity': level(0.5) });
		expect(commonValue(two, 'Intensity', showing)).toEqual({ value: level(0.5), mixed: false });
	});

	it('compares colours channel by channel', () => {
		const red: ParameterValue = { type: 'Color', value: { r: 1, g: 0, b: 0 } };
		const green: ParameterValue = { type: 'Color', value: { r: 0, g: 1, b: 0 } };
		expect(
			commonValue(two, 'ColorRgb', readingOf({ 'f/ColorRgb': red, 'g/ColorRgb': red })).mixed
		).toBe(false);
		expect(
			commonValue(two, 'ColorRgb', readingOf({ 'f/ColorRgb': red, 'g/ColorRgb': green })).mixed
		).toBe(true);
	});
});

describe('storing', () => {
	const entry = (over: Partial<ProgrammerValue> = {}): ProgrammerValue => ({
		id: 'e1',
		fixture_id: 'f',
		parameter_kind: 'Intensity',
		value: { type: 'Float', value: 0.7 },
		effect: null,
		locked: false,
		...over
	});

	const capture = (over: Partial<ParameterCapture> = {}): ParameterCapture => ({
		fixture_id: 'f',
		parameter_kind: 'Intensity',
		value: { type: 'Float', value: 0.1 },
		fade_in_ms: 0,
		fade_out_ms: 0,
		delay_in_ms: 0,
		effect: null,
		easing: 'Linear',
		...over
	});

	it('merges over the same parameter and leaves the rest of the cue alone', () => {
		const existing = [capture(), capture({ fixture_id: 'g' })];
		const stored = storeCaptures(existing, [entry()], 'merge', new Set(['e1']));
		expect(stored).toHaveLength(2);
		expect(stored.find((c) => c.fixture_id === 'f')!.value).toEqual({ type: 'Float', value: 0.7 });
		expect(stored.find((c) => c.fixture_id === 'g')!.value).toEqual({ type: 'Float', value: 0.1 });
	});

	it('keeps a capture for a different parameter of the same fixture', () => {
		const existing = [capture({ parameter_kind: 'Pan' })];
		const stored = storeCaptures(existing, [entry()], 'merge', new Set(['e1']));
		expect(stored).toHaveLength(2);
	});

	it('replaces the cue with what is in the programmer', () => {
		const existing = [capture({ fixture_id: 'g' })];
		const stored = storeCaptures(existing, [entry()], 'replace', new Set(['e1']));
		expect(stored).toHaveLength(1);
		expect(stored[0].fixture_id).toBe('f');
	});

	it('stores only what was ticked', () => {
		const entries = [entry(), entry({ id: 'e2', parameter_kind: 'Pan' })];
		expect(storeCaptures([], entries, 'replace', new Set(['e2']))).toHaveLength(1);
		expect(storeCaptures([], entries, 'replace', new Set())).toEqual([]);
	});

	it('leaves times at zero so the cue’s own timing is inherited', () => {
		const [stored] = storeCaptures([], [entry()], 'replace', new Set(['e1']));
		expect([stored.fade_in_ms, stored.fade_out_ms, stored.delay_in_ms]).toEqual([0, 0, 0]);
	});

	it('deselecting everything under merge leaves the cue exactly as it was', () => {
		const existing = [capture()];
		expect(storeCaptures(existing, [entry()], 'merge', new Set())).toEqual(existing);
	});
});

describe('loading a cue back into the programmer', () => {
	const cue: Cue = {
		id: 'c',
		name: 'Look',
		number: 1,
		captures: [
			{
				fixture_id: 'f',
				parameter_kind: 'Intensity',
				value: { type: 'Float', value: 0.4 },
				fade_in_ms: 0,
				fade_out_ms: 0,
				delay_in_ms: 0,
				effect: null,
				easing: 'Linear'
			}
		],
		follow_mode: 'Manual',
		fade_in_ms: 500,
		fade_out_ms: 500,
		is_active: false
	};

	it('gives one entry per capture, keyed the way a live grab would be', () => {
		const [loaded] = entriesFromCue(cue);
		expect(loaded.id).toBe(entryId('f', 'Intensity'));
		expect(loaded.value).toEqual({ type: 'Float', value: 0.4 });
		expect(loaded.locked).toBe(false);
	});

	it('round-trips back into the same captures', () => {
		const entries = entriesFromCue(cue);
		const stored = storeCaptures([], entries, 'replace', new Set(entries.map((e) => e.id)));
		expect(stored).toEqual(cue.captures);
	});
});

describe('values', () => {
	it('reads a number out of anything that has one', () => {
		expect(asFloat({ type: 'Float', value: 0.25 })).toBe(0.25);
		expect(asFloat({ type: 'Int', value: 7 })).toBe(7);
		expect(asFloat({ type: 'Bool', value: true })).toBe(1);
		expect(asFloat({ type: 'Text', value: 'x' })).toBeNull();
		expect(asFloat(undefined)).toBeNull();
	});

	it('puts a number back into the kind it came from', () => {
		expect(withFloat({ type: 'Float', value: 0 }, 0.5)).toEqual({ type: 'Float', value: 0.5 });
		expect(withFloat({ type: 'Int', value: 0 }, 12.4)).toEqual({ type: 'Int', value: 12 });
		expect(withFloat({ type: 'Bool', value: false }, 1)).toEqual({ type: 'Bool', value: true });
	});

	it('keeps a float inside its range', () => {
		expect(withFloat({ type: 'Float', value: 0 }, 2)).toEqual({ type: 'Float', value: 1 });
		expect(withFloat({ type: 'Float', value: 0 }, -1)).toEqual({ type: 'Float', value: 0 });
	});

	it('round-trips a colour through hex', () => {
		expect(rgbToHex({ r: 1, g: 0, b: 0 })).toBe('#ff0000');
		expect(hexToRgb('#ff0000')).toEqual({ r: 1, g: 0, b: 0 });
		expect(hexToRgb('f00')).toEqual({ r: 1, g: 0, b: 0 });
	});

	it('refuses something that is not a colour', () => {
		expect(hexToRgb('nonsense')).toBeNull();
		expect(hexToRgb('#12345')).toBeNull();
	});

	it('nudges each kind by a share of its own range', () => {
		expect(nudge({ type: 'Float', value: 0.5 }, 0.1)).toEqual({ type: 'Float', value: 0.6 });
		expect(nudge({ type: 'Int', value: 100 }, 0.1)).toEqual({ type: 'Int', value: 126 });
		expect(nudge({ type: 'Color', value: { r: 0.5, g: 0, b: 1 } }, 0.25)).toEqual({
			type: 'Color',
			value: { r: 0.75, g: 0.25, b: 1 }
		});
	});

	it('leaves text where it is, having nowhere to nudge it to', () => {
		const text: ParameterValue = { type: 'Text', value: 'Beware' };
		expect(nudge(text, 0.5)).toEqual(text);
	});
});

describe('an effect through store and back', () => {
	const spec = (t0: number | null): EffectSpec => ({
		effect_id: 'fx',
		curve: { Shape: 'Sine' },
		rate: { Hz: 0.5 },
		low: { type: 'Float', value: 0 },
		high: { type: 'Float', value: 1 },
		width: 0.5,
		direction: 'Forward',
		phase: 0.25,
		spread: 'Linear',
		t0
	});

	const held: ProgrammerValue = {
		id: 'e1',
		fixture_id: 'f',
		parameter_kind: 'Intensity',
		value: { type: 'Float', value: 0 },
		effect: spec(1_000),
		locked: false
	};

	it('drops the anchor on the way in, because the cue supplies one', () => {
		const [capture] = storeCaptures([], [held], 'replace', new Set(['e1']));

		expect(capture.effect).not.toBeNull();
		expect(capture.effect?.t0).toBeNull();
		expect(capture.effect?.phase).toBe(0.25);
		expect(capture.effect?.rate).toEqual({ Hz: 0.5 });
	});

	it('takes a fresh one on the way out, because the operator is holding it again', () => {
		const [capture] = storeCaptures([], [held], 'replace', new Set(['e1']));
		const before = Date.now();

		const [restored] = entriesFromCue({
			id: 'c',
			name: 'Look',
			number: 1,
			captures: [capture],
			follow_mode: 'Manual',
			fade_in_ms: 0,
			fade_out_ms: 0,
			is_active: false
		});

		expect(restored.effect?.phase).toBe(0.25);
		expect(restored.effect?.t0).toBeGreaterThanOrEqual(before);
	});

	it('leaves a plain value alone', () => {
		const plain = { ...held, effect: null };
		const [capture] = storeCaptures([], [plain], 'replace', new Set(['e1']));

		expect(capture.effect).toBeNull();
		expect(entriesFromCue({
			id: 'c',
			name: 'Look',
			number: 1,
			captures: [capture],
			follow_mode: 'Manual',
			fade_in_ms: 0,
			fade_out_ms: 0,
			is_active: false
		})[0].effect).toBeNull();
	});
});
