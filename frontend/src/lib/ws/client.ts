import type { ClientMessage, ServerMessage } from '$lib/generated/index.js';
import type { JsonValue } from '$lib/generated/serde_json/JsonValue.js';
import type { PathSegment } from '$lib/generated/index.js';

/** Called with the new value and the exact path it was written to. */
export type SubscriptionHandler = (value: unknown, path: PathSegment[]) => void;

type PendingRequest = {
	resolve: (msg: ServerMessage) => void;
	reject: (err: Error) => void;
};

export class PultWsClient {
	private socket: WebSocket | null = null;
	private pending = new Map<string, PendingRequest>();
	private subscriptionHandlers = new Map<string, Set<SubscriptionHandler>>();
	private reconnectTimeout: ReturnType<typeof setTimeout> | null = null;
	private reconnectDelay = 1000;
	private messageQueue: ClientMessage[] = [];
	private connectListeners = new Set<() => void>();

	onConnect: (() => void) | undefined = undefined;
	onDisconnect: (() => void) | undefined = undefined;
	onError: ((message: string) => void) | undefined = undefined;

	constructor(private readonly url: string) {}

	connect(): void {
		if (this.socket?.readyState === WebSocket.OPEN) return;
		try {
			this.socket = new WebSocket(this.url);
		} catch {
			this.scheduleReconnect();
			return;
		}

		this.socket.onopen = () => {
			this.reconnectDelay = 1000;
			console.log('[pult] WebSocket connected');
			// Re-register all active subscriptions with the server
			for (const pattern of this.subscriptionHandlers.keys()) {
				this.socket!.send(JSON.stringify({ type: 'Subscribe', payload: { pattern } }));
			}
			// Flush messages queued before the socket was ready
			const queued = this.messageQueue.splice(0);
			for (const msg of queued) {
				this.socket!.send(JSON.stringify(msg));
			}
			this.connectListeners.forEach((cb) => cb());
			this.onConnect?.();
		};

		this.socket.onclose = () => {
			console.log('[pult] WebSocket disconnected');
			this.onDisconnect?.();
			this.scheduleReconnect();
		};

		this.socket.onerror = () => {
			this.socket?.close();
		};

		this.socket.onmessage = (event) => {
			try {
				const msg: ServerMessage = JSON.parse(event.data);
				this.handleServerMessage(msg);
			} catch (e) {
				console.warn('[pult] failed to parse server message', e);
			}
		};
	}

	disconnect(): void {
		if (this.reconnectTimeout) clearTimeout(this.reconnectTimeout);
		this.socket?.close();
		this.socket = null;
	}

	/** Register a callback to fire on every (re)connect. Returns an unsubscribe function. */
	addConnectListener(cb: () => void): () => void {
		this.connectListeners.add(cb);
		return () => this.connectListeners.delete(cb);
	}

	private scheduleReconnect(): void {
		if (this.reconnectTimeout) return;
		this.reconnectTimeout = setTimeout(() => {
			this.reconnectTimeout = null;
			this.reconnectDelay = Math.min(this.reconnectDelay * 2, 16000);
			this.connect();
		}, this.reconnectDelay);
	}

	private handleServerMessage(msg: ServerMessage): void {
		if (msg.type === 'Pong') return;

		if (msg.type === 'Update') {
			this.subscriptionHandlers.forEach((handlers, pattern) => {
				if (pathMatchesPattern(msg.payload.path, pattern)) {
					handlers.forEach((h) => h(msg.payload.value, msg.payload.path));
				}
			});
			return;
		}

		if (msg.type === 'GetResult' || msg.type === 'SetAck' || msg.type === 'CallResult') {
			const id = msg.payload.request_id;
			const pending = this.pending.get(id);
			if (pending) {
				this.pending.delete(id);
				pending.resolve(msg);
			}
			return;
		}

		if (msg.type === 'Error') {
			console.error('[pult] server error:', msg.payload.message);
			this.onError?.(msg.payload.message);
		}
	}

