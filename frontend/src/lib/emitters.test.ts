import { describe, expect, it } from 'vitest';

import type { Emitter, EmitterPreset, ParameterDefinition } from './generated/index.js';
import { anEmitter, hexOf, presetOf, vecOf } from './emitters.js';

const die = (name: string): Emitter => ({ name, rgb: null, subtractive: false });
const preset = (name: string, names: string[]): EmitterPreset => ({ name, emitters: names.map(die) });
const PRESETS = [preset('RGB', ['Red', 'Green', 'Blue']), preset('RGBW', ['Red', 'Green', 'Blue', 'White'])];

const color = (names: string[]): ParameterDefinition => ({
	kind: 'ColorRgb',
	direction: 'Output',
	binding: null,
	default_value: { type: 'Color', r: 1, g: 1, b: 1 } as unknown as ParameterDefinition['default_value'],
	physical: null,
	slots: [],
	feature_group: null,
	emitters: names.map(die),
	per_cell: false
});

describe('emitters', () => {
	it('reads a color that names none as plain RGB', () => {
		expect(presetOf(color([]), PRESETS)?.name).toBe('RGB');
	});

	it('matches a preset by names in order, since order is the byte order', () => {
		expect(presetOf(color(['Red', 'Green', 'Blue', 'White']), PRESETS)?.name).toBe('RGBW');
		expect(presetOf(color(['White', 'Red', 'Green', 'Blue']), PRESETS)).toBeNull();
	});

	it('round-trips a swatch', () => {
		expect(hexOf({ x: 1, y: 0.55, z: 0 })).toBe('#ff8c00');
		expect(vecOf('#ff8c00')).toEqual({ x: 1, y: 0.549, z: 0 });
		expect(hexOf(null)).toBe('#808080');
		expect(vecOf('nope')).toBeNull();
	});

	it('names a new emitter so it does not collide', () => {
		expect(anEmitter(['Red', 'White']).name).toBe('White 2');
	});
});
