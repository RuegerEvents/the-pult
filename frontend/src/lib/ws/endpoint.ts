/**
 * Where the console is.
 *
 * The backend serves this page, so the answer is almost always "wherever you got
 * this from" — which is what a `?port=` query string used to have to say, and what
 * a hardcoded `ws://localhost:7700` used to get wrong the moment the console was
 * opened from a tablet rather than from the machine it runs on.
 *
 * The socket URL is therefore worked out from `window.location` before anything is
 * asked of anybody — a console must not wait on a request to make its first one.
 * `/api/config` answers the rest, the things a page cannot work out for itself:
 * which station this is and what version it is running.
 */

/** What `GET /api/config` says. Everything but `wsPath` is for display. */
export type BackendConfig = {
	wsPath: string;
	port: number;
	syncPort: number;
	nodeId: string;
	version: string;
};

/** Where the socket has always been, and the only path this joins to an origin. */
export const WS_PATH = '/ws';

/**
 * Resolution order, most specific first:
 *
 * 1. `?port=NNNN` — a second station on this machine, which is what
 *    `scripts/demo.sh --two` prints and what a dev server pointed at one backend
 *    needs in order to reach another.
 * 2. The origin this page came from — the backend serving its own frontend, or
 *    Vite proxying through to it.
 */
export function backendOrigin(location: { origin: string; search: string }): string {
	const port = new URLSearchParams(location.search).get('port');
	if (port && /^\d+$/.test(port)) {
		const url = new URL(location.origin);
		url.port = port;
		return url.origin;
	}
	return location.origin;
}

/** The console's socket, as seen from this page. */
export function wsUrl(location: { origin: string; search: string }): string {
	return wsUrlFrom(backendOrigin(location), WS_PATH);
}

/** An http(s) origin and a path, as the ws(s) URL they name. */
export function wsUrlFrom(origin: string, path: string): string {
	const url = new URL(path, origin);
	url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
	return url.toString();
}

/**
 * Ask the station what it is. Nothing waits on this: it is the version in the
 * corner and the id in the overlay, not the socket, so a station that does not
 * answer costs a label rather than a console.
 */
export async function fetchConfig(origin: string, fetcher = fetch): Promise<BackendConfig | null> {
	try {
		const response = await fetcher(new URL('/api/config', origin));
		if (!response.ok) return null;
		return (await response.json()) as BackendConfig;
	} catch {
		return null;
	}
}