	private send(msg: ClientMessage): void {
		if (this.socket?.readyState === WebSocket.OPEN) {
			this.socket.send(JSON.stringify(msg));
		} else {
			this.messageQueue.push(msg);
		}
	}

	private requestId(): string {
		return crypto.randomUUID();
	}

	async get(path: (string | number)[]): Promise<unknown> {
		const id = this.requestId();
		return new Promise((resolve, reject) => {
			this.pending.set(id, {
				resolve: (msg) => {
					if (msg.type === 'GetResult') resolve(msg.payload.value);
					else reject(new Error('unexpected response'));
				},
				reject,
			});
			this.send({ type: 'Get', payload: { path: path.map(segmentFromJs), request_id: id } });
		});
	}

	async set(path: (string | number)[], value: unknown): Promise<void> {
		const id = this.requestId();
		return new Promise((resolve, reject) => {
			this.pending.set(id, {
				resolve: (msg) => {
					if (msg.type === 'SetAck') {
						if (msg.payload.ok) resolve();
						else {
							const err = msg.payload.error ?? 'set failed';
							this.onError?.(err);
							reject(new Error(err));
						}
					} else {
						reject(new Error('unexpected response'));
					}
				},
				reject,
			});
			this.send({
				type: 'Set',
				payload: { path: path.map(segmentFromJs), value: value as JsonValue, request_id: id },
			});
		});
	}

	async call(method: string, args: unknown = {}): Promise<unknown> {
		const id = this.requestId();
		return new Promise((resolve, reject) => {
			this.pending.set(id, {
				resolve: (msg) => {
					if (msg.type === 'CallResult') {
						if (!msg.payload.error) resolve(msg.payload.result ?? null);
						else {
							this.onError?.(msg.payload.error);
							reject(new Error(msg.payload.error));
						}
					} else {
						reject(new Error('unexpected response'));
					}
				},
				reject,
			});
			this.send({ type: 'Call', payload: { method, args: args as JsonValue, request_id: id } });
		});
	}

	/**
	 * Listen to every update whose path matches `pattern`.
	 *
	 * The handler is given the path as well as the value, so a subscriber watching a
	 * whole subtree can apply the change where it landed instead of re-reading.
	 */
	subscribe(pattern: string, handler: SubscriptionHandler): () => void {
		if (!this.subscriptionHandlers.has(pattern)) {
			this.subscriptionHandlers.set(pattern, new Set());
			this.send({ type: 'Subscribe', payload: { pattern } });
		}
		this.subscriptionHandlers.get(pattern)!.add(handler);
		return () => {
			const handlers = this.subscriptionHandlers.get(pattern);
			if (!handlers) return;
			handlers.delete(handler);
			if (handlers.size === 0) {
				this.subscriptionHandlers.delete(pattern);
				this.send({ type: 'Unsubscribe', payload: { pattern } });
			}
		};
	}

	ping(): void {
		this.send({ type: 'Ping' });
	}
}

// ── Path helpers ──────────────────────────────────────────────────────────────

function segmentFromJs(s: string | number): string | number {
	return s;
}

function pathMatchesPattern(path: unknown[], pattern: string): boolean {
	const pathParts = (path as (string | number)[]).map(String);
	const patternParts = pattern.split('/');
	return matchPattern(patternParts, pathParts);
}

function matchPattern(pattern: string[], path: string[]): boolean {
	if (pattern.length === 0 && path.length === 0) return true;
	if (pattern[0] === '**') {
		for (let skip = 0; skip <= path.length; skip++) {
			if (matchPattern(pattern.slice(1), path.slice(skip))) return true;
		}
		return false;
	}
	if (pattern.length === 0 || path.length === 0) return false;
	if (pattern[0] === '*' || pattern[0] === path[0]) {
		return matchPattern(pattern.slice(1), path.slice(1));
	}
	return false;
}
