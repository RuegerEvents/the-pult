import { describe, expect, it } from 'vitest';

import type { EditableParameter } from './programmer.js';
import {
	encoderPage,
	encoderRole,
	encoderSlots,
	faderKeys,
	faderRaw,
	fromFader,
	hueOf,
	learnable,
	ledKeys,
	oops,
	pageCount,
	toFader,
	turned,
	typed,
	type WingMap
} from './wing.js';

/**
 * A slice of MA's table for the onPC command wing, with the indices a real one
 * reports: fader 0 is executor 201, its touch contact is hardkey 189, the key under it
 * is EXEC 201 at 283 and the one above it EXEC 101 at 289.
 */
const MAP: WingMap = {
	module: 'grandMA3 onPC command wing',
	hardkeys: {
		'155': { label: 'STORE', Code: 'STORE' },
		'189': { label: 'FADER exec 201', Code: 'FADER', ExecutorIndex: '201' },
		'283': { label: 'EXEC exec 201', Code: 'EXEC', ExecutorIndex: '201' },
		'289': { label: 'EXEC exec 101', Code: 'EXEC', ExecutorIndex: '101' },
		'237': { label: 'X1 exec 291', Code: 'X1', ExecutorIndex: '291' }
	},
	faders: { '0': { label: 'exec 201', ExecutorIndex: '201' } },
	encoders: {
		'12': { label: 'Inside1', Type: 'Inside1' },
		'13': { label: 'Outside1', Type: 'Outside1' },
		'21': { label: 'Outside5', Type: 'Outside5' },
		'6': { label: 'EXEC GrandKnob', Code: 'EXEC', SpecialExecutor: 'GrandKnob' }
	},
	leds: [
		{ label: 'STORE', Code: 'STORE', R: 55 },
		{ label: 'X1', Code: 'X1', R: 115, ExecutorIndex: '291' },
		{ label: 'exec 291', R: 88, G: 93, B: 106, ExecutorIndex: '291' },
		{ label: 'exec 201', R: 136, ExecutorIndex: '201' },
		{ label: '', R: 252 }
	]
};

describe('the keypad', () => {
	it('runs digits together and spaces everything else', () => {
		let line = '';
		for (const word of ['fixture', '1', '2', 'thru', '2', '0', 'at', '5', '0']) line = typed(line, word);
		expect(line).toBe('fixture 12 thru 20 at 50');
	});

	it('keeps a dot a word of its own, so it cuts cells rather than making a decimal', () => {
		expect(typed(typed('fixture 3', '.'), '1')).toBe('fixture 3 . 1');
	});

	it('takes back the last word with Oops, a run of digits being one', () => {
		expect(oops('fixture 12 thru 20')).toBe('fixture 12 thru');
		expect(oops('fixture')).toBe('');
	});
});

describe('the faders and their keys', () => {
	it('knows a touch contact, the Go under a fader and the Off above it', () => {
		const keys = faderKeys(MAP);
		expect(keys.get(189)).toEqual({ role: 'touch', fader: 0 });
		expect(keys.get(283)).toEqual({ role: 'go', fader: 0 });
		expect(keys.get(289)).toEqual({ role: 'off', fader: 0 });
		expect(keys.get(155)).toBeUndefined();
	});

	it('scales the 12-bit position both ways', () => {
		expect(faderRaw(1)).toBe(4096);
		expect(faderRaw(0.5)).toBe(2048);
		expect(toFader({ type: 'Float', value: 0.25 })).toBe(0.25);
		expect(fromFader({ type: 'Float', value: 0 }, 0.75)).toEqual({ type: 'Float', value: 0.75 });
	});

	it('keeps a color\'s hue through the bottom of the fader', () => {
		const amber = { type: 'Color' as const, value: { r: 1, g: 0.5, b: 0, overrides: {} } };
		const black = fromFader(amber, 0);
		expect(black).toEqual({ type: 'Color', value: { r: 0, g: 0, b: 0, overrides: {} } });
		// Black alone has no hue to bring back; the remembered one does.
		expect(fromFader(black, 0.5, hueOf(amber))).toEqual({
			type: 'Color',
			value: { r: 0.5, g: 0.25, b: 0, overrides: {} }
		});
		expect(hueOf(black)).toBeNull();
	});

	it('moves a color by its brightness, keeping its hue', () => {
		const amber = { type: 'Color' as const, value: { r: 1, g: 0.5, b: 0, overrides: {} } };
		expect(fromFader(amber, 0.5)).toEqual({ type: 'Color', value: { r: 0.5, g: 0.25, b: 0, overrides: {} } });
		expect(toFader(amber)).toBe(1);
	});
});

