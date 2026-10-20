import { describe, expect, it } from 'vitest';

import type { EffectLayer, Fixture, FixtureType } from './generated/index.js';
import {
	draggedCycles,
	draggedLevel,
	draggedRate,
	draggedWidth,
	gradientCurve,
	gridCurve,
	gridLevels,
	havingKind,
	layerPhases,
	pixelSpread,
	readPixelSpread,
	templateApply
} from './effectPool.js';
import { physicalReadout } from './units.js';

const chase: EffectLayer = {
	kind: 'Intensity',
	curve: { Shape: 'Square' },
	rate: { Hz: 1 },
	low: { type: 'Float', value: 0 },
	high: { type: 'Float', value: 1 },
	width: 0.25,
	direction: 'Forward',
	spread: 'Linear',
	spread_cycles: 1
};

const color: EffectLayer = {
	...chase,
	kind: 'ColorRgb',
	low: { type: 'Color', value: { r: 1, g: 0, b: 0, overrides: {} } },
	high: { type: 'Color', value: { r: 1, g: 0.5, b: 0, overrides: {} } }
};

function rig(): { fixtures: Map<string, Fixture>; types: Map<string, FixtureType> } {
	const param = (kind: unknown) => ({ kind, direction: 'Output' });
	const types = new Map<string, FixtureType>([
		['par', { id: 'par', parameters: [param('Intensity'), param('ColorRgb')] } as unknown as FixtureType],
		['dim', { id: 'dim', parameters: [param('Intensity')] } as unknown as FixtureType],
		['pixel', { id: 'pixel', parameters: [param('ColorRgb')] } as unknown as FixtureType]
	]);
	const fixture = (id: string, type: string) =>
		({ id, fixture_type_id: type }) as unknown as Fixture;
	const fixtures = new Map<string, Fixture>([
		['a', fixture('a', 'par')],
		['b', fixture('b', 'dim')],
		['c', fixture('c', 'par')],
		['p1', fixture('p1', 'pixel')],
		['p2', fixture('p2', 'pixel')]
	]);
	return { fixtures, types };
}

describe('applying a template', () => {
	it('spreads in the selection’s order, not the patch’s', () => {
		const { fixtures, types } = rig();
		const [only] = templateApply({ id: 't', layers: [chase] }, ['c', 'b', 'a'], fixtures, types);
		expect(only.order).toEqual(['c', 'b', 'a']);
		expect(only.specs.c.phase).toBe(0);
		expect(only.specs.b.phase).toBeCloseTo(1 / 3);
		expect(only.specs.a.phase).toBeCloseTo(2 / 3);
	});

	it('gives each layer to the fixtures that have its parameter', () => {
		const { fixtures, types } = rig();
		const applies = templateApply({ id: 't', layers: [chase, color] }, ['a', 'b', 'c'], fixtures, types);
		expect(applies.map((a) => a.order)).toEqual([
			['a', 'b', 'c'],
			['a', 'c']
		]);
		// A layer is one effect: every spec in it shares an id, and remembers the template.
		const ids = new Set(Object.values(applies[1].specs).map((s) => s.effect_id));
		expect(ids.size).toBe(1);
		expect(applies[1].specs.a.template).toBe('t');
	});

	it('leaves out a layer nothing selected can take', () => {
		const { fixtures, types } = rig();
		const applies = templateApply({ id: 't', layers: [chase, color] }, ['b'], fixtures, types);
		expect(applies).toHaveLength(1);
		expect(havingKind('ColorRgb', ['b'], fixtures, types)).toEqual([]);
	});
});

