// The little WebSocket client the demo scripts talk to a backend with.
//
// Nothing here is privileged — it is the same protocol the frontend speaks, so if
// this drifts out of date it will fail loudly rather than quietly doing the wrong
// thing.

export function connect(port) {
	const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
	const pending = new Map();
	let counter = 0;

	ws.onmessage = (event) => {
		const { payload } = JSON.parse(event.data);
		const waiting = payload?.request_id && pending.get(payload.request_id);
		if (!waiting) return;
		pending.delete(payload.request_id);
		if (payload.error) waiting.reject(new Error(payload.error));
		else waiting.resolve(payload.value ?? payload.result ?? null);
	};

	function request(type, extra) {
		const request_id = String(++counter);
		return new Promise((resolve, reject) => {
			pending.set(request_id, { resolve, reject });
			ws.send(JSON.stringify({ type, payload: { request_id, ...extra } }));
			setTimeout(() => reject(new Error(`${type} timed out`)), 5000);
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

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
