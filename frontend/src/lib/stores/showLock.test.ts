import { describe, it, expect } from 'vitest';
import { get } from 'svelte/store';

import { canEdit, showLocked, toggleShowLock } from './showLock.js';

describe('the Show lock', () => {
	it('starts unlocked: there is no edit mode to enter', () => {
		showLocked.set(false);
		expect(get(canEdit)).toBe(true);
	});

	it('takes every edit handle away at once', () => {
		showLocked.set(false);
		toggleShowLock();
		expect(get(canEdit)).toBe(false);
		toggleShowLock();
		expect(get(canEdit)).toBe(true);
	});
});
