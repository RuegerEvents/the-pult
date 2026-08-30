/**
 * Which panels this browser has unlocked for editing.
 *
 * Most panels are two things at once: a view of the show and a way to change it. On
 * a laptop that costs nothing, but a console is a tablet gaffer-taped to a truss as
 * often as it is a desk, and a delete button eight pixels from a selector is a
 * fixture unpatched during a show.
 *
 * So the rule is: **view by default, edit on purpose.** A panel that can change the
 * show renders its controls only while it is unlocked, and the toggle lives in the
 * tile chrome rather than inside the panel, so it looks and behaves the same
 * everywhere and cannot be lost among a panel's own buttons.
 *
 * # Why this is not in the schema
 *
 * Whether *this* browser is editing is not a fact about the show. Two operators on
 * one rig want their own answers, and a showfile that reopened with the patch
 * unlocked would have forgotten the whole point. It is not persisted either: a
 * reload is exactly when you want to be locked again.
 */

import { writable, type Writable } from 'svelte/store';

/** One store per panel id, made on first ask and kept for the session. */
const stores = new Map<string, Writable<boolean>>();

/**
 * Whether `panel` is unlocked, as a store a panel can subscribe to.
 *
 * Panels only ever read this. The toggle in the chrome is the one writer, which is
 * what keeps "am I editing" a single answer per panel rather than one per component
 * that happens to care.
 */
export function editing(panel: string): Writable<boolean> {
	let store = stores.get(panel);
	if (!store) {
		store = writable(false);
		stores.set(panel, store);
	}
	return store;
}

/**
 * Lock a panel again.
 *
 * Called when a panel is closed, so that reopening it later starts locked. Without
 * this, closing the Patch panel mid-edit and reopening it an hour later would put
 * an unlocked delete button under the operator's thumb with no warning.
 */
export function stopEditing(panel: string): void {
	stores.get(panel)?.set(false);
}

/** Lock everything. For a "hands off" gesture, and for tests. */
export function stopEditingAll(): void {
	for (const store of stores.values()) store.set(false);
}
