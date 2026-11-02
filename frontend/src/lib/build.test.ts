import { describe, expect, it } from 'vitest';

import { isStale } from './build.js';

describe('a page built before the station it is talking to', () => {
	it('is stale when the station serves another build', () => {
		expect(isStale('100', '200', false, null)).toBe(true);
		expect(isStale('200', '200', false, null)).toBe(false);
	});

	it('is never stale in dev, or against a station with no stamp', () => {
		expect(isStale('100', '200', true, null)).toBe(false);
		expect(isStale('100', null, false, null)).toBe(false);
		expect(isStale('100', undefined, false, null)).toBe(false);
	});

	it('does not reload twice for the same build', () => {
		expect(isStale('100', '200', false, '200')).toBe(false);
		expect(isStale('100', '300', false, '200')).toBe(true);
	});
});
