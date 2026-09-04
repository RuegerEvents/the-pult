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

/**
 * What a screen draws the rig as. Four questions rather than a quality ladder:
 * where is everything, where is it pointing, what is in the air, what would a
 * camera see.
 */
export type RenderMode = 'wireframe' | 'cones' | 'real' | 'photoreal';

export const RENDER_MODES: { value: RenderMode; label: string; blurb: string }[] = [
	{ value: 'wireframe', label: 'Wireframe', blurb: 'Trusses and bodies as lines, and a line to where each light points. Costs almost nothing.' },
	{ value: 'cones', label: 'Cones', blurb: 'Flat cones in each light\'s colour, nothing added. Never goes white, and cheap enough for any rig.' },
	{ value: 'real', label: 'Real', blurb: 'Beams through the haze. The working view.' },
	{ value: 'photoreal', label: 'Photoreal', blurb: 'Beams summed in high dynamic range, tone-mapped and bloomed, so crossing beams roll off instead of clipping.' }
];

export type ViewSettings = {
	mode: RenderMode;
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

export const DEFAULT_VIEW: ViewSettings = { mode: 'real', workLight: 0.4, resolution: 1.5 };

/** The choices the panel offers for resolution, and what to call them. */
export const RESOLUTIONS: { value: number; label: string }[] = [
	{ value: 1, label: 'Fast (1×)' },
	{ value: 1.5, label: 'Balanced (1.5×)' },
	{ value: 2, label: 'Sharp (2×)' }
];

const STORAGE_KEY = 'pult.view';

/**
 * Whatever storage or a caller hands over, brought inside what the view will do —
 * on top of `fallback`, so a partial change keeps the rest.
 */
export function parseView(value: unknown, fallback: ViewSettings = DEFAULT_VIEW): ViewSettings {
	const given = (value && typeof value === 'object' ? value : {}) as Partial<ViewSettings>;
	const mode = RENDER_MODES.some((m) => m.value === given.mode) ? (given.mode as RenderMode) : fallback.mode;
	return {
		mode,
		workLight: finiteIn(given.workLight, 0, 1, fallback.workLight),
		resolution: finiteIn(given.resolution, 0.5, 3, fallback.resolution)
	};
}

function readBack(): ViewSettings {
	if (!browser) return DEFAULT_VIEW;
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return DEFAULT_VIEW;
		return parseView(JSON.parse(raw));
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
		const next = parseView(change, current);
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
