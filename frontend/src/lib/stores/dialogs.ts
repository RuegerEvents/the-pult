/**
 * Which dialogs are open, newest last — so something that is not a keyboard can
 * answer one.
 *
 * The command wing has ESC and PLEASE, and an operator at the wing should not have to
 * reach for the mouse to close a dialog or confirm one. `Dialog` registers itself here
 * for as long as it is mounted; the wing asks for the top one.
 */

import { writable } from 'svelte/store';

export type OpenDialog = {
	/** Dismiss it, as its ✕ would. */
	close: () => void;
	/** Accept it, as its primary button would — where it has one. */
	accept?: () => void;
};

const stack: OpenDialog[] = [];

/**
 * How many are open, for the station running the wing: it hands ESC and PLEASE to
 * this page while one is, rather than to the command line behind it.
 */
export const dialogCount = writable(0);

/** A dialog opened. Answers the function that says it closed. */
export function opened(dialog: OpenDialog): () => void {
	stack.push(dialog);
	dialogCount.set(stack.length);
	return () => {
		const at = stack.indexOf(dialog);
		if (at >= 0) stack.splice(at, 1);
		dialogCount.set(stack.length);
	};
}

/** The dialog on top, if any is open. */
export const topDialog = (): OpenDialog | undefined => stack[stack.length - 1];
