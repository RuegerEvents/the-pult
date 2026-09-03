/**
 * What this browser is costing itself, told to the station.
 *
 * A console is a browser, and since the engine lost its tick a browser showing a rig
 * is *evaluating* that rig in wasm on every animation frame, against a clock it had
 * to estimate. All of which is invisible from anywhere else: a station can say what
 * its own output frames cost, and nothing at all about the tablet at the back of the
 * room that is dropping every third one.
 *
 * Two halves here.
 *
 * {@link FrameMeter} is the measurement, and it is pure — a class rather than a store
 * so the whole of it can be exercised without a browser. It is fed from the one loop
 * that already exists (`stores/output.ts`, which evaluates once per animation frame)
 * rather than from a loop of its own, because a second loop would keep a page
 * rendering purely to prove that it can, which is exactly the wrong thing to do to
 * the tablet this is meant to diagnose. A page drawing nothing therefore measures
 * nothing and says so, rather than reporting a frame rate of zero.
 *
 * {@link reportBrowserStats} is the sending. Every couple of seconds, unconditionally
 * — a browser worth knowing about is precisely the one with nobody in front of it, so
 * reporting cannot be something a panel opts into.
 *
 * And the figures stay with the station serving this page: the `clients` path is
 * LOCAL. What crosses to the other consoles is the *exception* — a window that was
 * {@link struggling} becomes a `warn` in the station's log, which task 48 already
 * carries everywhere. A fault is occasional and a frame rate is every second, and
 * that difference is the whole reason one replicates and the other does not.
 */

import { readable, type Readable } from 'svelte/store';

import { Throttle } from './logs.js';
import { clockOffset } from './ws/clock.js';
import type { PultWsClient } from './ws/client.js';
import type { BrowserFrames } from './generated/index.js';

/** How often a browser describes itself. The same window a station's row uses. */
export const REPORT_INTERVAL_MS = 2000;

/**
 * Below this, motion has stopped reading as motion.
 *
 * A moving light at 20 fps is a light that visibly steps. Fixed rather than measured
 * against the panel's own refresh rate: what matters is whether an operator can
 * believe what they are watching, and that threshold is the same on a 60 Hz laptop
 * and a 120 Hz tablet.
 */
export const SLOW_FPS = 20;

/** A single frame this long is a stall an operator saw, whatever the mean was. */
export const STALL_MS = 100;

/**
 * A window has to have some frames in it before it can be judged.
 *
 * A page that started drawing a fifth of a second before the window closed has a
 * frame rate of five, and it is not in trouble — it has just woken up.
 */
const ENOUGH_FRAMES = 10;

/** One line a minute per reason: a struggling console must not also fill the ring. */
const COMPLAIN_EVERY_MS = 60_000;

/**
 * What the frames of one window cost.
 *
 * Accumulated as sums rather than as a list of frames: a window is a couple of
 * hundred frames and keeping them all to take a mean of them is a hundred allocations
 * a second on the machine that is already struggling.
 */
export class FrameMeter {
	private frames = 0;
	private totalMs = 0;
	private worstMs = 0;
	private evaluatingMs = 0;
	private worstEvaluatingMs = 0;
	private parameters = 0;
	private openedAt: number;

	constructor(private readonly now: () => number = () => performance.now()) {
		this.openedAt = this.now();
	}

	/**
	 * One animation frame: how long since the last one, how much of that was the
	 * evaluator, and how many parameters it was asked for.
	 *
	 * The frame time is the *gap between frames* rather than the work done in one,
	 * because the gap is what an operator sees. A page whose own work takes 2 ms and
	 * which is nonetheless served a frame every 200 ms is a page that is stuttering,
	 * and only the gap says so.
	 */
	frame(frameMs: number, evaluatingMs: number, parameters: number): void {
		this.frames += 1;
		this.totalMs += frameMs;
		this.worstMs = Math.max(this.worstMs, frameMs);
		this.evaluatingMs += evaluatingMs;
		this.worstEvaluatingMs = Math.max(this.worstEvaluatingMs, evaluatingMs);
		this.parameters = parameters;
	}

	/**
	 * Close the window and start the next one.
	 *
	 * `null` when nothing drew, which is a page with no light on it or a tab the
	 * browser has stopped serving frames to. Absent rather than zero, the way an idle
	 * connector carries no `FrameCost` at all: zero would read as instant, and
	 * "nothing happened" is the truth.
	 */
	close(): BrowserFrames | null {
		const at = this.now();
		const windowMs = Math.max(0, at - this.openedAt);
		const frames = this.frames;
		const sample: BrowserFrames | null =
			frames === 0
				? null
				: {
						mean_ms: this.totalMs / frames,
						max_ms: this.worstMs,
						evaluating_mean_ms: this.evaluatingMs / frames,
						evaluating_max_ms: this.worstEvaluatingMs,
						parameters: this.parameters,
						frames,
						window_ms: Math.round(windowMs)
					};
		this.frames = 0;
		this.totalMs = 0;
		this.worstMs = 0;
		this.evaluatingMs = 0;
		this.worstEvaluatingMs = 0;
		// The parameter count is deliberately kept: it describes what the page is
		// showing, not what happened in the window, and a window with no frames in it
		// has not stopped showing it.
		this.openedAt = at;
		return sample;
	}
}

