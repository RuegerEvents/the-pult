/**
 * What this console prefers, whichever show it has open.
 *
 * The third kind of setting the system has. A show's settings live in the showfile
 * and replicate to every station; a start-up flag is decided before the console is
 * running. These are neither: they belong to the machine somebody is sitting at, and
 * they change while it runs.
 *
 * Over HTTP rather than the socket because they are not show data — nothing here
 * syncs, nothing here is in the oplog, and nothing here can be taken back.
 */

import type { FadeCurves } from './generated/index.js';
import { backendOrigin } from './ws/endpoint.js';

export type Preferences = {
	/** What a newly created show starts its history depth at. */
	historyDepth: number;
	/** The range the console will actually accept, so a control can say so. */
	historyDepthMin: number;
	historyDepthMax: number;
	/** What a newly created show starts its home fade time at, in milliseconds. */
	homeFadeMs: number;
	homeFadeMsMax: number;
	/** What a newly created show starts its haze at. Both 0 to 1. */
	hazeDensity: number;
	hazeTurbulence: number;
	/** And what shape its fades have, per group of parameter. */
	fadeCurves: FadeCurves;
	/** How often this station takes a version of its own, in minutes. `0` is off. */
	autosaveMinutes: number;
	/** How many of those it keeps before the oldest is dropped. */
	autosaveKeep: number;
};

const url = () => `${backendOrigin(window.location)}/api/preferences`;

export async function readPreferences(): Promise<Preferences | null> {
	try {
		const response = await fetch(url());
		return response.ok ? ((await response.json()) as Preferences) : null;
	} catch {
		// A console whose preferences cannot be read still opens the show; the panel
		// says it could not rather than the page failing to load.
		return null;
	}
}

/**
 * Change them, and answer with what was actually stored.
 *
 * Not always what was asked for — a value outside what the console will do comes
 * back at the nearest one that is — which is why the caller takes the answer rather
 * than assuming. Only what is named changes; the rest is left alone.
 */
export async function writePreferences(
	change: Partial<
		Pick<
			Preferences,
			'historyDepth' | 'homeFadeMs' | 'hazeDensity' | 'hazeTurbulence' | 'fadeCurves'
		>
	>
): Promise<Preferences | null> {
	try {
		const response = await fetch(url(), {
			method: 'PUT',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify(change)
		});
		return response.ok ? ((await response.json()) as Preferences) : null;
	} catch {
		return null;
	}
}
