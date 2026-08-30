import { describe, it, expect } from 'vitest';
import type {
	Cue,
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

const fixture = (over: Partial<Fixture> = {}): Fixture => ({
	id: 'f',
	name: 'Spot',
	fixture_type_id: 'mover',
	address: { Dmx: { universe: 1, address: 1 } },
	position: null,
	live_values: {},
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

	it('reports the value when the selection agrees', () => {
		const two = [
			fixture({ live_values: { Intensity: level(0.5) } }),
			fixture({ id: 'g', live_values: { Intensity: level(0.5) } })
		];
		expect(commonValue(two, 'Intensity')).toEqual({ value: level(0.5), mixed: false });
	});

	it('says so when it does not', () => {
		const two = [
			fixture({ live_values: { Intensity: level(0.5) } }),
			fixture({ id: 'g', live_values: { Intensity: level(0.9) } })
		];
		expect(commonValue(two, 'Intensity').mixed).toBe(true);
	});

	it('ignores fixtures that have never reported the parameter', () => {
		const two = [fixture({ live_values: { Intensity: level(0.5) } }), fixture({ id: 'g' })];
		expect(commonValue(two, 'Intensity')).toEqual({ value: level(0.5), mixed: false });
	});

	it('compares colours channel by channel', () => {
		const red: ParameterValue = { type: 'Color', value: { r: 1, g: 0, b: 0 } };
		const green: ParameterValue = { type: 'Color', value: { r: 0, g: 1, b: 0 } };
		expect(
			commonValue(
				[fixture({ live_values: { ColorRgb: red } }), fixture({ id: 'g', live_values: { ColorRgb: red } })],
				'ColorRgb'
			).mixed
		).toBe(false);
		expect(
			commonValue(
				[fixture({ live_values: { ColorRgb: red } }), fixture({ id: 'g', live_values: { ColorRgb: green } })],
				'ColorRgb'
			).mixed
		).toBe(true);
	});
});

describe('storing', () => {
	const entry = (over: Partial<ProgrammerValue> = {}): ProgrammerValue => ({
		id: 'e1',
		fixture_id: 'f',
		parameter_kind: 'Intensity',
		value: { type: 'Float', value: 0.7 },
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
				delay_in_ms: 0
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
