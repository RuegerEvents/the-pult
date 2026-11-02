/**
 * Whether this page is a stale copy of the console.
 *
 * A station is redeployed and the tablet at the back of the room goes on running the
 * page it loaded yesterday: reconnected, talking to the new station, and drawing it
 * with yesterday's code (found at the gig, 2026-10-25). Every build carries
 * SvelteKit's stamp — `version` from `$app/environment` in the page, the same string
 * in `_app/version.json` beside it — and the station reports the stamp of the build it
 * serves in `/api/config` as `build`. Differing is stale.
 *
 * Pure, so the rule is tested; `stores/build.ts` is the part that reloads.
 */

/**
 * `page` is the stamp this page was built with, `station` what the station serves.
 * Never stale in dev, where Vite serves the page and the station's stamp is of
 * whatever `frontend/build` last held; and never when the station has no stamp (a
 * fresh clone's placeholder) or the page has already reloaded for exactly this one
 * and still came back different — a cache in the way, which a second reload would
 * not get past either, and a loop of reloads is worse than a stale page.
 */
export function isStale(page: string, station: string | null | undefined, dev: boolean, reloadedFor: string | null): boolean {
	if (dev || !station || station === page) return false;
	return reloadedFor !== station;
}
