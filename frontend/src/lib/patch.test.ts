import { describe, expect, it } from 'vitest';
import type { Fixture, ParameterValue } from './generated/index.js';
import {
	channelRange,
	clashingFixtures,
	defaultValueFor,
	formatValue,
	parameterKey,
	parameterKindLabel
} from './patch.js';

function aFixture(partial: Partial<Fixture> = {}): Fixture {
	return {
		id: crypto.randomUUID(),
		name: 'Spot',
		fixture_type_id: 'type',
		universe: 1,
		dmx_address: 1,
		position: null,
		live_values: {},
		active_preset: null,
		...partial
	};
}

describe('parameter keys', () => {
	it('names the plain kinds after themselves', () => {
		expect(parameterKindLabel('Intensity')).toBe('Intensity');
		expect(parameterKindLabel('ColorRgb')).toBe('ColorRgb');
	});

	it('gives each raw channel its own key', () => {
		expect(parameterKindLabel({ Raw: 5 })).toBe('Raw:5');
		expect(parameterKey({ Raw: 5 })).not.toBe(parameterKey({ Raw: 6 }));
	});

	// The backend's parameter_key writes these, so a mismatch means the patch table
	// silently shows nothing for a parameter that is in fact moving.
	it('matches the keys the backend writes', () => {
		expect(parameterKey('Intensity')).toBe('Intensity');
		expect(parameterKey({ Raw: 12 })).toBe('Raw:12');
	});
});

describe('defaults for a kind', () => {
	it('gives colour a colour and intensity a number', () => {
		expect(defaultValueFor('ColorRgb')).toEqual({ type: 'Color', value: { r: 0, g: 0, b: 0 } });
		expect(defaultValueFor('Intensity')).toEqual({ type: 'Float', value: 0 });
		expect(defaultValueFor('GoboIndex')).toEqual({ type: 'Int', value: 0 });
	});
});

describe('formatting a live value', () => {
	it('shows a level as a percentage', () => {
		expect(formatValue({ type: 'Float', value: 1 })).toBe('100%');
		expect(formatValue({ type: 'Float', value: 0.5 })).toBe('50%');
		expect(formatValue({ type: 'Float', value: 0 })).toBe('0%');
	});

	it('shows a colour as hex', () => {
		expect(formatValue({ type: 'Color', value: { r: 1, g: 0.5, b: 0 } })).toBe('#ff8000');
	});

	it('clamps a colour rather than producing nonsense hex', () => {
		expect(formatValue({ type: 'Color', value: { r: 4, g: -1, b: 0 } })).toBe('#ff0000');
	});

	it('shows booleans and integers plainly', () => {
		expect(formatValue({ type: 'Bool', value: true })).toBe('on');
		expect(formatValue({ type: 'Int', value: 7 })).toBe('7');
	});

	it('shows a dash for a parameter that has never been driven', () => {
		expect(formatValue(undefined as unknown as ParameterValue)).toBe('–');
	});
});

describe('channel ranges', () => {
	it('shows a single channel as one number', () => {
		expect(channelRange(aFixture({ dmx_address: 10 }), 1)).toBe('10');
	});

	it('shows a multi-channel fixture as a range ending on its last channel', () => {
		expect(channelRange(aFixture({ dmx_address: 10 }), 4)).toBe('10–13');
	});

	it('treats a zero-channel type as occupying one channel', () => {
		expect(channelRange(aFixture({ dmx_address: 10 }), 0)).toBe('10');
	});
});

describe('address clashes', () => {
	const span = () => 4;

	it('finds nothing wrong with fixtures that do not overlap', () => {
		const fixtures = [aFixture({ dmx_address: 1 }), aFixture({ dmx_address: 5 })];
		expect(clashingFixtures(fixtures, span).size).toBe(0);
	});

	it('flags both fixtures when they overlap', () => {
		const a = aFixture({ dmx_address: 1 });
		const b = aFixture({ dmx_address: 3 });
		const clashes = clashingFixtures([a, b], span);
		expect(clashes).toEqual(new Set([a.id, b.id]));
	});

	it('flags an exact double patch', () => {
		const a = aFixture({ dmx_address: 1 });
		const b = aFixture({ dmx_address: 1 });
		expect(clashingFixtures([a, b], span).size).toBe(2);
	});

	it('ignores overlaps across different universes', () => {
		const a = aFixture({ dmx_address: 1, universe: 1 });
		const b = aFixture({ dmx_address: 1, universe: 2 });
		expect(clashingFixtures([a, b], span).size).toBe(0);
	});

	it('does not flag a fixture against itself', () => {
		expect(clashingFixtures([aFixture()], span).size).toBe(0);
	});

	it('flags every fixture in a pile-up, not just the first pair', () => {
		const a = aFixture({ dmx_address: 1 });
		const b = aFixture({ dmx_address: 2 });
		const c = aFixture({ dmx_address: 3 });
		expect(clashingFixtures([a, b, c], span).size).toBe(3);
	});
});
