import type { PultWsClient } from './client.js';

// ── PathProxy types ───────────────────────────────────────────────────────────

type LeafProxy<T> = {
	set(value: T): Promise<void>;
	get(): Promise<T>;
	subscribe(cb: (value: T) => void): () => void;
};

type ObjectProxy<T> = LeafProxy<T> & {
	readonly [K in keyof T]: PathProxy<T[K]>;
};

type ArrayProxy<E> = LeafProxy<E[]> & {
	[n: number]: PathProxy<E>;
	nth(n: number): PathProxy<E>;
	byId(id: string): PathProxy<E>;
};

export type PathProxy<T> = T extends (infer E)[]
	? ArrayProxy<E>
	: T extends object
		? ObjectProxy<T>
		: LeafProxy<T>;

// ── Runtime proxy factory ─────────────────────────────────────────────────────

export function createDataProxy<T>(
	client: PultWsClient,
	path: (string | number)[] = []
): PathProxy<T> {
	return new Proxy({} as PathProxy<T>, {
		get(_target, prop) {
			if (prop === 'set') {
				return (value: T) => client.set(path, value);
			}
			if (prop === 'get') {
				return () => client.get(path) as Promise<T>;
			}
			if (prop === 'subscribe') {
				return (cb: (value: T) => void) => {
					const pattern = path.join('/');
					return client.subscribe(pattern, cb as (value: unknown) => void);
				};
			}
			if (prop === 'nth') {
				return (n: number) => createDataProxy(client, [...path, n]);
			}
			if (prop === 'byId') {
				return (id: string) => createDataProxy(client, [...path, id]);
			}
			if (typeof prop === 'symbol') return undefined;
			// Numeric index access: proxy[5]
			const num = Number(prop);
			if (!Number.isNaN(num)) {
				return createDataProxy(client, [...path, num]);
			}
			// String key access: proxy.name
			return createDataProxy(client, [...path, prop as string]);
		},
	});
}

// ── Svelte store helper ───────────────────────────────────────────────────────

import { readable, type Readable } from 'svelte/store';

/**
 * Creates a Svelte readable store that fetches the initial value and subscribes
 * to live updates for the given path proxy.
 */
export function proxyStore<T>(proxy: PathProxy<T>, initial: T): Readable<T> {
	return readable<T>(initial, (set) => {
		// Fetch initial snapshot
		proxy.get().then((v) => set(v as T)).catch(() => {});
		// Subscribe to updates
		const unsub = proxy.subscribe((v) => set(v as T));
		return unsub;
	});
}
