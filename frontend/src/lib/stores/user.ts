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
 *
 * There is no nobody. A client that has never been told who is at it works as the
 * show's default user, which the backend guarantees exists, because a write carrying
 * no author can never be taken back — and a console whose first change is permanent
 * is a console with undo switched off. What is remembered here is therefore not "who
 * am I" but "has anybody said", and those are different questions: the first always
 * has an answer, the second is what decides whether to keep asking.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import type { User } from '$lib/generated/index.js';
import { colourFor, DEFAULT_USER_ID } from '$lib/users.js';
import { collection, showClient, showData } from './show.js';

const STORAGE_KEY = 'pult.user';

export const users = collection('users');

/** The id this browser is working as. The default until somebody says otherwise. */
export const userId = writable<string>(read() ?? DEFAULT_USER_ID);

/**
 * Whether anybody has said who they are at this client.
 *
 * Client state rather than show data: whether *this browser* has been told is not a
 * fact about the show, and two browsers on one show can honestly differ about it.
 * False means the client is working as the default because nothing was chosen, which
 * is worth saying out loud — everyone unsaid shares one undo history.
 */
export const hasChosen = writable<boolean>(read() !== null);

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
 *
 * Choosing the default *deliberately* is a choice like any other and is remembered
 * as one, so the console stops asking. That is why this does not special-case the
 * default id: the distinction that matters is whether somebody said, not what they
 * said.
 */
export function beUser(id: string): void {
	userId.set(id);
	hasChosen.set(true);
	try {
		localStorage.setItem(STORAGE_KEY, id);
	} catch {
		// Not being remembered is survivable; not being identified is not, so the
		// socket is told either way.
	}
	showClient().identify(id);
}

/**
 * Stop working as whoever this client was working as.
 *
 * A real thing to want on a shared desk at the end of a session — and it lands on
 * the show's default rather than on nobody, because nobody is a state in which
 * nothing can be taken back. The stored identity goes, so the next visit to this
 * browser starts as the default rather than as whoever last used it.
 */
export function signOut(): void {
	try {
		localStorage.removeItem(STORAGE_KEY);
	} catch {
		// Nothing to clear, or nowhere to clear it. Either way the socket is told
		// below, which is the part that decides what a write is attributed to.
	}
	userId.set(DEFAULT_USER_ID);
	hasChosen.set(false);
	showClient().identify(DEFAULT_USER_ID);
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
	showClient().identify(get(userId));
}
