import { describe, it, expect } from 'vitest';

import { isTextField, shortcutFor } from './undo.js';

const press = (key: string, mods: Partial<KeyboardEvent> = {}) => ({
	key,
	ctrlKey: false,
	metaKey: false,
	shiftKey: false,
	...mods
});

describe('what a keystroke means', () => {
	it('undoes on ctrl-z and cmd-z', () => {
		expect(shortcutFor(press('z', { ctrlKey: true }), false)).toBe('undo');
		expect(shortcutFor(press('z', { metaKey: true }), false)).toBe('undo');
	});

	/** Both spellings, because both are already in people's hands. */
	it('redoes on ctrl-shift-z and on ctrl-y', () => {
		expect(shortcutFor(press('z', { ctrlKey: true, shiftKey: true }), false)).toBe('redo');
		expect(shortcutFor(press('y', { ctrlKey: true }), false)).toBe('redo');
	});

	it('takes the key however it is capitalised', () => {
		expect(shortcutFor(press('Z', { metaKey: true }), false)).toBe('undo');
	});

	it('ignores a bare keypress', () => {
		expect(shortcutFor(press('z'), false)).toBeNull();
		expect(shortcutFor(press('a', { ctrlKey: true }), false)).toBeNull();
	});

	/**
	 * Somebody halfway through typing a cue name means the browser's own undo.
	 * Taking their fixture back instead would be a nasty surprise.
	 */
	it('leaves a text field alone', () => {
		expect(shortcutFor(press('z', { ctrlKey: true }), true)).toBeNull();
		expect(shortcutFor(press('y', { ctrlKey: true }), true)).toBeNull();
	});
});

describe('spotting somewhere a person is typing', () => {
	const el = (tagName: string, contentEditable = false) =>
		({ tagName, isContentEditable: contentEditable }) as unknown as EventTarget;

	it('knows the fields', () => {
		expect(isTextField(el('INPUT'))).toBe(true);
		expect(isTextField(el('TEXTAREA'))).toBe(true);
		expect(isTextField(el('SELECT'))).toBe(true);
		expect(isTextField(el('DIV', true))).toBe(true);
	});

	it('and knows what is not one', () => {
		expect(isTextField(el('DIV'))).toBe(false);
		expect(isTextField(el('BUTTON'))).toBe(false);
		expect(isTextField(null)).toBe(false);
	});
});
