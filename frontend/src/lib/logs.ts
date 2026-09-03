/**
 * The merged log, as a browser holds it.
 *
 * Two streams arrive and have to become one list: the backlog that `log.tail`
 * answers when the panel opens, and the live batches that keep coming on
 * `Update`. They overlap — a line written between the RPC being asked and its
 * answer being read is in both — and they can have holes, because the station's
 * broadcast drops for a listener that fell behind rather than slowing the console
 * down to keep it.
 *
 * `(node_id, seq)` is what makes both tractable. `seq` is monotonic within one run
 * of one station, so a repeat is exactly a `seq` already held, and a hole is
 * exactly a jump — which is worth *saying*, because a log that quietly skips a
 * thousand lines is worse than one that admits it.
 *
 * Ordering is by `at_ms` across stations and by `seq` within one, because two
 * stations' clocks agree only as closely as their skew allows. That is honest
 * rather than exact, and it is the best available until the stations agree a clock.
 * See the station-clock-offset entry in the roadmap.
 *
 * Pure, and tested as such: nothing here touches the socket.
 */

import type { LogLevel, LogLine, LogSource } from './generated/index.js';

/** Quietest first, which is also least verbose first. */
export const LEVELS: LogLevel[] = ['error', 'warn', 'info', 'debug', 'trace'];

/** Does a line at `level` reach a reader who asked for `threshold`? */
export function passes(level: LogLevel, threshold: LogLevel): boolean {
	return LEVELS.indexOf(level) <= LEVELS.indexOf(threshold);
}

/** A line, or a marker standing in for lines that never arrived. */
export type Entry =
	| { kind: 'line'; line: LogLine }
	| { kind: 'gap'; nodeId: string; missing: number; atMs: number; seq: number };

/** What one station's stream has told us so far. */
type Seen = { lowest: number; highest: number; have: Set<number> };

/**
 * A bounded, deduplicated, gap-aware view of everything that has arrived.
 *
 * Kept as a class rather than a store so the whole of it can be exercised without
 * a component: the panel wraps one of these in `$state`.
 */
export class LogBuffer {
	private lines: LogLine[] = [];
	private seen = new Map<string, Seen>();

	constructor(private readonly cap: number = 5000) {}

	/**
	 * Take in lines from either source, and say how many were new.
	 *
	 * Idempotent per `(node_id, seq)`, which is what lets the backlog and the live
	 * stream be merged without either having to know about the other.
	 */
	add(incoming: LogLine[]): number {
		let added = 0;
		for (const line of incoming) {
			const key = line.node_id;
			let seen = this.seen.get(key);
			if (!seen) {
				seen = { lowest: line.seq, highest: line.seq, have: new Set() };
				this.seen.set(key, seen);
			}
			if (seen.have.has(line.seq)) continue;

			seen.have.add(line.seq);
			seen.lowest = Math.min(seen.lowest, line.seq);
			seen.highest = Math.max(seen.highest, line.seq);
			this.lines.push(line);
			added++;
		}
		if (added > 0) this.sort();
		this.trim();
		return added;
	}

	/**
	 * Everything held, in order, with a marker wherever a station's numbering
	 * jumps.
	 *
	 * A gap is only ever *between* two lines held from one station. A run that
	 * starts partway through — which every panel opened mid-show does — is not a
	 * gap, it is a beginning, so nothing is claimed about what came before the
	 * oldest line held.
	 */
	entries(threshold?: LogLevel, sources?: (s: LogSource) => boolean): Entry[] {
		const out: Entry[] = [];
		const previous = new Map<string, LogLine>();

		for (const line of this.lines) {
			const before = previous.get(line.node_id);
			previous.set(line.node_id, line);

			// Measured before filtering, so turning the level down does not invent
			// gaps out of the lines it hid.
			if (before && line.seq > before.seq + 1) {
				out.push({
					kind: 'gap',
					nodeId: line.node_id,
					missing: line.seq - before.seq - 1,
					atMs: line.at_ms,
					seq: line.seq - 1
				});
			}
			if (threshold && !passes(line.level, threshold)) continue;
			if (sources && !sources(line.source)) continue;
			out.push({ kind: 'line', line });
		}
		return out;
	}

	/** Which stations have said anything, for the panel's chips. */
	stations(): string[] {
		return [...this.seen.keys()];
	}

	clear() {
		this.lines = [];
		this.seen.clear();
	}

	get size(): number {
		return this.lines.length;
	}

	/**
	 * By the emitting station's clock, and by `seq` within one station.
	 *
	 * The tie-break matters more than it looks: a station can write several lines
	 * in one millisecond, and `at_ms` alone would let them shuffle on every
	 * insertion, so a panel would reorder lines it had already shown.
	 */
	private sort() {
		this.lines.sort((a, b) =>
			a.at_ms !== b.at_ms
				? a.at_ms - b.at_ms
				: a.node_id !== b.node_id
					? a.node_id.localeCompare(b.node_id)
					: a.seq - b.seq
		);
	}

	/** Drop the oldest, and forget the sequence numbers that went with them. */
	private trim() {
		if (this.lines.length <= this.cap) return;
		const dropped = this.lines.splice(0, this.lines.length - this.cap);
		for (const line of dropped) {
			const seen = this.seen.get(line.node_id);
			// Kept in `have` deliberately: a line dropped for age must not be
			// re-added by a backlog fetch, or scrolling back would resurrect it.
			if (seen) seen.lowest = Math.min(seen.lowest + 1, seen.highest);
		}
	}
}

/**
 * Fold repeated reports into one, so a panel erroring every frame is one line and
 * a count rather than a thousand lines.
 *
 * Used on the way *out*, for what a browser reports about itself: the station's
 * ring is 5,000 lines and a render loop throwing would fill it in a second,
 * pushing out everything that explained why.
 */
export class Throttle {
	private lastAt = new Map<string, number>();
	private suppressed = new Map<string, number>();

	constructor(
		private readonly windowMs: number = 1000,
		private readonly now: () => number = () => Date.now()
	) {}

	/**
	 * Should this be reported, and how many of it does that report stand for?
	 *
	 * `null` means "not yet" — it has been seen, and it will be counted into
	 * whatever the next report of the same signature carries.
	 */
	admit(signature: string): { count: number } | null {
		const at = this.now();
		const last = this.lastAt.get(signature);
		if (last !== undefined && at - last < this.windowMs) {
			this.suppressed.set(signature, (this.suppressed.get(signature) ?? 0) + 1);
			return null;
		}
		this.lastAt.set(signature, at);
		const count = 1 + (this.suppressed.get(signature) ?? 0);
		this.suppressed.delete(signature);
		return { count };
	}
}
