import { describe, expect, it } from 'vitest';

import { masterOver, mastersOf } from './driving.js';
import type { RunningEffect, RunningFade, Sequence } from './generated/index.js';

const fade = (cue_id: string): RunningFade => ({
	from: { type: 'Float', value: 0 },
	to: { type: 'Float', value: 1 },
	t0: 0,
	duration_ms: 0,
	easing: 'Linear',
	cue_id
});

const sequence = (cue_ids: string[], master: number | null): Sequence => ({
	id: 'seq',
	name: 'Act',
	cue_ids,
	active_cue_index: 0,
	went_at: null,
	master
});

/** The page's copy of `Masters::over`; the station's tests are the ones in sequence.rs. */
describe('a sequence fader', () => {
	it('reaches the intensities its own cues play, and nothing else', () => {
		const masters = mastersOf([sequence(['cue'], 0.4)]);
		expect(masterOver(masters, 'Intensity', undefined, fade('cue'))).toBe(0.4);
		expect(masterOver(masters, 'Intensity@3', undefined, fade('cue'))).toBe(0.4);
		expect(masterOver(masters, 'Pan', undefined, fade('cue'))).toBeUndefined();
		expect(masterOver(masters, 'Intensity', undefined, fade('other'))).toBeUndefined();
	});

	it('follows the effect over the fade, and never a programmer shape', () => {
		const masters = mastersOf([sequence(['cue'], 0.5)]);
		const effect = (source: RunningEffect['source']) => ({ source }) as RunningEffect;
		expect(masterOver(masters, 'Intensity', effect({ Cue: 'cue' }), fade('other'))).toBe(0.5);
		expect(masterOver(masters, 'Intensity', effect('Programmer'), fade('cue'))).toBeUndefined();
	});

	it('keeps nothing for a sequence at full', () => {
		expect(mastersOf([sequence(['cue'], null), sequence(['c2'], 1)]).size).toBe(0);
	});
});
