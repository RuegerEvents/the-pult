import { describe, expect, it } from 'vitest';

import type { FadeCurves } from './generated/index.js';
import { curveForKey, fadeGroup, resolveEasing } from './fade.js';

/** What a new show has, which is the case every one of these is really about. */
const curves: FadeCurves = {
	intensity: 'Linear',
	position: 'EaseInOut',
	color: 'Linear',
	beam: 'Linear',
	other: 'Linear'
};

describe('which group a parameter is in', () => {
	it('reads the name, not the index', () => {
		expect(fadeGroup('Gobo:1')).toBe('Beam');
		expect(fadeGroup('Gobo:2')).toBe('Beam');
		expect(fadeGroup('ColorWheel:1')).toBe('Color');
		expect(fadeGroup('Named:Fog output')).toBe('Other');
	});

	it('puts the pair that matters on opposite sides', () => {
		expect(fadeGroup('Intensity')).toBe('Intensity');
		expect(fadeGroup('Pan')).toBe('Position');
		expect(fadeGroup('Tilt')).toBe('Position');
	});
});

describe('what a fade will actually take', () => {
	it('asks the capture, then the cue, then the show', () => {
		expect(resolveEasing(curves, 'Step', 'Linear', 'Pan')).toBe('Step');
		expect(resolveEasing(curves, null, 'Linear', 'Pan')).toBe('Linear');
		expect(resolveEasing(curves, null, null, 'Pan')).toBe('EaseInOut');
	});

	it('answers per group and not once for the whole cue', () => {
		// The one thing a single show-level curve could not do, and the reason there
		// are five of them: one cue moving a head and dimming a lamp wants both.
		expect(curveForKey(curves, 'Pan')).toBe('EaseInOut');
		expect(curveForKey(curves, 'Intensity')).toBe('Linear');
	});
});
