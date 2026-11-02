import { describe, expect, it } from 'vitest';

import { fuzzyFilter, fuzzyScore } from './fuzzy.js';

describe('fuzzy matching', () => {
	it('matches in order and refuses what is out of order', () => {
		expect(fuzzyScore('tl', 'Timeline')).not.toBeNull();
		expect(fuzzyScore('lt', 'Tl')).toBeNull();
	});

	it('puts a prefix first, then a word start, then a scatter', () => {
		const titles = ['Stations', 'System log', 'Timeline', 'Settings'];
		expect(fuzzyFilter('s', titles, (t) => t)[0]).toBe('Stations');
		expect(fuzzyFilter('sl', titles, (t) => t)[0]).toBe('System log');
	});

	it('finds the 3D Rig by "rig"', () => {
		expect(fuzzyFilter('rig', ['Rig tools', '3D Rig', 'Programmer'], (t) => t)).toEqual(['Rig tools', '3D Rig']);
	});

	it('keeps everything, in order, for an empty query', () => {
		expect(fuzzyFilter('  ', ['b', 'a'], (t) => t)).toEqual(['b', 'a']);
	});
});
