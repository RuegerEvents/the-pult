/**
 * A short history of a figure, kept by whoever is looking at it.
 *
 * Everything the System panel shows is *one closed window*: a station's row carries
 * what its connectors' frames cost over the couple of seconds just ended, a browser's
 * report the same about its own. Nothing anywhere keeps a series — which is right, and
 * is the same rule as everywhere else in this console: a station's row is a
 * measurement taken at a moment, and a growing series on a replicated row is the one
 * thing that entry said a row genuinely cannot hold.
 *
 * So the history is the *reader's*. A panel receives every report as it lands anyway,
 * and keeping the last couple of minutes of what it saw costs a few dozen numbers and
 * nothing on the wire. The honest limitation, which the panel says out loud: a trace
 * starts empty when the tile is opened and covers only what that tile witnessed. It is
 * a sparkline, not a record.
 *
 * Pure, and tested as such — the panel wraps these in `$state`.
 */

/** How many windows a trace holds. Sixty at a two-second report is two minutes. */
export const TRACE_LENGTH = 60;

/**
 * The last few readings of one figure, oldest first.
 *
 * Deduplicated by the *stamp* rather than by the value, because a report that has not
 * been renewed is not a new reading: a station going quiet would otherwise draw a flat
 * line of its last figure repeated, which reads as a machine steadily doing something.
 */
export class Trace {
	private values: number[] = [];
	private lastStamp: number | null = null;

	constructor(private readonly length: number = TRACE_LENGTH) {}

	/**
	 * Take one reading, if it is one.
	 *
	 * `stamp` identifies the window the value came from — a station's `last_seen`, a
	 * browser's `at_ms`. The same stamp twice is the same report seen twice, which a
	 * panel re-rendering for an unrelated reason will do.
	 */
	push(value: number, stamp: number): void {
		if (this.lastStamp === stamp) return;
		this.lastStamp = stamp;
		this.values.push(value);
		if (this.values.length > this.length) this.values.shift();
	}

	/** What is held, oldest first. */
	get points(): number[] {
		return this.values;
	}

	get length_(): number {
		return this.values.length;
	}
}

/**
 * A set of traces, one per thing being watched, that forgets what stops reporting.
 *
 * Keyed by whatever names the figure — `"<station id>/<connector>"`, a browser's
 * session. `keep` is called with the keys that still exist so a connector removed from
 * the show, or a tab that closed, does not leave a line behind for ever.
 */
export class Traces {
	private held = new Map<string, Trace>();

	constructor(private readonly length: number = TRACE_LENGTH) {}

	push(key: string, value: number, stamp: number): number[] {
		let trace = this.held.get(key);
		if (!trace) {
			trace = new Trace(this.length);
			this.held.set(key, trace);
		}
		trace.push(value, stamp);
		return trace.points;
	}

	points(key: string): number[] {
		return this.held.get(key)?.points ?? [];
	}

	/** Drop everything not in this set. */
	keep(keys: Iterable<string>): void {
		const alive = new Set(keys);
		for (const key of [...this.held.keys()]) {
			if (!alive.has(key)) this.held.delete(key);
		}
	}
}

/**
 * A sparkline's path, as an SVG `d`, over a box `width` by `height`.
 *
 * Scaled from zero to the highest point rather than from the lowest, because these are
 * costs and rates: a frame time bouncing between 4.0 and 4.1 ms should read as flat,
 * and a floor-to-ceiling scale would draw it as an alarming sawtooth. `ceiling` pins
 * the top where a figure has a meaningful one — a frame rate against the rate it ought
 * to be — so two browsers' lines can be compared with each other.
 *
 * Fewer than two points is no line at all: one reading is not a trend, and drawing a
 * dot at it invites reading one.
 */
export function sparkline(
	points: number[],
	width: number,
	height: number,
	ceiling?: number
): string | null {
	if (points.length < 2) return null;
	// A given ceiling *is* the top, rather than a floor under one: the point of it is
	// that two lines share a scale, which a reading raising the top would defeat.
	// Without one the tallest reading is the top, so a cost with no natural budget
	// still fills the box.
	const top = ceiling ?? Math.max(...points, Number.MIN_VALUE);
	const step = width / (points.length - 1);
	return points
		.map((value, index) => {
			const x = index * step;
			// Clamped, so a browser briefly beating the ceiling — a 120 Hz panel against
			// a 60 fps scale — draws along the top rather than out of the box.
			const y = height - Math.min(1, value / top) * height;
			return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
		})
		.join(' ');
}
