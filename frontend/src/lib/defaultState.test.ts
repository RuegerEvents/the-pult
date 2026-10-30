import { describe, expect, it } from 'vitest';

import type { Cue, Sequence, Timeline } from '$lib/generated/index.js';
import { describeDefaultState } from './defaultState.js';

const sequence = (id: string, name: string) => ({ id, name }) as Sequence;
const cue = (id: string, number: number, name = '') => ({ id, number, name }) as Cue;
const timeline = (id: string, name: string) => ({ id, name }) as Timeline;

describe('a default state in words', () => {
	it('names each sequence, its cue and a master not at full, then the timelines', () => {
		const lines = describeDefaultState(
			{
				sequences: [
					{ sequence_id: 's1', cue_id: 'c3', master: 1 },
					{ sequence_id: 's2', cue_id: 'c1', master: 0.4 }
				],
				timelines: ['t1'],
				saved_at: 0
			},
			[sequence('s1', 'Ambient'), sequence('s2', 'Arbeitslicht')],
			[cue('c3', 3, 'Night'), cue('c1', 1.5)],
			[timeline('t1', 'Pumpkin Room Loop')]
		);
		expect(lines).toEqual([
			'Ambient: at cue 3 “Night”',
			'Arbeitslicht: at cue 1.5, master 40%',
			'Pumpkin Room Loop: plays from the start'
		]);
	});

	it('says a sequence held only for its master is off', () => {
		const lines = describeDefaultState(
			{ sequences: [{ sequence_id: 's1', cue_id: null, master: 0 }], timelines: [], saved_at: 0 },
			[sequence('s1', 'Strobe')],
			[],
			[]
		);
		expect(lines).toEqual(['Strobe: off, master 0%']);
	});

	it('says what has gone rather than dropping it', () => {
		const lines = describeDefaultState(
			{ sequences: [{ sequence_id: 'x', cue_id: 'y', master: null }], timelines: ['z'], saved_at: 0 },
			[],
			[],
			[]
		);
		expect(lines).toEqual([
			'a sequence that is gone: at a cue that is gone',
			'a timeline that is gone: plays from the start'
		]);
	});
});