describe('a pixel has no dimmer', () => {
	it('takes an intensity layer as a gray of that level', () => {
		const { fixtures, types } = rig();
		const [only] = templateApply({ id: 't', layers: [chase] }, ['p1', 'p2'], fixtures, types);
		expect(only.kind).toBe('ColorRgb');
		expect(only.layer).toBe(0);
		expect(only.specs.p2.high).toEqual({ type: 'Color', value: { r: 1, g: 1, b: 1, overrides: {} } });
		expect(only.specs.p2.phase).toBeCloseTo(0.5);
	});
	it('but not where the template says what color it is', () => {
		const { fixtures, types } = rig();
		const applies = templateApply({ id: 't', layers: [chase, color] }, ['p1', 'a'], fixtures, types);
		// The intensity reaches the par; the color reaches both; nothing grays the pixel.
		expect(applies.map((a) => [a.layer, a.kind, a.order])).toEqual([
			[0, 'Intensity', ['a']],
			[1, 'ColorRgb', ['p1', 'a']]
		]);
	});
});

describe('spread over more than one cycle', () => {
	it('repeats a chase along a strip', () => {
		const two = layerPhases({ spread: 'Linear', spread_cycles: 2 }, 4);
		expect(two).toEqual([0, 0.5, 0, 0.5]);
	});
	it('none at all is everything together', () => {
		expect(layerPhases({ spread: 'Linear', spread_cycles: 0 }, 3)).toEqual([0, 0, 0]);
	});
});

describe('a strip’s toggles', () => {
	it('are the spreads the engine already has', () => {
		expect(pixelSpread({ reverse: false, mirror: false })).toBe('Linear');
		expect(pixelSpread({ reverse: true, mirror: false })).toBe('Reversed');
		expect(pixelSpread({ reverse: true, mirror: true })).toBe('Center');
		expect(readPixelSpread('Center')).toEqual({ reverse: false, mirror: true });
	});
	it('a gradient is evenly spaced crossfading colors', () => {
		const curve = gradientCurve([
			{ r: 1, g: 0, b: 0 },
			{ r: 0, g: 0, b: 1 }
		]);
		expect('Steps' in curve && curve.Steps.map((s) => [s.at, s.easing])).toEqual([
			[0, 'Linear'],
			[0.5, 'Linear']
		]);
	});
});

describe('the step grid', () => {
	it('round-trips through a step list', () => {
		const curve = gridCurve([1, 0, 0.75, 0], false);
		expect(gridLevels(curve)).toEqual([1, 0, 0.75, 0]);
		expect('Steps' in curve && curve.Steps[2].at).toBe(0.5);
		expect('Steps' in curve && curve.Steps.every((s) => s.easing === 'Step')).toBe(true);
	});
	it('a color step list is not a grid', () => {
		expect(gridLevels(gradientCurve([{ r: 1, g: 0, b: 0 }]))).toBeNull();
		expect(gridLevels({ Shape: 'Sine' })).toBeNull();
	});
});

describe('dragging', () => {
	it('doubles the rate every 120 px, from where the drag began', () => {
		expect(draggedRate(1, 120)).toBe(2);
		expect(draggedRate(1, -120)).toBe(0.5);
		expect(draggedRate(1, 0)).toBe(1);
		expect(draggedRate(10, 1000)).toBe(20);
	});
	it('keeps a width off both ends and snaps the spread to quarters', () => {
		expect(draggedWidth(1.2)).toBe(0.95);
		expect(draggedWidth(-1)).toBe(0.05);
		expect(draggedCycles(0.3)).toBe(1.25);
		expect(draggedCycles(2)).toBe(4);
		expect(draggedLevel(0.333)).toBe(0.33);
	});
});

describe('a fader in its own unit', () => {
	it('reads a strip’s speed in hertz', () => {
		expect(physicalReadout(0.1, { from: 0, to: 10, unit: 'Hertz' }).short).toBe('1.0 Hz');
		expect(physicalReadout(1, { from: 0, to: 10, unit: 'Hertz' }).text).toBe('10.0 Hz');
	});
	it('reads a parameter with no range as a percent, as before', () => {
		expect(physicalReadout(0.5, null)).toEqual({ short: '50', text: '50%' });
	});
	it('reads a pan in whole degrees', () => {
		expect(physicalReadout(0.5, { from: -270, to: 270, unit: 'Degrees' }).short).toBe('0°');
	});
});
