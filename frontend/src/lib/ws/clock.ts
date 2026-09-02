/**
 * The station's clock, as this browser sees it.
 *
 * Everything the console is doing is anchored in *console milliseconds* — a fade's
 * `t0`, an effect's anchor, a cue's `went_at` — and nothing stores the values those
 * produce. So a browser that wants to draw a moving light has to work it out for
 * itself, and to do that it has to know what time the station thinks it is.
 *
 * Its own clock will not do. A laptop a few seconds out runs every fade a few seconds
 * early or late, and does it *silently*: each individual value is plausible, and the
 * only symptom is the screen disagreeing with the lamps. Which is why the rule here is
 * that a client with no offset yet says so rather than guessing — {@link consoleNow}
 * answers `null`, and a panel shows nothing rather than something wrong.
 *
 * The estimate is the ordinary one, the same shape `PeerLink::rtt_ms` uses between
 * stations: stamp the question, halve the round trip, and keep the sample that had the
 * shortest one. A short round trip is the one least likely to have queued in either
 * direction, so it is the one whose midpoint is closest to the truth.
 *
 * And it is *maintained*, not taken once. Clocks step — NTP corrects a drift, a laptop
 * wakes up — and an offset taken at connect would be wrong from then until a reload.
 */

import { readable, type Readable } from 'svelte/store';

/** How often the offset is re-measured once it is established. */
const REFRESH_MS = 30_000;

/** How often it is asked for while there is no answer yet. */
const RETRY_MS = 1_000;

/**
 * How many samples make up an estimate.
 *
 * The best of a handful, not an average: a mean is dragged by the slow ones, and a
 * slow round trip is exactly the sample whose midpoint is least likely to be the
 * middle of anything.
 */
const SAMPLES = 5;

/** What the browser knows about the station's clock. */
export type ClockOffset = {
	/** Milliseconds to add to `Date.now()` to get console time. */
	offsetMs: number;
	/** The round trip the estimate came from, which is roughly its uncertainty. */
	rttMs: number;
	/** `Date.now()` when it was last measured. */
	measuredAt: number;
};

let current: ClockOffset | null = null;
const listeners = new Set<(offset: ClockOffset | null) => void>();

function publish(next: ClockOffset | null) {
	current = next;
	listeners.forEach((notify) => notify(current));
}

/**
 * The console millisecond it is now, or `null` if this client cannot yet say.
 *
 * Callers must handle the `null`. That is the whole point of it: a number here that
 * has not been placed against the station's clock is a number that will draw the rig
 * in the wrong place, and a visible gap is better than an invisible lie.
 */
export function consoleNow(): number | null {
	return current === null ? null : Date.now() + current.offsetMs;
}

/** The same, for a specific browser instant rather than for now. */
export function consoleAt(localMs: number): number | null {
	return current === null ? null : localMs + current.offsetMs;
}

/** What is known about the offset, or `null` while nothing is. */
export function clockOffset(): ClockOffset | null {
	return current;
}

/** Forget the offset. A reconnect starts again rather than trusting an old one. */
export function forgetOffset(): void {
	publish(null);
}

/** A store of the same, for a panel that has to say whether it can draw yet. */
export const clock: Readable<ClockOffset | null> = readable<ClockOffset | null>(current, (set) => {
	set(current);
	listeners.add(set);
	return () => listeners.delete(set);
});

/** What a clock sync needs from whoever owns the socket. */
export type ClockTransport = {
	/** Send one `ClockSync` carrying this stamp. */
	ask: (sentAt: number) => void;
};

/**
 * One in-flight estimate: the samples taken so far, and the best of them.
 *
 * Held here rather than on the socket because a reconnect throws the socket away and
 * the arithmetic is the same either side of one.
 */
export class ClockSync {
	private samples: ClockOffset[] = [];
	private timer: ReturnType<typeof setTimeout> | null = null;

	constructor(private readonly transport: ClockTransport) {}

	/** Begin, or begin again after a reconnect. */
	start(): void {
		this.samples = [];
		this.schedule(0);
	}

	stop(): void {
		if (this.timer !== null) clearTimeout(this.timer);
		this.timer = null;
	}

	/**
	 * Take one sample from a station's answer.
	 *
	 * `offset = station + rtt/2 - now`: the station's reading, moved forward by half
	 * the round trip to guess where it has got to by the time the answer arrived, then
	 * measured against this browser's own clock.
	 */
	answered(sentAt: number, stationMs: number, receivedAt = Date.now()): void {
		const rttMs = Math.max(0, receivedAt - sentAt);
		const sample: ClockOffset = {
			offsetMs: stationMs + rttMs / 2 - receivedAt,
			rttMs,
			measuredAt: receivedAt,
		};
		this.samples.push(sample);

		const best = this.samples.reduce((a, b) => (b.rttMs < a.rttMs ? b : a));
		// Published on the first sample rather than after all of them: a rough offset
		// now is better than a blank screen for the next five seconds, and the ones
		// after it can only sharpen it.
		publish(best);

		if (this.samples.length >= SAMPLES) {
			this.samples = [best];
			this.schedule(REFRESH_MS);
		} else {
			this.schedule(0);
		}
	}

	private schedule(delayMs: number): void {
		if (this.timer !== null) clearTimeout(this.timer);
		this.timer = setTimeout(
			() => {
				this.timer = null;
				this.transport.ask(Date.now());
				// If the answer never comes, ask again rather than waiting for ever.
				this.timer = setTimeout(() => this.schedule(0), RETRY_MS);
			},
			delayMs
		);
	}
}
