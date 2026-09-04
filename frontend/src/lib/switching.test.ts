import { describe, expect, it } from 'vitest';

import {
	overdue,
	plausible,
	SWITCH_PATIENCE_MS,
	SWITCHING_SHOWS,
	switchFromClose,
	switchTitle
} from './switching.js';

describe('what a close frame means', () => {
	it("is a switch when it carries the station's own code, in the station's words", () => {
		expect(switchFromClose(SWITCHING_SHOWS, 'opening Festival.pult', 1000)).toEqual({
			doing: 'opening Festival.pult',
			since: 1000
		});
	});

	it('is not a switch for any other close', () => {
		// A process that died, a network that went, a tab the browser throttled: a
		// lost console, and drawn as one. Pretending it was a switch would put up a
		// screen that promises the page will follow a station that is not coming.
		expect(switchFromClose(1006, '', 1000)).toBeNull();
		expect(switchFromClose(1000, 'opening Festival.pult', 1000)).toBeNull();
	});

	it('still counts with no reason given', () => {
		expect(switchFromClose(SWITCHING_SHOWS, '', 5)?.doing).toBe('changing shows');
	});
});

describe('what the screen says', () => {
	it('capitalises the phrase and trails off', () => {
		expect(switchTitle('opening Festival.pult')).toBe('Opening Festival.pult…');
		expect(switchTitle('')).toBe('Changing shows…');
	});
});

describe('a switch that never ends', () => {
	it('is overdue after the patience runs out, and not before', () => {
		const began = { doing: 'closing the show', since: 10_000 };
		expect(overdue(began, 10_000 + SWITCH_PATIENCE_MS - 1)).toBe(false);
		expect(overdue(began, 10_000 + SWITCH_PATIENCE_MS + 1)).toBe(true);
	});
});

describe('what storage hands back', () => {
	it('believes a recent, well-formed switch', () => {
		expect(plausible({ doing: 'opening A.pult', since: 900 }, 1000)).toBe(true);
	});

	it('does not believe the wrong shape, the future, or last week', () => {
		expect(plausible(null, 1000)).toBe(false);
		expect(plausible({ doing: 3 }, 1000)).toBe(false);
		expect(plausible({ doing: 'x', since: 2000 }, 1000)).toBe(false);
		expect(plausible({ doing: 'x', since: 0 }, 2 * 60 * 60 * 1000)).toBe(false);
	});
});
