/**
 * What the station this page is talking to says it is.
 *
 * `/api/config` used to be asked once, at start-up, for the version in the corner.
 * It answers a second thing now — which show is open — and that can change while the
 * page is up, because opening a show is the station stopping and another one
 * starting in its place. So it is asked again on every reconnect, which is exactly
 * when the answer might have moved: a switch closes this tab's socket, and the
 * reconnect is the first moment there is anybody to ask.
 *
 * And when it *has* moved, this tab reloads. Every store in it is holding the
 * previous show's rig, and there is no honest way to keep any of it — see
 * `shouldReload` in `$lib/shows.ts`, which is where the rule lives and is tested.
 * The tablet at the back of the room, on somebody else's socket, sees the same
 * thing for the same reason.
 */

import { readable, type Readable } from 'svelte/store';

import { shouldReload } from '$lib/shows.js';
import type { PultWsClient } from '$lib/ws/client.js';
import { backendOrigin, fetchConfig, type BackendConfig } from '$lib/ws/endpoint.js';

/** What this page loaded onto, and what it is looking at now. */
export type StationStore = Readable<BackendConfig | null>;

/**
 * Watch the station.
 *
 * `reload` is injected so the rule can be exercised without a browser; in the app it
 * is `location.reload`.
 */
export function watchStation(
	client: PultWsClient,
	origin: string,
	reload: () => void = () => location.reload(),
	/**
	 * Called each time the station has answered and this tab is staying put — which
	 * is the moment a switch is over. Not called when the answer is a reload: the
	 * page that comes back asks again and hears it then.
	 */
	onFresh: () => void = () => {}
): StationStore {
	return readable<BackendConfig | null>(null, (set) => {
		// What this page believes it loaded onto. Set once, from the first answer:
		// a page with nothing to compare against must not reload, or it would loop.
		let loadedWith: BackendConfig | null = null;

		const ask = async () => {
			const config = await fetchConfig(origin);
			if (!config) return;
			if (shouldReload(loadedWith, config)) {
				reload();
				return;
			}
			loadedWith ??= config;
			set(config);
			onFresh();
		};

		void ask();
		// Every reconnect, because a switch is what closes the socket and coming back
		// is the first moment there is anything to ask.
		return client.addConnectListener(() => void ask());
	});
}
