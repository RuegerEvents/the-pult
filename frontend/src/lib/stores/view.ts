/**
 * How this screen draws the rig.
 *
 * Neither show data nor a console setting. The haze is the show's, because how hazy
 * the room is is a fact about the room; a work light is not — it is how brightly
 * *this screen* draws what no lamp is lighting, so a designer squinting at a dark
 * pre-vis and an operator with the house lights up can each see the trusses without
 * arguing about it through the showfile. And how many pixels a panel renders is a
 * fact about the machine in front of somebody, which is why it lives beside the
 * layout in `localStorage` rather than in either.
 *
 * Every rig panel on this screen reads the one store, so two of them open at once
 * agree.
 */

import { browser } from '$app/environment';
import { writable } from 'svelte/store';

export type ViewSettings = {
	/**
	 * How much light there is in the room that no fixture is making, 0–1. One is the
	 * house lights up, bright enough to read every truss; zero is a blackout with
	 * only the beams showing; 0.4 is the view as it was first drawn.
	 */
	workLight: number;
	/**
	 * The most device pixels per CSS pixel the rig view renders at. A Retina display
	 * asks for two and a ProMotion one draws a hundred and twenty times a second, and
	 * a beam shader running over every one of those pixels is what pinned a GPU at
	 * full load; one and a half is indistinguishable at arm's length and costs a bit
	 * over half as much.
	 */
	resolution: number;
};

export const DEFAULT_VIEW: ViewSettings = { workLight: 0.4, resolution: 1.5 };

/** The choices the panel offers for resolution, and what to call them. */
export const RESOLUTIONS: { value: number; label: string }[] = [
	{ value: 1, label: 'Fast (1×)' },
	{ value: 1.5, label: 'Balanced (1.5×)' },
	{ value: 2, label: 'Sharp (2×)' }
];

const STORAGE_KEY = 'pult.view';

function readBack(): ViewSettings {
	if (!browser) return DEFAULT_VIEW;
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return DEFAULT_VIEW;
		const parsed = JSON.parse(raw) as Partial<ViewSettings>;
		return {
			workLight: finiteIn(parsed.workLight, 0, 1, DEFAULT_VIEW.workLight),
			resolution: finiteIn(parsed.resolution, 0.5, 3, DEFAULT_VIEW.resolution)
		};
	} catch {
		return DEFAULT_VIEW;
	}
}

function finiteIn(value: unknown, min: number, max: number, fallback: number): number {
	return typeof value === 'number' && Number.isFinite(value)
		? Math.min(max, Math.max(min, value))
		: fallback;
}

export const view = writable<ViewSettings>(readBack());

export function setView(change: Partial<ViewSettings>): void {
	view.update((current) => {
		const next = {
			workLight: finiteIn(change.workLight ?? current.workLight, 0, 1, current.workLight),
			resolution: finiteIn(change.resolution ?? current.resolution, 0.5, 3, current.resolution)
		};
		if (browser) {
			try {
				localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
			} catch {
				// A browser with storage turned off still gets the setting for this page.
			}
		}
		return next;
	});
}
