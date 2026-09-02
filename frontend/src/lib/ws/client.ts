import type { ClientMessage, HistoryEntry, ServerMessage } from '$lib/generated/index.js';
import type { JsonValue } from '$lib/generated/serde_json/JsonValue.js';
import type { PathSegment } from '$lib/generated/index.js';
import { ClockSync, forgetOffset } from './clock.js';

/** Called with the new value and the exact path it was written to. */
export type SubscriptionHandler = (value: unknown, path: PathSegment[]) => void;

/** What an undo actually took back: one path to name it, and how many there were. */
export type UndoOutcome = { path: PathSegment[]; changed: number };

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
	/** Who this client last said it was, so a reconnect can say it again. */
	private userId: string | null = null;
	/** The one act the writes going out right now are part of, if there is one. */
	private gesture: string | null = null;
	private connectListeners = new Set<() => void>();
	/**
	 * What time the station thinks it is.
	 *
	 * Kept here because it belongs to the connection: the objects a browser evaluates
	 * are anchored in the console milliseconds of the station on the other end of this
	 * socket, and a different station would be a different clock.
	 */
	private clock = new ClockSync({
		ask: (sentAt) => this.send({ type: 'ClockSync', payload: { sent_at: sentAt } }),
	});

	onConnect: (() => void) | undefined = undefined;
	onDisconnect: (() => void) | undefined = undefined;
	onError: ((message: string) => void) | undefined = undefined;

	constructor(readonly url: string) {}

	/**
	 * The same backend, over HTTP.
	 *
	 * Assets are bytes and travel as ordinary requests rather than over this socket,
	 * but they come from the same station — so the address is derived from the one
	 * already given here rather than worked out a second time.
	 */
	httpUrl(path: string): string {
		const base = new URL(this.url);
		base.protocol = base.protocol === 'wss:' ? 'https:' : 'http:';
		base.pathname = path;
		return base.toString();
	}

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
			// Before anything else: a write that lands before the server knows who
			// sent it is a write nobody can take back.
			if (this.userId !== null) {
				this.socket!.send(
					JSON.stringify({ type: 'Identify', payload: { user_id: this.userId } })
				);
			}
			// Re-register all active subscriptions with the server
			for (const pattern of this.subscriptionHandlers.keys()) {
				this.socket!.send(JSON.stringify({ type: 'Subscribe', payload: { pattern } }));
			}
			// Flush messages queued before the socket was ready
			const queued = this.messageQueue.splice(0);
			for (const msg of queued) {
				this.socket!.send(JSON.stringify(msg));
			}
			// Ask what time it is before anything asks what a light is doing. The
			// answer is what makes every evaluation on this page mean anything.
			this.clock.start();
			this.connectListeners.forEach((cb) => cb());
			this.onConnect?.();
		};

		this.socket.onclose = () => {
			console.log('[pult] WebSocket disconnected');
			// The offset belonged to that connection. Keeping it across a reconnect
			// would mean drawing the rig against a station this client is no longer
			// talking to, which is exactly the silent wrongness this exists to avoid.
			this.clock.stop();
			forgetOffset();
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
		this.clock.stop();
		forgetOffset();
		this.socket?.close();
		this.socket = null;
	}

	/** Register a callback to fire on every (re)connect. Returns an unsubscribe function. */
	addConnectListener(cb: () => void): () => void {
		this.connectListeners.add(cb);
		return () => this.connectListeners.delete(cb);
	}

	/**
	 * Try again now, rather than waiting out the backoff.
	 *
	 * The delay doubles to sixteen seconds, which is right for a console left running
	 * overnight and much too long for somebody who has just started the backend and is
	 * watching the screen. Ignored while a socket is already opening, because a second
	 * one would leave the first to fail and schedule a reconnect of its own.
	 */
	retryNow(): void {
		if (this.socket?.readyState === WebSocket.CONNECTING) return;
		if (this.reconnectTimeout) {
			clearTimeout(this.reconnectTimeout);
			this.reconnectTimeout = null;
		}
		this.reconnectDelay = 1000;
		this.connect();
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

		if (msg.type === 'ClockSync') {
			this.clock.answered(msg.payload.sent_at, msg.payload.station_ms);
			return;
		}

		if (msg.type === 'Update') {
			this.subscriptionHandlers.forEach((handlers, pattern) => {
				if (pathMatchesPattern(msg.payload.path, pattern)) {
					handlers.forEach((h) => h(msg.payload.value, msg.payload.path));
				}
			});
			return;
		}

		// Every reply that carries a request id, so the promise waiting on it settles.
		// A type left off this list does not fail — it hangs, which is worse, because
		// the caller's `finally` never runs either.
		if (
			msg.type === 'GetResult' ||
			msg.type === 'SetAck' ||
			msg.type === 'CallResult' ||
			msg.type === 'UndoResult' ||
			msg.type === 'HistoryResult'
		) {
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
				payload: {
					path: path.map(segmentFromJs),
					value: value as JsonValue,
					request_id: id,
					// Stamped here rather than at every call site: a drag reaches the
					// socket through the programmer's staging, a path proxy and a
					// panel's own handler, and any of the three forgetting would make
					// a drag cost a hundred Ctrl-Zs.
					...(this.gesture ? { gesture: this.gesture } : {}),
				},
			});
		});
	}

	/**
	 * Say who is at this client, so its writes can be taken back.
	 *
	 * Remembered and re-sent on reconnect: a socket that drops mid-show and comes
	 * back anonymous would keep working and quietly stop being undoable, which is
	 * the kind of fault nobody notices until they press Ctrl-Z.
	 */
	identify(userId: string | null): void {
		this.userId = userId;
		this.send({ type: 'Identify', payload: { user_id: userId } });
	}

	/**
	 * Mark every write from now until this is unset as one act.
	 *
	 * Not remembered across a reconnect, unlike the user: a gesture is a pointer
	 * being held, and a socket that dropped in the middle of one has already lost
	 * the drag. Whoever opened it closes it.
	 */
	duringGesture(gesture: string | null): void {
		this.gesture = gesture;
	}

	/** Take back this user's last gesture, or put back their last undo. */
	async undo(redo = false): Promise<UndoOutcome | null> {
		const id = this.requestId();
		return new Promise((resolve, reject) => {
			this.pending.set(id, {
				resolve: (msg) => {
					if (msg.type !== 'UndoResult') return reject(new Error('unexpected response'));
					const { undone, changed } = msg.payload;
					resolve(undone ? { path: undone, changed } : null);
				},
				reject
			});
			this.send({ type: 'Undo', payload: { redo, request_id: id } });
		});
	}

	/** The recent history, newest first. */
	async history(limit = 100): Promise<HistoryEntry[]> {
		const id = this.requestId();
		return new Promise((resolve, reject) => {
			this.pending.set(id, {
				resolve: (msg) => {
					if (msg.type === 'HistoryResult') resolve(msg.payload.entries);
					else reject(new Error('unexpected response'));
				},
				reject
			});
			this.send({ type: 'History', payload: { limit, request_id: id } });
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
