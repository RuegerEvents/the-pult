import { describe, expect, it } from 'vitest';
import type { Fixture, ParameterValue } from './generated/index.js';
import {
	addressLabel,
	channelRange,
	clashingFixtures,
	defaultDirectionFor,
	defaultValueFor,
	formatValue,
	isDmx,
	kindFromLabel,
	kindLabel,
	nextFreeAddress,
	parameterKey,
	parameterKindLabel,
	PARAMETER_KINDS
} from './patch.js';

function aFixture(partial: Partial<Fixture> = {}): Fixture {
	return {
		id: crypto.randomUUID(),
		name: 'Spot',
		fixture_type_id: 'type',
		address: { Dmx: { universe: 1, address: 1 } },
		position: null,
		live_values: {},
		active_preset: null,
		...partial
	};
}

const atDmx = (universe: number, address: number, partial: Partial<Fixture> = {}) =>
	aFixture({ address: { Dmx: { universe, address } }, ...partial });

const onNode = (serial: string, universe: number | null = null) =>
	aFixture({ address: { OpenHaunt: { serial, universe } } });

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
		expect(parameterKey({ Switch: 0 })).toBe('Switch:0');
		expect(parameterKey({ Contact: 3 })).toBe('Contact:3');
		expect(parameterKey('Temperature')).toBe('Temperature');
		expect(parameterKey('Humidity')).toBe('Humidity');
		expect(parameterKey('AirQuality')).toBe('AirQuality');
		expect(parameterKey('Text')).toBe('Text');
	});

	it('keeps every port of a numbered kind apart', () => {
		expect(parameterKey({ Contact: 0 })).not.toBe(parameterKey({ Contact: 1 }));
		expect(parameterKey({ Switch: 0 })).not.toBe(parameterKey({ Contact: 0 }));
	});
});

describe('picking a kind', () => {
	it('numbers a switch or contact after the port it is bound to', () => {
		expect(kindFromLabel('Switch', 2)).toEqual({ Switch: 2 });
		expect(kindFromLabel('Contact', 5)).toEqual({ Contact: 5 });
		expect(kindFromLabel('Intensity', 5)).toBe('Intensity');
	});

	it('round-trips every kind in the picker back to its own label', () => {
		for (const label of PARAMETER_KINDS) {
			expect(kindLabel(kindFromLabel(label, 1))).toBe(label);
		}
	});

	it('reads sensors and drives everything else', () => {
		expect(defaultDirectionFor({ Contact: 0 })).toBe('Input');
		expect(defaultDirectionFor('Temperature')).toBe('Input');
		expect(defaultDirectionFor({ Switch: 0 })).toBe('Output');
		expect(defaultDirectionFor('Intensity')).toBe('Output');
	});
});

describe('defaults for a kind', () => {
	it('gives colour a colour and intensity a number', () => {
		expect(defaultValueFor('ColorRgb')).toEqual({ type: 'Color', value: { r: 0, g: 0, b: 0 } });
		expect(defaultValueFor('Intensity')).toEqual({ type: 'Float', value: 0 });
		expect(defaultValueFor('GoboIndex')).toEqual({ type: 'Int', value: 0 });
	});

	it('gives a contact a boolean and a display a string', () => {
		expect(defaultValueFor({ Contact: 0 })).toEqual({ type: 'Bool', value: false });
		expect(defaultValueFor({ Switch: 1 })).toEqual({ type: 'Bool', value: false });
		expect(defaultValueFor('Text')).toEqual({ type: 'Text', value: '' });
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

	it('shows text as itself, and empty text as nothing driven', () => {
		expect(formatValue({ type: 'Text', value: 'BOO' })).toBe('BOO');
		expect(formatValue({ type: 'Text', value: '' })).toBe('–');
	});

	it('shows a dash for a parameter that has never been driven', () => {
		expect(formatValue(undefined as unknown as ParameterValue)).toBe('–');
	});
});

describe('addresses', () => {
	it('tells a DMX fixture from one on a node', () => {
		expect(isDmx(atDmx(1, 1))).toBe(true);
		expect(isDmx(onNode('1a2b3c'))).toBe(false);
	});

	it('labels a DMX fixture by universe and address', () => {
		expect(addressLabel(atDmx(2, 17))).toBe('2 / 17');
	});

	it('labels a node fixture by serial, with its universe only if it gateways one', () => {
		expect(addressLabel(onNode('1a2b3c'))).toBe('1a2b3c');
		expect(addressLabel(onNode('1a2b3c', 5))).toBe('1a2b3c · universe 5');
	});
});

describe('channel ranges', () => {
	it('shows a single channel as one number', () => {
		expect(channelRange(atDmx(1, 10), 1)).toBe('10');
	});

	it('shows a multi-channel fixture as a range ending on its last channel', () => {
		expect(channelRange(atDmx(1, 10), 4)).toBe('10–13');
	});

	it('treats a zero-channel type as occupying one channel', () => {
		expect(channelRange(atDmx(1, 10), 0)).toBe('10');
	});

	it('shows nothing for a fixture that occupies no channels at all', () => {
		expect(channelRange(onNode('1a2b3c'), 4)).toBe('');
	});
});

describe('address clashes', () => {
	const span = () => 4;

	it('finds nothing wrong with fixtures that do not overlap', () => {
		expect(clashingFixtures([atDmx(1, 1), atDmx(1, 5)], span).size).toBe(0);
	});

	it('flags both fixtures when they overlap', () => {
		const a = atDmx(1, 1);
		const b = atDmx(1, 3);
		expect(clashingFixtures([a, b], span)).toEqual(new Set([a.id, b.id]));
	});

	it('flags an exact double patch', () => {
		expect(clashingFixtures([atDmx(1, 1), atDmx(1, 1)], span).size).toBe(2);
	});

	it('ignores overlaps across different universes', () => {
		expect(clashingFixtures([atDmx(1, 1), atDmx(2, 1)], span).size).toBe(0);
	});

	it('does not flag a fixture against itself', () => {
		expect(clashingFixtures([atDmx(1, 1)], span).size).toBe(0);
	});

	it('flags every fixture in a pile-up, not just the first pair', () => {
		expect(clashingFixtures([atDmx(1, 1), atDmx(1, 2), atDmx(1, 3)], span).size).toBe(3);
	});

	it('never flags a fixture that has no channels to clash over', () => {
		const node = onNode('1a2b3c', 1);
		const light = atDmx(1, 1);
		expect(clashingFixtures([node, light, onNode('1a2b3c', 1)], span).size).toBe(0);
	});
});

describe('the next free address', () => {
	const span = () => 4;

	it('starts at one in an empty universe', () => {
		expect(nextFreeAddress([], 1, span)).toBe(1);
		expect(nextFreeAddress([atDmx(2, 1)], 1, span)).toBe(1);
	});

	it('lands after the last fixture in the universe', () => {
		expect(nextFreeAddress([atDmx(1, 1), atDmx(1, 5)], 1, span)).toBe(9);
	});

	it('is not moved by fixtures that are not on DMX at all', () => {
		expect(nextFreeAddress([onNode('1a2b3c', 1), atDmx(1, 1)], 1, span)).toBe(5);
	});
});
