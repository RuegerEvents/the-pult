/**
 * What went wrong in this browser, told to the station.
 *
 * A console is a browser by design, so an exception inside a panel is invisible
 * twice over: the operator sees a tile that stopped updating, and the station sees
 * nothing at all. The tablet at the back of the room is the console nobody is
 * watching, and "the tablet's rig panel is throwing" being readable from the booth
 * is the whole point.
 *
 * A browser line is a line like any other once it reaches the station: it goes in
 * the ring, in the file, and out to peers on the same threshold, tagged with the
 * short form of this socket's session. Which means one bad panel could otherwise
 * fill a 5,000-line ring in a second and push out everything that explained why —
 * hence {@link Throttle}, which sends the first of a burst and folds the rest into
 * a count on the next one.
 *
 * Deliberately not a toast. An operator mid-show does not need a dialog about a
 * stack trace; somebody looking at the log later does.
 */

import { Throttle } from './logs.js';
import type { PultWsClient } from './ws/client.js';

/** Long enough that a per-frame fault is one line a second, not sixty. */
const WINDOW_MS = 1000;

/** Trimmed before sending: a stack trace is not a log line. */
const MAX_MESSAGE = 2000;

function describe(error: unknown, fallback: string): string {
	if (error instanceof Error) {
		// The first frame or two of the stack is what says *where*, and the rest is
		// framework noise that would push the next line out of the ring.
		const where = error.stack?.split('\n').slice(1, 3).join(' ').trim();
		return where ? `${error.name}: ${error.message} — ${where}` : `${error.name}: ${error.message}`;
	}
	if (typeof error === 'string' && error) return error;
	return fallback;
}

/**
 * Start reporting this browser's own faults into the station's log.
 *
 * Returns a function that stops again. Safe to call where there is no `window`,
 * which is what the static build's prerender pass is.
 */
export function reportBrowserErrors(client: PultWsClient): () => void {
	if (typeof window === 'undefined') return () => {};

	const throttle = new Throttle(WINDOW_MS);

	const report = (message: string) => {
		const trimmed = message.slice(0, MAX_MESSAGE);
		const admitted = throttle.admit(trimmed);
		if (!admitted) return;
		// Failing to report a fault must never itself raise one, or a station that
		// has gone away turns one exception into an unbounded loop of them.
		client.call('log.report', { level: 'error', message: trimmed, count: admitted.count }).catch(
			() => {}
		);
	};

	const onError = (event: ErrorEvent) => {
		report(describe(event.error, event.message || 'an error with nothing said about it'));
	};
	const onRejection = (event: PromiseRejectionEvent) => {
		report(`unhandled rejection: ${describe(event.reason, String(event.reason))}`);
	};

	window.addEventListener('error', onError);
	window.addEventListener('unhandledrejection', onRejection);

	return () => {
		window.removeEventListener('error', onError);
		window.removeEventListener('unhandledrejection', onRejection);
	};
}
