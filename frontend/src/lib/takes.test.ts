import { describe, expect, it } from 'vitest';

import type { InputConfig, TimelineTrack } from './generated/index.js';
import { armArgs, armChoice, PROGRAMMER, releases, takeSummary, trackSource } from './takes.js';

describe('arming', () => {
	it('reads what the timeline is armed with into a picker value', () => {
		expect(armChoice(null)).toBe('');
		expect(armChoice({ Input: 'abc' })).toBe('abc');
		expect(armChoice({ Programmer: { release: false } })).toBe(PROGRAMMER);
	});

	it('gives the faders back unless told otherwise', () => {
		expect(releases(null)).toBe(true);
		expect(releases({ Programmer: { release: false } })).toBe(false);
	});

	it('turns a picker value into the record command', () => {
		expect(armArgs('', true)).toEqual({ inputId: null });
		expect(armArgs('abc', true)).toEqual({ inputId: 'abc' });
		expect(armArgs(PROGRAMMER, false)).toEqual({ programmer: true, release: false });
	});
});

describe('what is being recorded', () => {
	it('counts keys, points and the laps already stored', () => {
		const source = { Programmer: { release: true } };
		expect(takeSummary({ source, keys: 1, points: 1, passes: 0 })).toBe(
			'Recording 1 key · 1 change point'
		);
		expect(takeSummary({ source, keys: 3, points: 1200, passes: 2 })).toBe(
			'Recording 3 keys · 1,200 change points · 2 passes stored'
		);
	});

	it('names where a take came from', () => {
		const track = (input_id: string | null): TimelineTrack => ({
			id: 't',
			name: 'Take 1',
			asset: 'sha',
			input_id,
			offset_ms: 0,
			enabled: true
		});
		const inputs = [{ id: 'i', name: 'Guest' } as InputConfig];
		expect(trackSource(track(null), inputs)).toBe('programmer');
		expect(trackSource(track('i'), inputs)).toBe('Guest');
		expect(trackSource(track('gone'), inputs)).toBe('an input');
	});
});
