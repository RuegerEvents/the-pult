/**
 * A take as it is being recorded, drawn from what this page can see.
 *
 * The station records a pass from the programmer and writes the take when it stops, so
 * until then there was nothing to draw: the lane appeared, whole, at the end. This page
 * already holds every programmer value and the timeline's position, so it keeps a
 * *preview* of the pass by the station's own rules (`model/takes.rs`'s `Pass`): the
 * first look is the baseline and records nothing, a key is recorded from its first
 * change, and a key that leaves the programmer is let go. It is a picture and nothing
 * else — the take that plays is the station's, and replaces this when it is stored.
 */

import type { ParameterValue, ProgrammerValue } from '$lib/generated/index.js';
import { parameterKey } from '$lib/patch.js';

/** One key's movement: positions in timeline milliseconds and a level 0..1, `null`
 *  where it was let go. */
export type LiveKey = { fixtureId: string; key: string; points: [number, number | null][] };

/** A level to draw, the way the station's outline reads a value. */
export function levelOf(value: ParameterValue): number {
	switch (value.type) {
		case 'Float':
		case 'Int':
			return value.value;
		case 'Bool':
			return value.value ? 1 : 0;
		case 'Color':
			return Math.max(value.value.r, value.value.g, value.value.b);
		default:
			return 0;
	}
}

export class LivePass {
	private seen: Map<string, number> | null = null;
	readonly keys = new Map<string, LiveKey>();

	/** Look at the programmer as it is, at a position. */
	observe(positionMs: number, programmer: ProgrammerValue[]): void {
		const now = new Map<string, { fixtureId: string; key: string; level: number }>();
		for (const entry of programmer) {
			if (entry.effect) continue; // a shape has no change points
			const key = parameterKey(entry.parameter_kind);
			now.set(`${entry.fixture_id}/${key}`, { fixtureId: entry.fixture_id, key, level: levelOf(entry.value) });
		}
		const seen = this.seen;
		this.seen = new Map([...now].map(([id, v]) => [id, v.level]));
		if (!seen) return;
		const ms = Math.max(0, Math.round(positionMs));
		for (const [id, v] of now) {
			if (seen.get(id) === v.level) continue;
			this.push(id, v.fixtureId, v.key, ms, v.level);
		}
		for (const id of seen.keys()) {
			if (now.has(id)) continue;
			const live = this.keys.get(id);
			if (live) this.push(id, live.fixtureId, live.key, ms, null);
		}
	}

	private push(id: string, fixtureId: string, key: string, ms: number, level: number | null) {
		const live = this.keys.get(id) ?? { fixtureId, key, points: [] };
		const last = live.points[live.points.length - 1];
		const at = last ? Math.max(ms, last[0]) : ms;
		if (last && last[0] === at) last[1] = level;
		else live.points.push([at, level]);
		this.keys.set(id, live);
	}

	/** Where the pass starts and how far it has got. */
	span(): { from: number; to: number } | null {
		let from = Infinity;
		let to = -Infinity;
		for (const k of this.keys.values()) {
			for (const [ms] of k.points) {
				from = Math.min(from, ms);
				to = Math.max(to, ms);
			}
		}
		return from === Infinity ? null : { from, to };
	}
}
