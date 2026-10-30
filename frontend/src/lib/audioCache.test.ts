import { describe, expect, it } from 'vitest';

import { cacheWarning } from './audioCache.js';
import type { AudioCacheStats } from './generated/index.js';

const MB = 1_000_000;
const size = (bytes: number) => `${Math.round(bytes / MB)} MB`;

function stats(over: Partial<AudioCacheStats>): AudioCacheStats {
	return {
		budget: 1000 * MB,
		held: 0,
		files: 0,
		timeline_bytes: 0,
		timeline_files: 0,
		timeline_decoded: 0,
		mapped: 0,
		mapped_files: 0,
		...over
	};
}

describe('cacheWarning', () => {
	it('says nothing for a station that cannot say', () => {
		expect(cacheWarning(stats({ budget: 0, held: 5000 * MB, timeline_bytes: 5000 * MB }), size)).toBeNull();
	});

	it('says nothing with room to spare', () => {
		expect(cacheWarning(stats({ held: 500 * MB, timeline_bytes: 500 * MB }), size)).toBeNull();
	});

	it('is close past four fifths of the budget', () => {
		expect(cacheWarning(stats({ held: 850 * MB, timeline_bytes: 300 * MB }), size)?.level).toBe('close');
	});

	it('is over when the timelines alone do not fit, however the rest stands', () => {
		const warning = cacheWarning(stats({ held: 1500 * MB, timeline_bytes: 1500 * MB }), size);
		expect(warning?.level).toBe('over');
		expect(warning?.text).toContain('1500 MB');
	});
});
