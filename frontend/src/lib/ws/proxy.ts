import type { PultWsClient } from './client.js';

// ── Subscribe options ─────────────────────────────────────────────────────────

export type SubscribeOptions = {
	/** Deliver the current value immediately on subscribe (default: true). */
	initial?: boolean;
};

// ── PathProxy types ───────────────────────────────────────────────────────────

type LeafProxy<T> = {
	set(value: T): Promise<void>;
	get(): Promise<T>;
	subscribe(cb: (value: T) => void, opts?: SubscribeOptions): () => void;
};

// Navigation keys come from NonNullable<T> so that nullable object types (e.g. Show | null)
// still expose their fields for path navigation.
type ObjectProxy<T> = LeafProxy<T> & {
	readonly [K in keyof NonNullable<T>]: PathProxy<NonNullable<T>[K]>;
};

type ArrayProxy<E> = LeafProxy<E[]> & {
	[n: number]: PathProxy<E>;
	nth(n: number): PathProxy<E>;
	byId(id: string): PathProxy<E>;
	/** Subscribe to any change at or under this collection; re-fetches and delivers the full array. */
	subscribeDeep(cb: (value: E[]) => void, opts?: SubscribeOptions): () => void;
};

// Non-distributive conditional: [T] prevents union splitting so `number | null` → LeafProxy.
// NonNullable<T> for the branch checks so nullable object types remain navigable.
export type PathProxy<T> = [NonNullable<T>] extends [(infer E)[]]
	? ArrayProxy<E>
	: [NonNullable<T>] extends [object]
		? ObjectProxy<T>
		: LeafProxy<T>;

// ── Runtime proxy factory ─────────────────────────────────────────────────────

export function createDataProxy<T>(
	client: PultWsClient,
	path: (string | number)[] = []
): PathProxy<T> {
	// Function target so the apply trap fires when the proxy is called directly.
	// This makes every nested path callable: proxy.goNext() sets the path with empty args,
	// proxy.goToCue({ cueId }) sets the path with the provided args.
	const target = () => {};
	return new Proxy(target as unknown as PathProxy<T>, {
		get(_target, prop) {
			if (prop === 'set') {
				return (value: unknown) => client.set(path, value);
			}
			if (prop === 'get') {
				return () => client.get(path);
			}
			if (prop === 'subscribe') {
				return (cb: (value: unknown) => void, opts?: SubscribeOptions) => {
					const initial = opts?.initial !== false;
					const doFetch = () => client.get(path).then(v => cb(v)).catch(() => {});
					if (initial) doFetch();
					const unsubData = client.subscribe(path.join('/'), cb);
					const unsubConnect = client.addConnectListener(doFetch);
					return () => { unsubData(); unsubConnect(); };
				};
			}
			if (prop === 'subscribeDeep') {
				return (cb: (value: unknown) => void, opts?: SubscribeOptions) =>
					subscribeDeep(client, path, cb, opts);
			}
			if (prop === 'nth') {
				return (n: number) => createDataProxy(client, [...path, n]);
			}
			if (prop === 'byId') {
				return (id: string) => createDataProxy(client, [...path, id]);
			}
			if (prop === 'delete') {
				return () => client.set([...path, '__delete'], null);
			}
			if (prop === 'create') {
				return (data: unknown) => client.set([...path, '__create'], data);
			}
			if (typeof prop === 'symbol') return undefined;
			const num = Number(prop);
			if (!Number.isNaN(num)) {
				return createDataProxy(client, [...path, num]);
			}
			return createDataProxy(client, [...path, prop as string]);
		},
		apply(_target, _thisArg, args) {
			// Proxy called as a function → treat as a command: set this path with provided args.
			return client.set(path, args[0] ?? {});
		},
	});
}


// ── subscribeDeep ─────────────────────────────────────────────────────────────

type Entity = { id?: string } & Record<string, unknown>;

/**
 * Watch a collection and everything under it.
 *
 * Updates are applied to the local copy where the path says exactly what changed,
 * rather than re-reading the collection. During a fade the backend sends one update
 * per moving fixture per tick, so re-reading would be dozens of round trips a
 * second for data the message already carried.
 *
 * Anything not recognised falls back to a re-read, so an unusual path is slow
 * rather than wrong.
 */
function subscribeDeep(
	client: PultWsClient,
	path: (string | number)[],
	cb: (value: unknown) => void,
	opts?: SubscribeOptions
): () => void {
	const initial = opts?.initial !== false;
	let current: Entity[] | null = null;

	const deliver = () => {
		if (current) cb(current);
	};

	const refetch = () =>
		client
			.get(path)
			.then((v) => {
				current = Array.isArray(v) ? (v as Entity[]) : null;
				deliver();
			})
			.catch(() => {});

	if (initial) refetch();

	const unsubData = client.subscribe([...path, '**'].join('/'), (value, updatePath) => {
		if (!current || !updatePath) {
			refetch();
			return;
		}
		const rest = updatePath.slice(path.length);

		// The collection itself: a create or a delete.
		if (rest.length === 0) {
			current = Array.isArray(value) ? (value as Entity[]) : null;
			deliver();
			return;
		}

		const index = current.findIndex((e) => e?.id === String(rest[0]));
		if (index < 0) {
			refetch();
			return;
		}

		// One whole entity, or one field of it.
		if (rest.length === 1) {
			current = current.map((e, i) => (i === index ? (value as Entity) : e));
			deliver();
			return;
		}
		if (rest.length === 2) {
			const field = String(rest[1]);
			current = current.map((e, i) => (i === index ? { ...e, [field]: value } : e));
			deliver();
			return;
		}

		refetch();
	});
	const unsubConnect = client.addConnectListener(refetch);
	return () => {
		unsubData();
		unsubConnect();
	};
}

// ── Svelte store helper ───────────────────────────────────────────────────────

import { readable, type Readable } from 'svelte/store';

/** Creates a Svelte readable store backed by a path proxy. Auto-fetches initial value. */
export function proxyStore<T>(proxy: PathProxy<T>): Readable<T | null> {
	return readable<T | null>(null, (set) => {
		return proxy.subscribe((v) => set(v as T));
	});
}
