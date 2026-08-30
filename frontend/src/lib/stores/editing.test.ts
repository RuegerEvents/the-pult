import { describe, it, expect } from 'vitest';
import { get } from 'svelte/store';

import { editing, stopEditing, stopEditingAll } from './editing.js';

describe('unlocking a panel for editing', () => {
	it('starts locked, because a mis-hit is more likely than a deliberate edit', () => {
		expect(get(editing('a-panel-nobody-has-touched'))).toBe(false);
	});

	/**
	 * The toggle in the chrome and the panel reading the store have to be looking at
	 * the same answer. If `editing()` minted a fresh store per caller, the toggle
	 * would flip its own copy and the panel would never hear about it.
	 */
	it('gives every asker the same store for one panel', () => {
		const fromToggle = editing('patch');
		const fromPanel = editing('patch');

		fromToggle.set(true);
		expect(get(fromPanel)).toBe(true);
	});

	it('keeps panels apart', () => {
		editing('patch').set(true);
		expect(get(editing('devices'))).toBe(false);
	});

	/**
	 * Closing a panel locks it, so reopening it an hour later does not put an
	 * unlocked delete button under a thumb with nothing having said so.
	 */
	it('locks a panel again when it is closed', () => {
		editing('flows').set(true);
		stopEditing('flows');
		expect(get(editing('flows'))).toBe(false);
	});

	it('locking a panel nobody has opened is not an error', () => {
		expect(() => stopEditing('never-opened')).not.toThrow();
	});

	it('can lock everything at once', () => {
		editing('patch').set(true);
		editing('devices').set(true);

		stopEditingAll();

		expect(get(editing('patch'))).toBe(false);
		expect(get(editing('devices'))).toBe(false);
	});
});
