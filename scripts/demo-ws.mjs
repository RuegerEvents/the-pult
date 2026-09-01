// The little WebSocket client the demo scripts talk to a backend with.
//
// Nothing here is privileged — it is the same protocol the frontend speaks, so if
// this drifts out of date it will fail loudly rather than quietly doing the wrong
// thing. That property is worth more the bigger the show being seeded: a two
// thousand fixture rig is the largest exercise of the write path anything in this
// repo performs, and it goes through the same door a browser does.

/**
 * Open a socket to a station.
 *
 * `timeoutMs` is how long one request may wait. The default suits a handful of
 * writes; a large seed raises it, because with many writes in flight at once a
 * request can sit in the engine's queue for a while through no fault of its own,
 * and a timeout firing on *queueing* would report a broken station where there is
 * only a busy one.
 */
export function connect(port, { timeoutMs = 5000 } = {}) {
	const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
	const pending = new Map();
	let counter = 0;

	ws.onmessage = (event) => {
		const { payload } = JSON.parse(event.data);
		const waiting = payload?.request_id && pending.get(payload.request_id);
		if (!waiting) return;
		pending.delete(payload.request_id);
		clearTimeout(waiting.timer);
		if (payload.error) waiting.reject(new Error(payload.error));
		else waiting.resolve(payload.value ?? payload.result ?? null);
	};

	function request(type, extra) {
		const request_id = String(++counter);
		return new Promise((resolve, reject) => {
			// Cleared when the answer arrives. With thousands of writes in a seed,
			// timers that outlive their request keep the process alive after the work
			// is done and the script looks hung when it is finished.
			const timer = setTimeout(() => {
				pending.delete(request_id);
				reject(new Error(`${type} timed out after ${timeoutMs}ms`));
			}, timeoutMs);
			pending.set(request_id, { resolve, reject, timer });
			ws.send(JSON.stringify({ type, payload: { request_id, ...extra } }));
		});
	}

	// PathSegment is untagged on the wire: a path is plain strings, and anything
	// that parses as a uuid is read as an entity id.
	return {
		open: new Promise((resolve, reject) => {
			ws.onopen = () => resolve();
			ws.onerror = () => reject(new Error(`could not reach the backend on ${port}`));
		}),
		close: () => ws.close(),
		get: (path) => request('Get', { path }),
		set: (path, value) => request('Set', { path, value }),
		create: (table, entity) => request('Set', { path: [table, '__create'], value: entity }),
		call: (method, args = {}) => request('Call', { method, args })
	};
}

/**
 * How many writes to keep in flight at once.
 *
 * Bounded rather than "all of them". The engine is one actor behind a channel 256
 * deep, so firing two thousand writes at once does not go faster — it fills the
 * channel, and the backpressure then arrives as a per-request timeout on writes
 * that were only ever waiting their turn. This is comfortably under that, and it is
 * still a hundredfold improvement on awaiting one round trip at a time.
 */
export const WINDOW = 64;

/**
 * Run `work` over `items`, keeping at most `limit` of them in flight.
 *
 * Results come back in the order the items were given, whatever order they finish
 * in — a seed reads ids back out of what it created, so that matters.
 */
export async function inWindow(items, work, { limit = WINDOW, onProgress } = {}) {
	const list = [...items];
	const results = new Array(list.length);
	let next = 0;
	let done = 0;

	const worker = async () => {
		while (next < list.length) {
			const index = next++;
			results[index] = await work(list[index], index);
			done += 1;
			if (onProgress) onProgress(done, list.length);
		}
	};

	await Promise.all(Array.from({ length: Math.min(limit, list.length) }, worker));
	return results;
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
