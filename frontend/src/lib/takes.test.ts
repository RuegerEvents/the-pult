import { describe, expect, it } from 'vitest';

import type { InputConfig, TimelineTrack } from './generated/index.js';
import {
	armArgs,
	armChoice,
	cropped,
	PROGRAMMER,
	releases,
	rowKeys,
	rowSpan,
	slid,
	splitRows,
	takeSummary,
	trackSource
} from './takes.js';

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
			enabled: true,
			fixture_id: null,
			key: null,
			from_ms: null,
			to_ms: null
		});
		const inputs = [{ id: 'i', name: 'Guest' } as InputConfig];
		expect(trackSource(track(null), inputs)).toBe('programmer');
		expect(trackSource(track('i'), inputs)).toBe('Guest');
		expect(trackSource(track('gone'), inputs)).toBe('an input');
	});
});

describe('a take is a row per parameter', () => {
	const row = (over: Partial<TimelineTrack> = {}): TimelineTrack => ({
		id: 'a',
		name: '',
		asset: 'sha',
		input_id: null,
		offset_ms: 0,
		enabled: true,
		fixture_id: 'f',
		key: 'Jaw',
		from_ms: null,
		to_ms: null,
		...over
	});
	const outline = [
		{ fixture_id: 'f', key: 'Jaw', from_ms: 100, to_ms: 900, points: 3, levels: [] },
		{ fixture_id: 'f', key: 'Head', from_ms: 0, to_ms: 2_000, points: 2, levels: [] }
	];

	it('is where its window is, or where its own parameter moved', () => {
		expect(rowSpan(row(), rowKeys(row(), outline))).toEqual({ fromMs: 100, toMs: 900 });
		expect(rowSpan(row({ from_ms: 300 }), rowKeys(row(), outline))).toEqual({ fromMs: 300, toMs: 900 });
		expect(rowSpan(row({ key: null, fixture_id: null }), outline)).toEqual({ fromMs: 0, toMs: 2_000 });
	});

	it('slices in two at a position inside it, and nowhere else', () => {
		const span = () => ({ fromMs: 100, toMs: 900 });
		const split = splitRows([row()], new Set(['a']), 500, span, () => 'b')!;
		expect(split.map((t) => [t.id, t.from_ms, t.to_ms])).toEqual([
			['a', 100, 500],
			['b', 500, 900]
		]);
		expect(splitRows([row()], new Set(['a']), 950, span, () => 'b')).toBeNull();
	});

	it('crops an end, never past the other', () => {
		const span = { fromMs: 100, toMs: 900 };
		expect(cropped(row(), 'front', 400, span)).toMatchObject({ from_ms: 400, to_ms: 900 });
		expect(cropped(row(), 'back', 50, span)).toMatchObject({ from_ms: 100, to_ms: 101 });
	});

	it('takes its window with it when it is moved', () => {
		expect(slid(row({ from_ms: 100, to_ms: 500 }), 250)).toMatchObject({ offset_ms: 250, from_ms: 350, to_ms: 750 });
	});
});
