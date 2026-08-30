/**
 * Who is at this client.
 *
 * Chosen rather than derived from the machine, because one person often has two
 * clients — the desktop console and a tablet on the same show — and both are them.
 * That is the whole point: undo is per *person*, so an operator can take back on the
 * tablet what they did at the desk.
 *
 * Kept in `localStorage` like the layout, and for the same reason: which user this
 * browser is is not a fact about the show. The `users` collection, on the other hand,
 * is show data — a name to attribute a change to has to mean the same thing on every
 * console.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import type { User } from '$lib/generated/index.js';
import { colourFor } from '$lib/users.js';
import { collection, showClient, showData } from './show.js';

const STORAGE_KEY = 'pult.user';

export const users = collection('users');

/** The id this browser is working as, or null before anybody has said. */
export const userId = writable<string | null>(read());

/** The user themselves, once the collection has caught up. */
export const currentUser: Readable<User | null> = derived(
	[userId, users],
	([$id, $users]) => $users.find((u) => u.id === $id) ?? null
);

function read(): string | null {
	if (typeof localStorage === 'undefined') return null;
	try {
		return localStorage.getItem(STORAGE_KEY);
	} catch {
		// A browser with storage turned off still works; it just asks who you are
		// every time, which is annoying rather than broken.
		return null;
	}
}

/**
 * Work as this user from now on.
 *
 * Told to the socket as well as remembered, because the backend attributes writes
 * per connection — the store alone would leave the desk thinking nobody was there.
 */
export function beUser(id: string | null): void {
	userId.set(id);
	try {
		if (id) localStorage.setItem(STORAGE_KEY, id);
		else localStorage.removeItem(STORAGE_KEY);
	} catch {
		// Not being remembered is survivable; not being identified is not, so the
		// socket is told either way.
	}
	showClient().identify(id);
}

/** Add somebody and start working as them. */
export async function addUser(name: string): Promise<void> {
	const trimmed = name.trim();
	if (!trimmed) return;
	const id = crypto.randomUUID();
	await showData().users.create({
		id,
		name: trimmed,
		colour: colourFor(get(users).length)
	});
	beUser(id);
}

/** Tell the socket who we are, once there is a socket. Called from `+layout.svelte`. */
export function identifyOnConnect(): void {
	const id = get(userId);
	if (id) showClient().identify(id);
}
