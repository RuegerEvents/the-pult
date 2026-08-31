/**
 * There is no nobody.
 *
 * A client that has never been told who is at it still has to be somebody, because
 * a write carrying no author can never be taken back. These pin down the three
 * states that matter — never asked, told, and told and then signed out — and that
 * none of them is nobody.
 *
 * The module reads `localStorage` when it is first imported, so each test sets the
 * storage it wants and then imports a fresh copy.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

import { DEFAULT_USER_ID } from '../users.js';

/** What the socket was told, in order. The backend attributes writes per connection,
 *  so this is the half that decides what an operation carries. */
const identified: (string | null)[] = [];

vi.mock('./show.js', () => ({
	collection: () => ({ subscribe: (run: (v: unknown[]) => void) => (run([]), () => {}) }),
	showClient: () => ({ identify: (id: string | null) => identified.push(id) }),
	showData: () => ({ users: { create: async () => {} } })
}));

const store = new Map<string, string>();

beforeEach(() => {
	store.clear();
	identified.length = 0;
	vi.resetModules();
	vi.stubGlobal('localStorage', {
		getItem: (k: string) => store.get(k) ?? null,
		setItem: (k: string, v: string) => void store.set(k, v),
		removeItem: (k: string) => void store.delete(k)
	});
});

const load = () => import('./user.js');

describe('a client that has never been told who is at it', () => {
	it('works as the show default rather than as nobody', async () => {
		const { userId } = await load();
		expect(get(userId)).toBe(DEFAULT_USER_ID);
	});

	it('knows that nobody has said, so the console can ask', async () => {
		const { hasChosen } = await load();
		expect(get(hasChosen)).toBe(false);
	});

	it('tells the socket who it is on connect, so its writes are attributed', async () => {
		const { identifyOnConnect } = await load();
		identifyOnConnect();
		expect(identified).toEqual([DEFAULT_USER_ID]);
	});
});

describe('a client that has been told', () => {
	it('keeps working as who it was told', async () => {
		store.set('pult.user', 'a-chosen-id');
		const { userId, hasChosen } = await load();
		expect(get(userId)).toBe('a-chosen-id');
		expect(get(hasChosen)).toBe(true);
	});

	it('remembers, and tells the socket', async () => {
		const { beUser, userId, hasChosen } = await load();
		beUser('somebody');
		expect(get(userId)).toBe('somebody');
		expect(get(hasChosen)).toBe(true);
		expect(store.get('pult.user')).toBe('somebody');
		expect(identified).toEqual(['somebody']);
	});

	/**
	 * Choosing the default on purpose is a choice like any other. What is remembered
	 * is that somebody said, not what they said, so the console stops asking.
	 */
	it('stops asking once the default is chosen deliberately', async () => {
		const { beUser, userId, hasChosen } = await load();
		beUser(DEFAULT_USER_ID);
		expect(get(userId)).toBe(DEFAULT_USER_ID);
		expect(get(hasChosen)).toBe(true);
	});
});

describe('signing out', () => {
	it('lands on the default rather than on nobody', async () => {
		store.set('pult.user', 'a-chosen-id');
		const { signOut, userId, hasChosen } = await load();

		signOut();

		expect(get(userId)).toBe(DEFAULT_USER_ID);
		expect(get(hasChosen)).toBe(false);
		expect(identified.at(-1)).toBe(DEFAULT_USER_ID);
	});

	it('forgets, so the next visit is not still the person who left', async () => {
		store.set('pult.user', 'a-chosen-id');
		const { signOut } = await load();
		signOut();
		expect(store.has('pult.user')).toBe(false);

		vi.resetModules();
		const { userId } = await load();
		expect(get(userId)).toBe(DEFAULT_USER_ID);
	});
});

describe('a browser with storage turned off', () => {
	it('is still somebody', async () => {
		vi.stubGlobal('localStorage', {
			getItem: () => {
				throw new Error('denied');
			},
			setItem: () => {
				throw new Error('denied');
			},
			removeItem: () => {
				throw new Error('denied');
			}
		});
		const { userId, beUser } = await load();
		expect(get(userId)).toBe(DEFAULT_USER_ID);

		// Not being remembered is survivable; not being identified is not.
		beUser('somebody');
		expect(get(userId)).toBe('somebody');
		expect(identified).toEqual(['somebody']);
	});
});
