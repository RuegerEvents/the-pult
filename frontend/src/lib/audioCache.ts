/**
 * Is a station's decoded sound about to cost it, or already costing it?
 *
 * The station keeps every file on a timeline decoded whatever it comes to, and only
 * the rest under its budget (`infra/audio/decode.rs`), because a file it had to let go
 * of was decoded again on the next look at the timelines — a core, and every Play and
 * Stop waiting behind it. So "over" is not the cache failing: it is the machine paying
 * for a show whose sound is bigger than what was set aside for it, and worth knowing
 * before the machine runs short of memory.
 *
 * The panel calls this rather than restating it, the way `stats.ts`'s `struggling()`
 * is called.
 */

import type { AudioCacheStats } from './generated/index.js';

/** Held past this share of the budget is close. */
export const CLOSE_SHARE = 0.8;

export type CacheWarning = { level: 'over' | 'close'; text: string };

export function cacheWarning(stats: AudioCacheStats, size: (bytes: number) => string): CacheWarning | null {
	if (stats.budget === 0) return null;
	if (stats.timeline_bytes > stats.budget) {
		return {
			level: 'over',
			text: `The timelines' sound is ${size(stats.timeline_bytes)} decoded, over the ${size(stats.budget)} this station sets aside. It is kept anyway, out of the machine's memory.`
		};
	}
	if (stats.held > stats.budget * CLOSE_SHARE) {
		return {
			level: 'close',
			text: `${size(stats.held)} of ${size(stats.budget)} in use. Past it, files no timeline holds are let go.`
		};
	}
	return null;
}