/** Frames per second over a window, read off the pair rather than stored. */
export const fpsOf = (frames: BrowserFrames): number =>
	frames.window_ms === 0 ? 0 : (frames.frames * 1000) / frames.window_ms;

/**
 * Was this window bad enough to be worth telling every console about?
 *
 * Two rules, because they catch different faults. A sustained low frame rate is a
 * page that cannot keep up with the rig it was given. A single long frame is a stall
 * — one 300 ms frame inside a window averaging 17 ms is invisible in the mean and
 * perfectly visible to the operator.
 *
 * Answers the reason in words, because the reason is what goes in the log line.
 */
export function struggling(frames: BrowserFrames | null): string | null {
	if (!frames || frames.frames < ENOUGH_FRAMES) return null;
	const fps = fpsOf(frames);
	if (fps < SLOW_FPS) {
		return `this browser is drawing at ${fps.toFixed(1)} fps over ${frames.parameters} parameters`;
	}
	if (frames.max_ms > STALL_MS) {
		return `this browser stalled for ${frames.max_ms.toFixed(0)} ms in one frame`;
	}
	return null;
}

/** What the browser is, in as many words as it will say. */
function label(): string {
	if (typeof navigator === 'undefined') return 'a browser';
	// `userAgent` is long, spoofed and mostly historical fiction; what is wanted here
	// is enough to tell the booth's Chrome from the tablet's Safari, and no more.
	const ua = navigator.userAgent;
	const engine = /Firefox\/[\d.]+/.exec(ua)?.[0] ?? /Chrome\/[\d.]+/.exec(ua)?.[0] ??
		(/Safari\//.test(ua) ? 'Safari' : 'a browser');
	const platform = /\(([^;)]+)/.exec(ua)?.[1]?.trim();
	return platform ? `${engine} on ${platform}` : engine;
}

/** Chromium offers a heap reading; nothing else does, and nothing is what is sent. */
function heap(): { used: number | null; limit: number | null } {
	const memory = (
		performance as Performance & {
			memory?: { usedJSHeapSize: number; jsHeapSizeLimit: number };
		}
	).memory;
	if (!memory) return { used: null, limit: null };
	return { used: memory.usedJSHeapSize, limit: memory.jsHeapSizeLimit };
}

/**
 * Everything this browser can honestly say about itself, for one window.
 *
 * `session` and `at_ms` are left for the station to fill in — a page cannot be
 * trusted to name its own key, and its clock is the very thing in question.
 */
export function describeSelf(frames: BrowserFrames | null): Record<string, unknown> {
	const offset = clockOffset();
	const { used, limit } = heap();
	return {
		session: '',
		label: label(),
		frames,
		heap_used: used,
		heap_limit: limit,
		// The estimate the page is already evaluating against, not a second one taken
		// here: two estimates of one quantity are two answers to it, and the panel is
		// meant to show what this page is actually using.
		clock_offset_ms: offset?.offsetMs ?? null,
		clock_rtt_ms: offset?.rttMs ?? null,
		at_ms: 0
	};
}

/**
 * Which row in the `clients` map is this browser.
 *
 * A page is not told its session id anywhere else, and must not be able to name one
 * for itself — so the answer comes back from the station on the first report, as the
 * key it landed under. `null` until then, and the panel simply marks nothing.
 */
let mine: string | null = null;
const watchers = new Set<(key: string | null) => void>();
export const thisBrowser: Readable<string | null> = readable<string | null>(null, (set) => {
	set(mine);
	watchers.add(set);
	return () => watchers.delete(set);
});

function iAm(key: string | null) {
	if (key === mine) return;
	mine = key;
	watchers.forEach((notify) => notify(mine));
}

/**
 * Start telling the station what this browser is costing.
 *
 * Returns a function that stops again. Safe where there is no `window`, which is what
 * the static build's prerender pass is.
 */
export function reportBrowserStats(client: PultWsClient, meter: FrameMeter): () => void {
	if (typeof window === 'undefined') return () => {};

	const complaints = new Throttle(COMPLAIN_EVERY_MS);

	const timer = setInterval(() => {
		const frames = meter.close();
		// A failure to report must never itself be reported, or a station that has
		// gone away turns one dropped call into an unbounded stream of them.
		client
			.call('client.report', { stats: describeSelf(frames) })
			.then((key) => iAm(typeof key === 'string' ? key : null))
			.catch(() => {});

		const bad = struggling(frames);
		if (!bad) return;
		// The one thing here that reaches the other consoles. Throttled by *reason*
		// rather than by text, so a page that keeps dropping to 14, 15 and 13 fps is
		// one line a minute rather than three.
		const admitted = complaints.admit(bad.split(' at ')[0]);
		if (!admitted) return;
		client
			.call('log.report', { level: 'warn', message: bad, count: admitted.count })
			.catch(() => {});
	}, REPORT_INTERVAL_MS);

	// A reconnect is a new socket and therefore a new session: the old row is gone
	// from the station's map, and claiming it would mark somebody else's row as this
	// browser's until the next report arrived.
	const stopWatchingConnection = client.addConnectListener(() => iAm(null));

	return () => {
		clearInterval(timer);
		stopWatchingConnection();
	};
}
