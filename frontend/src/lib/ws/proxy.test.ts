import { describe, expect, it } from 'vitest';
import { createDataProxy } from './proxy.js';
import type { PultWsClient } from './client.js';
import type { PathSegment } from '$lib/generated/index.js';

type Handler = (value: unknown, path: PathSegment[]) => void;

/** A stand-in for the WebSocket client that records what the proxy asks it to do. */
function fakeClient(initial: unknown) {
	let stored = initial;
	const handlers: { pattern: string; handler: Handler }[] = [];
	const gets: (string | number)[][] = [];
	const sets: [(string | number)[], unknown][] = [];

	const client = {
		get(path: (string | number)[]) {
			gets.push(path);
			return Promise.resolve(stored);
		},
		set(path: (string | number)[], value: unknown) {
			sets.push([path, value]);
			return Promise.resolve();
		},
		subscribe(pattern: string, handler: Handler) {
			handlers.push({ pattern, handler });
			return () => {};
		},
		addConnectListener() {
			return () => {};
		}
	} as unknown as PultWsClient;

	return {
		client,
		gets,
		sets,
		/** Deliver an update the way the backend would. */
		push(path: PathSegment[], value: unknown) {
			for (const { handler } of handlers) handler(value, path);
		},
		setStored(v: unknown) {
			stored = v;
		}
	};
}

const flush = () => new Promise((r) => setTimeout(r, 0));

const fixtures = () => [
	{ id: 'a', name: 'PAR L', live_values: {} },
	{ id: 'b', name: 'PAR R', live_values: {} }
];

describe('subscribeDeep', () => {
	it('delivers the collection once on subscribe', async () => {
		const fake = fakeClient(fixtures());
		const seen: unknown[] = [];
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep((v: unknown) => seen.push(v));
		await flush();

		expect(fake.gets).toHaveLength(1);
		expect(seen).toHaveLength(1);
	});

	// This is the whole point: a fade sends one update per moving fixture per tick,
	// and re-reading the collection each time was dozens of round trips a second.
	it('applies a field update without going back to the server', async () => {
		const fake = fakeClient(fixtures());
		const seen: unknown[] = [];
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep((v: unknown) => seen.push(v));
		await flush();
		fake.gets.length = 0;

		fake.push(['fixtures', 'a', 'live_values'], { Intensity: { type: 'Float', value: 1 } });
		await flush();

		expect(fake.gets).toHaveLength(0);
		const latest = seen.at(-1) as ReturnType<typeof fixtures>;
		expect(latest[0].live_values).toEqual({ Intensity: { type: 'Float', value: 1 } });
		expect(latest[1].live_values).toEqual({});
	});

	it('leaves the other fields of the entity alone', async () => {
		const fake = fakeClient(fixtures());
		const seen: unknown[] = [];
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep((v: unknown) => seen.push(v));
		await flush();

		fake.push(['fixtures', 'a', 'name'], 'Renamed');
		await flush();

		const latest = seen.at(-1) as ReturnType<typeof fixtures>;
		expect(latest[0].name).toBe('Renamed');
		expect(latest[0].id).toBe('a');
	});

	it('replaces a whole entity when the update names one', async () => {
		const fake = fakeClient(fixtures());
		const seen: unknown[] = [];
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep((v: unknown) => seen.push(v));
		await flush();
		fake.gets.length = 0;

		fake.push(['fixtures', 'b'], { id: 'b', name: 'Replaced', live_values: {} });
		await flush();

		expect(fake.gets).toHaveLength(0);
		const latest = seen.at(-1) as ReturnType<typeof fixtures>;
		expect(latest[1].name).toBe('Replaced');
	});

	it('takes a create or delete straight from the collection update', async () => {
		const fake = fakeClient(fixtures());
		const seen: unknown[] = [];
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep((v: unknown) => seen.push(v));
		await flush();
		fake.gets.length = 0;

		fake.push(['fixtures'], [fixtures()[0]]);
		await flush();

		expect(fake.gets).toHaveLength(0);
		expect((seen.at(-1) as unknown[]).length).toBe(1);
	});

	// Anything unrecognised has to be slow rather than wrong.
	it('re-reads when the update is for an entity it has never seen', async () => {
		const fake = fakeClient(fixtures());
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep(() => {});
		await flush();
		fake.gets.length = 0;

		fake.push(['fixtures', 'unknown-id', 'name'], 'Ghost');
		await flush();

		expect(fake.gets).toHaveLength(1);
	});

	it('re-reads when the update reaches deeper than a field', async () => {
		const fake = fakeClient(fixtures());
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		(createDataProxy(fake.client, ['fixtures']) as any).subscribeDeep(() => {});
		await flush();
		fake.gets.length = 0;

		fake.push(['fixtures', 'a', 'live_values', 'Intensity'], { type: 'Float', value: 1 });
		await flush();

		expect(fake.gets).toHaveLength(1);
	});
});


describe('relative writes', () => {
	/**
	 * The delta goes to the station, not the answer. Two people nudging one fader
	 * both get their nudge; two people computing 60 from the 50 they each read leave
	 * only one of them heard.
	 */
	it('sends the delta under __by rather than a destination', async () => {
		const { client, sets } = fakeClient({ fade_in_ms: 3000 });
		const proxy = createDataProxy<{ fade_in_ms: number }>(client, ['cues', 'c1']);

		await proxy.fade_in_ms.by(1500);

		expect(sets).toEqual([[['cues', 'c1', 'fade_in_ms', '__by'], 1500]]);
	});

	it('leaves set alone', async () => {
		const { client, sets } = fakeClient({ fade_in_ms: 3000 });
		const proxy = createDataProxy<{ fade_in_ms: number }>(client, ['cues', 'c1']);

		await proxy.fade_in_ms.set(4500);

		expect(sets).toEqual([[['cues', 'c1', 'fade_in_ms'], 4500]]);
	});
});
