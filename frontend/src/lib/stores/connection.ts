/**
 * Whether this page has the console right now, for the views that draw without the
 * workspace's chrome.
 *
 * The root layout owns the socket and says when it opens and closes; the workspace
 * answers a lost console with a cover over everything. A phone surface must not: the
 * maze runner needs to see what the last known state was, and that it is stale, rather
 * than a full-screen panel over the buttons they are about to press.
 */

import { writable } from 'svelte/store';

export type Connection = {
	connected: boolean;
	/** Whether this page has ever had the console. Before it has, there is nothing to show. */
	everConnected: boolean;
};

export const connection = writable<Connection>({ connected: false, everConnected: false });
