/**
 * What a page shows while the console changes shows.
 *
 * Opening a show is the station stopping and another one starting in its place: this
 * page's socket closes, the page reconnects onto the new console, finds it is looking
 * at a different show and reloads. Left to the ordinary connection handling that is
 * three things in a row — "the console stopped answering", a reload, "connecting to
 * the console" — for one act somebody asked for. So the act is named up front and
 * one screen covers the whole of it.
 *
 * Two ways a page learns a switch is happening, and they meet in one shape:
 *
 * - **It pressed the button.** The menu or the welcome screen begins the switch
 *   before it asks the station, with the name the operator clicked.
 * - **Somebody else did.** The station closes every socket with a close code of its
 *   own and the reason as text — "opening Festival.pult" — which is how the tablet at
 *   the back of the room draws the same screen as the console that pressed Open.
 *
 * The shapes and the rules are pure so they can be tested without a browser; the
 * store beside them in `stores/switching.ts` is what keeps one across a reload.
 */

/** The close code a station uses when it is making way for another. `4001`. */
export const SWITCHING_SHOWS = 4001;

/** A switch in progress: what is being done, and since when. */
export type Switch = {
	/** A lower-case phrase, as the station would say it: "opening Festival.pult". */
	doing: string;
	/** `Date.now()` when it began, so a switch that never ends can be noticed. */
	since: number;
};

/**
 * How long a switch is given before the screen admits it is waiting.
 *
 * A switch is a station stopping and starting: a second or two, and longer for a
 * show with a big rig to seed. Past this the cover stays but says so and offers the
 * retry, because a station that died mid-switch looks exactly like one that is slow
 * until somebody says how long is too long.
 */
export const SWITCH_PATIENCE_MS = 20_000;

/** What the screen says while a switch is under way. */
export function switchTitle(doing: string): string {
	const phrase = doing.trim() || 'changing shows';
	return `${phrase.charAt(0).toUpperCase()}${phrase.slice(1)}…`;
}

/**
 * The switch a close frame describes, or `null` for a close that was not one.
 *
 * Only the station's own code counts. A socket that closed any other way — the
 * process died, the network went, the tab was throttled — is a lost console and
 * must draw as one, because that is the honest screen for a stop nobody asked for.
 */
export function switchFromClose(code: number, reason: string, now: number): Switch | null {
	if (code !== SWITCHING_SHOWS) return null;
	return { doing: reason.trim() || 'changing shows', since: now };
}

/** Whether a switch has gone on long enough that the screen should say so. */
export function overdue(current: Switch, now: number): boolean {
	return now - current.since > SWITCH_PATIENCE_MS;
}

/** Whether a switch read back out of storage is worth believing. */
export function plausible(value: unknown, now: number): value is Switch {
	if (!value || typeof value !== 'object') return false;
	const { doing, since } = value as Partial<Switch>;
	if (typeof doing !== 'string' || typeof since !== 'number') return false;
	// One that began in the future, or an hour ago, is a stale entry from some
	// earlier session and not a switch this page is waiting on.
	return since <= now && now - since < 60 * 60 * 1000;
}
