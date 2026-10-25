/**
 * Which dialogs are open, newest last — so something that is not a keyboard can
 * answer one.
 *
 * The command wing has ESC and PLEASE, and an operator at the wing should not have to
 * reach for the mouse to close a dialog or confirm one. `Dialog` registers itself here
 * for as long as it is mounted; the wing asks for the top one.
 */

export type OpenDialog = {
	/** Dismiss it, as its ✕ would. */
	close: () => void;
	/** Accept it, as its primary button would — where it has one. */
	accept?: () => void;
};

const stack: OpenDialog[] = [];

/** A dialog opened. Answers the function that says it closed. */
export function opened(dialog: OpenDialog): () => void {
	stack.push(dialog);
	return () => {
		const at = stack.indexOf(dialog);
		if (at >= 0) stack.splice(at, 1);
	};
}

/** The dialog on top, if any is open. */
export const topDialog = (): OpenDialog | undefined => stack[stack.length - 1];
