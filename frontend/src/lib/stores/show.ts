/**
 * The show, as stores every panel can share.
 *
 * A tiled workspace can have the same collection on screen four times at once — the
 * rig, the plan, the values panel and the patch table all want the fixtures. Each
 * opening its own `subscribeDeep` would mean four copies of every fixture in memory
 * and four applications of every update, forty times a second during a fade.
 *
 * So there is one store per table, made once and handed out. Svelte's `readable`
 * already reference-counts for us: the subscription opens when the first panel asks
 * for it and closes when the last one goes away, which is why a panel nobody has
 * open costs nothing.
 *
 * Nothing here names a table. The keys come from `DataRoot`, which pult-codegen
 * generates from the schema, so a new collection is usable here the moment it exists.
 */

import { readable, type Readable } from 'svelte/store';
import type { Show } from '$lib/generated/index.js';
import type { DataRoot } from '$lib/ws/data.js';
import type { PultWsClient } from '$lib/ws/client.js';

/** The tables that are collections, as opposed to the singletons beside them. */
type Collections = {
	[K in keyof DataRoot as DataRoot[K] extends { subscribeDeep: unknown } ? K : never]: DataRoot[K];
};

/** What one row of a collection is. */
type Row<T> = T extends { get(): Promise<(infer E)[]> } ? E : never;

export type CollectionName = keyof Collections;

let root: DataRoot | null = null;
/**
 * The socket itself, for the few things that are not path writes.
 *
 * Undo, identify and the history are asked of the connection rather than of a path,
 * and a store has no Svelte context to pull one out of the way a component does.
 */
let socket: PultWsClient | null = null;
const stores = new Map<string, Readable<unknown[]>>();
/** Subscribers that asked for data before the connection existed. */
let waiting: ((data: DataRoot) => void)[] = [];

/** Point the shared stores at the live connection. Called once, from `+layout.svelte`. */
export function initShowStores(data: DataRoot, ws: PultWsClient): void {
	root = data;
	socket = ws;
	const queued = waiting;
	waiting = [];
	for (const start of queued) start(data);
}

/**
 * Do something with the connection, now or as soon as there is one.
 *
 * Module bodies run before any component's script does, so a store that subscribes
 * on the way in gets there before `initShowStores` — and the import order deciding
 * whether the programmer works is not something anyone should have to know about.
 */
function whenReady(use: (data: DataRoot) => void): void {
	if (root) use(root);
	else waiting.push(use);
}

/**
 * The connection, for the stores and actions built on top of these.
 *
 * Panels take the data root out of Svelte context, which is right for a component.
 * A store is not a component and has no context to read, so this is where it looks.
 */
export function showData(): DataRoot {
	if (!root) throw new Error('the show stores were used before initShowStores');
	return root;
}

/** The connection, for undo, identify and the history. */
export function showClient(): PultWsClient {
	if (!socket) throw new Error('the show stores were used before initShowStores');
	return socket;
}

/** One collection of the show, live. */
export function collection<K extends CollectionName>(
	table: K
): Readable<Row<Collections[K]>[]> {
	const cached = stores.get(table);
	if (cached) return cached as Readable<Row<Collections[K]>[]>;

	const store = readable<unknown[]>([], (set) => {
		let stop: (() => void) | undefined;
		whenReady((data) => {
			const proxy = data[table] as { subscribeDeep(cb: (v: unknown[]) => void): () => void };
			stop = proxy.subscribeDeep((value) => set(value));
		});
		return () => stop?.();
	});
	stores.set(table, store);
	return store as Readable<Row<Collections[K]>[]>;
}

/** The show itself: one row, or nothing before one has been made. */
export const show: Readable<Show | null> = readable<Show | null>(null, (set) => {
	let stop: (() => void) | undefined;
	whenReady((data) => {
		stop = data.show.subscribe((value) => set(value));
	});
	return () => stop?.();
});

/**
 * One LOCAL path of the station this page is talking to — `audio_status`, `devices`,
 * `takes` — live.
 *
 * Shared and reference-counted like the collections above, and asked again on every
 * reconnect: a LOCAL value is this station's, and a reconnect may be to a station that
 * has a different one. `null` until it has answered.
 */
export function localPath<T>(path: string): Readable<T | null> {
	const key = `local:${path}`;
	const cached = stores.get(key);
	if (cached) return cached as unknown as Readable<T | null>;

	const store = readable<T | null>(null, (set) => {
		const stops: (() => void)[] = [];
		let live = true;
		const apply = (value: unknown) => {
			if (live && value && typeof value === 'object') set(value as T);
		};
		whenReady(() => {
			if (!live || !socket) return;
			const ws = socket;
			stops.push(ws.subscribe(path, apply));
			const fetch = () => void ws.get([path]).then(apply);
			fetch();
			stops.push(ws.addConnectListener(fetch));
		});
		return () => {
			live = false;
			for (const stop of stops) stop();
		};
	});
	stores.set(key, store as unknown as Readable<unknown[]>);
	return store;
}