describe('the encoders', () => {
	it('reads which of the five an encoder is, outer coarse and inner fine', () => {
		expect(encoderRole(MAP, 13)).toEqual({ slot: 0, fine: false });
		expect(encoderRole(MAP, 12)).toEqual({ slot: 0, fine: true });
		expect(encoderRole(MAP, 21)).toEqual({ slot: 4, fine: false });
		expect(encoderRole(MAP, 6)).toBeUndefined();
	});

	const row = (key: string, value: EditableParameter['defaultValue']): EditableParameter => ({
		kind: key as EditableParameter['kind'],
		key,
		label: key,
		defaultValue: value,
		choices: [],
		physical: null,
		count: 1,
		unreachable: 0,
		mixed: false
	});

	it('pages intensity first, a color as three, and text not at all', () => {
		const slots = encoderSlots([
			row('Pan', { type: 'Float', value: 0.5 }),
			row('ColorRgb', { type: 'Color', value: { r: 0, g: 0, b: 0, overrides: {} } }),
			row('Named:Effect', { type: 'Text', value: '' }),
			row('Intensity', { type: 'Float', value: 0 })
		]);
		expect(slots.map((s) => s.label)).toEqual(['Intensity', 'Pan', 'Red', 'Green', 'Blue']);
		expect(pageCount(slots)).toBe(1);
		expect(encoderPage(slots, 0).length).toBe(5);
	});

	it('turns a float by 2% a click coarse and a quarter of one fine, inside its travel', () => {
		expect(turned({ type: 'Float', value: 0.5 }, 3, false)).toEqual({ type: 'Float', value: 0.56 });
		expect(turned({ type: 'Float', value: 0.5 }, 4, true)).toEqual({ type: 'Float', value: 0.51 });
		expect(turned({ type: 'Float', value: 0.99 }, 5, false)).toEqual({ type: 'Float', value: 1 });
	});

	it('steps a wheel one slot a click, and moves one channel of a color alone', () => {
		expect(turned({ type: 'Int', value: 2 }, -1, false)).toEqual({ type: 'Int', value: 1 });
		const red = turned({ type: 'Color', value: { r: 0.5, g: 0.5, b: 0.5, overrides: {} } }, 5, false, 'r');
		expect(red).toEqual({ type: 'Color', value: { r: 0.6, g: 0.5, b: 0.5, overrides: {} } });
	});
});

describe('the lights', () => {
	it('lights a key by its code and an executor button by its number, and no ring or lamp', () => {
		const keys = ledKeys(MAP);
		const of = (label: string) => keys.get(MAP.leds.find((l) => l.label === label)!);
		expect(of('STORE')).toBe(155);
		expect(of('X1')).toBe(237);
		expect(of('exec 201')).toBe(283);
		expect(of('exec 291')).toBeUndefined();
		expect(of('')).toBeUndefined();
	});
});

describe('learning from what the operator touches', () => {
	const parameter = { kind: 'Parameter' as const, fixture_id: 'f', parameter: 'Intensity' as const };
	const master = { kind: 'Master' as const, sequence_id: 's' };
	const go = { kind: 'Press' as const, target: { kind: 'SequenceGo' as const, sequence_id: 's' } };

	it('gives a fader a parameter or a level, and an encoder only a parameter', () => {
		expect(learnable({ kind: 'Fader', index: 0 }, parameter)).toEqual(parameter);
		expect(learnable({ kind: 'Fader', index: 0 }, master)).toEqual(master);
		expect(learnable({ kind: 'Fader', index: 0 }, go)).toBeNull();
		expect(learnable({ kind: 'Encoder', index: 13 }, master)).toBeNull();
	});

	it('gives a key a press, or a flash of the parameter it was shown', () => {
		expect(learnable({ kind: 'Key', index: 283 }, go)).toEqual(go);
		expect(learnable({ kind: 'Key', index: 283 }, parameter)).toEqual({
			kind: 'Flash',
			fixture_id: 'f',
			parameter: 'Intensity'
		});
		expect(learnable({ kind: 'Key', index: 283 }, master)).toBeNull();
	});
});
