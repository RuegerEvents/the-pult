/**
 * One thing an operator did, however many writes it took.
 *
 * A fader dragged across its travel is a few hundred pointer events, and across a
 * selection of twenty it is a few thousand writes. It is still one act, and Ctrl-Z
 * should take back the act rather than the last frame of it.
 *
 * Only the client knows where an act begins and ends — the backend sees a stream of
 * writes and no amount of guessing at the gaps between them would tell a drag from
 * two quick edits. So this says: everything written between `beginGesture` and
 * `endGesture` carries one id, and undo reverses them together.
 *
 * Nothing here is a Svelte store. There is no state anybody renders — a gesture is
 * something the socket is told about, not something the screen shows.
 */

import { showClient } from './show.js';

/**
 * How long a gesture stays open after the pointer comes up.
 *
 * Not zero, because a gesture does not end when the pointer does. The programmer
 * stages a move and writes it on the next frame, and a selection of twenty is twenty
 * round trips after that — so closing on the spot would leave a drag's own tail
 * outside it, and one drag would want three presses to take back.
 *
 * Closing late costs nothing: the only thing a stale id could spoil is the *next*
 * gesture, and beginning one replaces the id outright.
 */
const TAIL_MS = 400;

let current: string | null = null;
/** Bumped by every begin and end, so a close only fires if nothing has happened
 *  since it was scheduled. */
let generation = 0;
let closing: ReturnType<typeof setTimeout> | null = null;

/** Everything written from now on is one act. */
export function beginGesture(): void {
	generation += 1;
	if (closing) clearTimeout(closing);
	closing = null;
	current = crypto.randomUUID();
	showClient().duringGesture(current);
}

/** The act is over — once its last writes have gone out. */
export function endGesture(): void {
	if (!current) return;
	const mine = ++generation;
	if (closing) clearTimeout(closing);
	closing = setTimeout(() => {
		if (generation !== mine) return;
		current = null;
		closing = null;
		showClient().duringGesture(null);
	}, TAIL_MS);
}

/**
 * Keep an act open, starting one if there is none.
 *
 * For a control that fires repeatedly with nothing to say when it stopped — an
 * arrow key held down on a fader, which is a drag by another name. Each step pushes
 * the close back, so a run of them is one gesture and a pause ends it.
 */
export function nudging(): void {
	if (!current) beginGesture();
	endGesture();
}

/**
 * A pointer drag, as an action: `<div use:asOneGesture>`.
 *
 * On the element that captures the pointer, so the gesture lasts exactly as long as
 * the drag does — including when the pointer leaves the element, which is most
 * drags, and when it is cancelled by a phone call arriving, which is the one people
 * forget.
 */
export function asOneGesture(node: Element) {
	const down = () => beginGesture();
	const up = () => endGesture();
	node.addEventListener('pointerdown', down);
	node.addEventListener('pointerup', up);
	node.addEventListener('pointercancel', up);
	return {
		destroy() {
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointerup', up);
			node.removeEventListener('pointercancel', up);
		}
	};
}

/** For the tests, and for nothing else. */
export function currentGesture(): string | null {
	return current;
}
