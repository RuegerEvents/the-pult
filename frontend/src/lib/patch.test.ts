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
	kindOption,
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
		live_effects: {},
		live_fades: {},
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
		expect(parameterKey({ Named: 'Fog output' })).toBe('Named:Fog output');
	});

	// A node describes a port this console has no word for, and the name it gave is
	// the whole identity — in the key and on the operator's screen alike.
	it('shows a named kind under the name the device gave it', () => {
		expect(kindLabel({ Named: 'Fog output' })).toBe('Fog output');
		expect(parameterKey({ Named: 'Fog output' })).not.toBe(parameterKey({ Named: 'Tank level' }));
	});

	it('keeps every port of a numbered kind apart', () => {
		expect(parameterKey({ Contact: 0 })).not.toBe(parameterKey({ Contact: 1 }));
		expect(parameterKey({ Switch: 0 })).not.toBe(parameterKey({ Contact: 0 }));
	});
});

describe('picking a kind', () => {
	it('numbers the numbered kinds after the channel or port they are bound to', () => {
		expect(kindFromLabel('Switch', 2)).toEqual({ Switch: 2 });
		expect(kindFromLabel('Contact', 5)).toEqual({ Contact: 5 });
		expect(kindFromLabel('Raw', 7)).toEqual({ Raw: 7 });
		expect(kindFromLabel('Intensity', 5)).toBe('Intensity');
	});

	it('round-trips every kind in the picker back to the option it picks', () => {
		for (const label of PARAMETER_KINDS) {
			expect(kindOption(kindFromLabel(label, 1))).toBe(label);
		}
	});

	/**
	 * The two questions a kind gets asked, which are not the same question.
	 *
	 * `kindOption` answers "which of the fixed options is this", which is what a
	 * `<select>` needs. `kindLabel` answers "what is this called", and for a named
	 * parameter the answer is the name the device gave it — not the word "Named",
	 * which would tell an operator nothing about which port they were looking at.
	 */
	it('shows a named parameter its own name but picks the Named option', () => {
		const fog = kindFromLabel('Named', 0, 'Fog output');
		expect(fog).toEqual({ Named: 'Fog output' });
		expect(kindLabel(fog)).toBe('Fog output');
		expect(kindOption(fog)).toBe('Named');
	});

	/**
	 * A name is the whole identity of a named parameter: it is what the operator
	 * reads and what the `live_values` key is built from. An empty one would be a
	 * row with no label bound to a key of `Named:`, so it gets something visible
	 * to type over instead.
	 */
	it('never leaves a named parameter without a name', () => {
		expect(kindFromLabel('Named', 0)).toEqual({ Named: 'Unnamed' });
		expect(kindFromLabel('Named', 0, '   ')).toEqual({ Named: 'Unnamed' });
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

	/// The obvious definition: every fixture against every other. Slow, and
	/// self-evidently right, which is what makes it worth comparing against.
	function clashingPairwise(fixtures: Fixture[], span: (f: Fixture) => number): Set<string> {
		const clashing = new Set<string>();
		for (const a of fixtures) {
			for (const b of fixtures) {
				const da = 'Dmx' in a.address ? a.address.Dmx : null;
				const db = 'Dmx' in b.address ? b.address.Dmx : null;
				if (!da || !db || a.id === b.id || da.universe !== db.universe) continue;
				const aEnd = da.address + Math.max(span(a), 1) - 1;
				const bEnd = db.address + Math.max(span(b), 1) - 1;
				if (da.address <= bEnd && db.address <= aEnd) {
					clashing.add(a.id);
					clashing.add(b.id);
				}
			}
		}
		return clashing;
	}

	it('agrees with comparing every fixture against every other one', () => {
		// The sweep is subtle — it names only the furthest-reaching overlap it has
		// seen — so it is checked against the definition on a few hundred rigs
		// rather than on the handful of cases anyone would think to write out.
		let seed = 20260826;
		const random = (n: number) => {
			seed = (seed * 1103515245 + 12345) & 0x7fffffff;
			return seed % n;
		};

		for (let round = 0; round < 300; round++) {
			const fixtures = Array.from({ length: 1 + random(12) }, () =>
				atDmx(1 + random(3), 1 + random(20))
			);
			const spans = new Map(fixtures.map((f) => [f.id, 1 + random(6)]));
			const span = (f: Fixture) => spans.get(f.id) ?? 1;

			expect(clashingFixtures(fixtures, span)).toEqual(clashingPairwise(fixtures, span));
		}
	});

	it('flags every fixture in a long chain of overlaps', () => {
		// Each overlaps only its neighbours, so nothing is caught by a single
		// far-reaching fixture — the sweep has to carry the right one forward.
		const fixtures = Array.from({ length: 20 }, (_, i) => atDmx(1, 1 + i * 2));
		expect(clashingFixtures(fixtures, () => 3).size).toBe(20);
	});

	it('flags nothing in a rig that is packed but not overlapping', () => {
		const fixtures = Array.from({ length: 50 }, (_, i) => atDmx(1, 1 + i * 4));
		expect(clashingFixtures(fixtures, () => 4).size).toBe(0);
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
