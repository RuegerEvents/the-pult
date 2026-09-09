import { readFileSync } from 'node:fs';

import { describe, it, expect } from 'vitest';

import {
	cueIdsThrough,
	cueOnlyCompensation,
	insertNumber,
	reorderCueIds,
	trackedThrough
} from './cues.js';
import { parameterKey } from './patch.js';
import type { Cue, ParameterValue, Sequence } from './generated/index.js';

describe('numbering an inserted cue', () => {
	/**
	 * The midpoint, which is what fractional cue numbers have always been for. A list
	 * can be inserted into repeatedly without renumbering everything below it, so a
	 * cue an operator calls "cue 5" stays cue 5.
	 */
	it('lands between the two cues it goes between', () => {
		expect(insertNumber(1, 2)).toBe(1.5);
		expect(insertNumber(1.5, 2)).toBe(1.75);
		expect(insertNumber(1.75, 2)).toBe(1.875);
	});

	it('works on a list that does not start at one', () => {
		expect(insertNumber(10, 20)).toBe(15);
		expect(insertNumber(2.25, 2.5)).toBe(2.375);
	});

	/** Appending to a list that ends at 4.75 should give 5, not 5.75. */
	it('takes the next whole number when there is nothing after it', () => {
		expect(insertNumber(3)).toBe(4);
		expect(insertNumber(4.75)).toBe(5);
		expect(insertNumber(1.001)).toBe(2);
	});
});

describe('dragging a cue somewhere else', () => {
	const ids = ['a', 'b', 'c', 'd'];

	it('moves one down the list', () => {
		expect(reorderCueIds(ids, 0, 2)).toEqual(['b', 'c', 'a', 'd']);
	});

	it('moves one up the list', () => {
		expect(reorderCueIds(ids, 3, 1)).toEqual(['a', 'd', 'b', 'c']);
	});

	it('leaves the list alone when nothing moved', () => {
		expect(reorderCueIds(ids, 2, 2)).toEqual(ids);
	});

	it('clamps a drop past either end rather than losing the cue', () => {
		expect(reorderCueIds(ids, 0, 99)).toEqual(['b', 'c', 'd', 'a']);
		expect(reorderCueIds(ids, 3, -5)).toEqual(['d', 'a', 'b', 'c']);
	});

	it('ignores a drag from outside the list', () => {
		expect(reorderCueIds(ids, 9, 0)).toEqual(ids);
	});

	/**
	 * The ids are the order — `cue_ids` is `ordered` in the schema — so a drag
	 * rewrites that and leaves `Cue.number` alone. Renumbering on every drag would
	 * make a cue stop being the number an operator calls it because somebody moved a
	 * different one.
	 */
	it('never changes how many cues there are', () => {
		for (let from = 0; from < ids.length; from++) {
			for (let to = 0; to < ids.length; to++) {
				expect(reorderCueIds(ids, from, to)).toHaveLength(ids.length);
				expect(new Set(reorderCueIds(ids, from, to))).toEqual(new Set(ids));
			}
		}
	});
});


/**
 * `testdata/tracking.json` is read here and by
 * `crates/pult-schema/tests/tracking_corpus.rs`. "A cue is the stack up to it" is
 * evaluated twice — the station takes cues with it and the browser colours a sheet
 * with it — and this file is what makes the two the same rule rather than two
 * readings of one sentence.
 */
describe('the tracking corpus', () => {
	type Expected = { fixture: string; key: string; cue: string; value: ParameterValue };
	const corpus = JSON.parse(
		readFileSync(new URL('../../../testdata/tracking.json', import.meta.url), 'utf8')
	) as { cues: Cue[]; cases: { name: string; through: string[]; expected: Expected[] }[] };

	const byId = new Map(corpus.cues.map((cue) => [cue.id, cue]));

	for (const scenario of corpus.cases) {
		it(scenario.name, () => {
			const got = trackedThrough(scenario.through, (id) => byId.get(id));
			expect(
				got.map(({ cue, capture }) => ({
					fixture: capture.fixture_id,
					key: parameterKey(capture.parameter_kind),
					cue: cue.id,
					value: capture.value
				}))
			).toEqual(scenario.expected);
		});
	}

	it('is worth reading', () => {
		expect(corpus.cases.length).toBeGreaterThanOrEqual(5);
		expect(corpus.cases.some((c) => c.expected.length === 0)).toBe(true);
	});
});

describe('the cues a sequence lists up to one of them', () => {
	const sequence = { id: 's', name: 'Act one', cue_ids: ['a', 'b', 'c'] } as Sequence;

	it('includes the cue asked about', () => {
		expect(cueIdsThrough(sequence, 'b')).toEqual(['a', 'b']);
	});

	/** A cue that is not in this sequence tracks through nothing rather than through all of it. */
	it('answers nothing for a cue the sequence does not list', () => {
		expect(cueIdsThrough(sequence, 'z')).toEqual([]);
	});
});

describe('cue only', () => {
	const capture = (fixture: string, kind: 'Intensity' | 'Pan', v: number) =>
		({
			fixture_id: fixture,
			parameter_kind: kind,
			value: { type: 'Float', value: v },
			fade_in_ms: 2000,
			fade_out_ms: 900,
			delay_in_ms: 100,
			effect: null,
			easing: 'EaseIn'
		}) as Cue['captures'][number];

	const cue = (captures: Cue['captures']) => ({ id: 'next', captures }) as Cue;

	it('writes what the next cue was tracking into it, so the change stops here', () => {
		const before = new Map([['f1/Intensity', capture('f1', 'Intensity', 0.3)]]);
		const got = cueOnlyCompensation(cue([]), ['f1/Intensity'], before);
		expect(got).toHaveLength(1);
		expect(got?.[0].value).toEqual({ type: 'Float', value: 0.3 });
	});

	/**
	 * The value is going into a *different* cue, so it should move the way that cue
	 * moves. Copying the earlier cue's timing would give one parameter of the next cue
	 * a fade nobody set on it.
	 */
	it('carries the value and none of the timing', () => {
		const before = new Map([['f1/Intensity', capture('f1', 'Intensity', 0.3)]]);
		const got = cueOnlyCompensation(cue([]), ['f1/Intensity'], before);
		expect(got?.[0]).toMatchObject({ fade_in_ms: 0, fade_out_ms: 0, delay_in_ms: 0, easing: null });
	});

	it('leaves a key the next cue already captures alone', () => {
		const own = capture('f1', 'Intensity', 0.9);
		const before = new Map([['f1/Intensity', capture('f1', 'Intensity', 0.3)]]);
		expect(cueOnlyCompensation(cue([own]), ['f1/Intensity'], before)).toBeNull();
	});

	/** Nothing was tracking, so there is nothing to preserve and nothing to write. */
	it('says nothing about a key nothing was tracking', () => {
		expect(cueOnlyCompensation(cue([]), ['f1/Intensity'], new Map())).toBeNull();
	});

	it('appends to what the next cue already says rather than replacing it', () => {
		const own = capture('f2', 'Pan', 0.5);
		const before = new Map([['f1/Intensity', capture('f1', 'Intensity', 0.3)]]);
		const got = cueOnlyCompensation(cue([own]), ['f1/Intensity'], before);
		expect(got).toHaveLength(2);
		expect(got?.[0]).toBe(own);
	});
});
