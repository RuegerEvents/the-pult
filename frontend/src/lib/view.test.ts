import { describe, expect, it } from 'vitest';

import { DEFAULT_VIEW, parseView } from './stores/view.js';

describe('what a screen draws the rig as', () => {
	it('starts real, at forty percent work light, at one and a half pixels', () => {
		expect(parseView(null)).toEqual(DEFAULT_VIEW);
		expect(DEFAULT_VIEW.mode).toBe('real');
	});

	it('keeps a mode it knows and refuses one it does not', () => {
		expect(parseView({ mode: 'cones' }).mode).toBe('cones');
		// A showfile from a later build might name a mode this one has never heard
		// of; the view falls back rather than drawing nothing.
		expect(parseView({ mode: 'raytraced' }).mode).toBe('real');
	});

	it('changes one setting and keeps the rest', () => {
		const dark = parseView({ workLight: 0 }, { ...DEFAULT_VIEW, mode: 'photoreal' });
		expect(dark).toEqual({ ...DEFAULT_VIEW, mode: 'photoreal', workLight: 0 });
	});

	it('keeps a projection it knows and refuses one it does not', () => {
		expect(parseView({ projection: 'ortho' }).projection).toBe('ortho');
		expect(parseView({ projection: 'fisheye' }).projection).toBe(DEFAULT_VIEW.projection);
	});

	it('brings the numbers inside what the view will do', () => {
		expect(parseView({ workLight: 7, resolution: 0 })).toEqual({ ...DEFAULT_VIEW, workLight: 1, resolution: 0.5 });
		expect(parseView({ workLight: 'bright' }).workLight).toBe(DEFAULT_VIEW.workLight);
	});
});
