import { describe, it, expect } from 'vitest';

import type { DrivenBy } from './driving.js';
import { NO_CUE, cellsOfCue, source, trackedSource } from './sheet.js';
import type { Cue, ParameterCapture, ParameterValue } from './generated/index.js';

const value: ParameterValue = { type: 'Float', value: 0.5 };
const CUE_A = '11111111-1111-4111-8111-111111111111';
const CUE_B = '22222222-2222-4222-8222-222222222222';

const fade = (cueId: string) => ({
	from: value,
	to: value,
	t0: 0,
	duration_ms: 1000,
	easing: 'Linear' as const,
	cue_id: cueId
});

describe('what is driving one cell', () => {
	it('says nothing about a parameter nothing has ever driven', () => {
		expect(source(undefined, false, null)).toBe('none');
		expect(source({}, false, null)).toBe('none');
	});

	it('is home when only a home value places it', () => {
		expect(source({ home: value }, false, null)).toBe('home');
	});

	/** The stack, top down. Each of these beats everything below it. */
	it('puts the programmer above everything', () => {
		const driven: DrivenBy = { programmer: value, fade: fade(CUE_A), home: value };
		expect(source(driven, true, CUE_A)).toBe('programmer');
	});

	it('puts a recording above the playback under it', () => {
		expect(source({ fade: fade(CUE_A), home: value }, true, CUE_A)).toBe('track');
	});

	it('puts an effect above a fade', () => {
		const driven = {
			effect: { effect_id: 'e' },
			fade: fade(CUE_A),
			home: value
		} as unknown as DrivenBy;
		expect(source(driven, false, null)).toBe('effect');
	});

	/**
	 * The distinction the sheet exists for: with a cue on screen, that cue's own fade
	 * is hard and anybody else's is tracked. With no cue on screen there is nothing to
	 * be tracked *from*, so every cue's fade is simply a cue's.
	 */
	it('tells hard from tracked against the cue being looked at', () => {
		expect(source({ fade: fade(CUE_A) }, false, CUE_A)).toBe('cue');
		expect(source({ fade: fade(CUE_B) }, false, CUE_A)).toBe('tracked');
		expect(source({ fade: fade(CUE_B) }, false, null)).toBe('cue');
	});

	/**
	 * A release fades to exactly where the parameter rests, so colouring it as a cue's
	 * would name a cue that has stopped asserting anything.
	 */
	it('reads a fade with no cue behind it as home', () => {
		expect(source({ fade: fade(NO_CUE), home: value }, false, CUE_A)).toBe('home');
	});

	/** A take rolling over a parameter the show has never driven is still a take. */
	it('reports a recording over a parameter with no row at all', () => {
		expect(source(undefined, true, null)).toBe('track');
	});
});

describe('a capture that survived tracking', () => {
	const cue = (id: string) => ({ id }) as Cue;

	it('is hard in the cue being looked at and tracked from any other', () => {
		expect(trackedSource(cue(CUE_A), CUE_A)).toBe('cue');
		expect(trackedSource(cue(CUE_B), CUE_A)).toBe('tracked');
	});
});

describe('the cells of a cue', () => {
	const capture = (fixture: string, kind: ParameterCapture['parameter_kind']) =>
		({ fixture_id: fixture, parameter_kind: kind, value }) as ParameterCapture;

	it('keys by fixture and parameter, so one fixture can hold several', () => {
		const cue = { id: CUE_A } as Cue;
		const cells = cellsOfCue(
			[
				{ cue, capture: capture('f1', 'Intensity') },
				{ cue, capture: capture('f1', 'Pan') }
			],
			(c) => String(c.parameter_kind)
		);
		expect([...cells.keys()]).toEqual(['f1/Intensity', 'f1/Pan']);
	});
});
