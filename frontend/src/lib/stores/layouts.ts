/**
 * Layouts as a page holds them: the collection, which one the desk is looking at, and
 * the two things a page does to one — rewrite its elements, and press one.
 *
 * The rules are `$lib/layoutView.ts`'s; this is where they meet the socket.
 */

import { browser } from '$app/environment';
import { get, writable } from 'svelte/store';

import type { Layout, LayoutElement } from '$lib/generated/index.js';
import { pressOf } from '$lib/layoutView.js';
import { pressTarget } from '$lib/surface.js';
import { addToast } from '$lib/toasts.js';
import { consoleNow } from '$lib/ws/clock.js';
import { beginGesture, endGesture } from './gesture.js';
import { collection, showClient, showData } from './show.js';
import { select } from './selection.js';
import { surfaceShow } from './surface.js';

export const layouts = collection('layouts');

/** The row as it stands, or `undefined` when it has gone. */
export const currentLayout = (id: string): Layout | undefined => get(layouts).find((l) => l.id === id);

/**
 * Rewrite a layout's elements: the whole array, as one write.
 *
 * `gesture` opens one around it, so an edit from the Inspector is one Ctrl-Z. A drag
 * passes `false`, because the drag *is* the gesture — opened at pointer-down — and one
 * per frame would make a two-second drag a hundred acts.
 */
export async function writeElements(layoutId: string, elements: LayoutElement[], gesture = true): Promise<void> {
	if (gesture) beginGesture();
	try {
		await showData().layouts.byId(layoutId).elements.set(elements);
	} catch (e) {
		addToast(e instanceof Error ? e.message : String(e));
	} finally {
		if (gesture) endGesture();
	}
}

/** One element of a layout changed by `change`, in a gesture of its own. */
export async function editElements(
	layoutId: string,
	change: (elements: LayoutElement[]) => LayoutElement[]
): Promise<void> {
	const layout = currentLayout(layoutId);
	if (!layout) return;
	const next = change(layout.elements);
	if (next !== layout.elements) await writeElements(layoutId, next);
}

/**
 * Press an element, on the desk: its variable or its timeline through `pressTarget` —
 * the surface's own path, so unattributed and stamped exactly as a phone's tile is —
 * or its fixture made the selection. Nothing on the shared page calls this.
 */
export async function pressElement(element: LayoutElement): Promise<boolean> {
	const press = pressOf(element);
	if (!press) return false;
	if (press.kind === 'select') {
		select(press.fixtureId);
		return true;
	}
	try {
		await pressTarget(press.target, get(surfaceShow), showData(), showClient(), consoleNow());
	} catch (e) {
		addToast(e instanceof Error ? e.message : String(e));
	}
	return true;
}

// ── Which layout this browser is looking at ───────────────────────────────────

const SHOWN_KEY = 'pult.layout.shown';

function remembered(): string | null {
	if (!browser) return null;
	try {
		return localStorage.getItem(SHOWN_KEY);
	} catch {
		return null;
	}
}

/**
 * The layout the last panel was pointed at. Per browser, like which timeline is open:
 * a panel starts on it and then keeps its own choice, so two `layout` tiles can show two
 * places, and a reload brings back the last one picked.
 */
export const lastShownLayout = writable<string | null>(remembered());

lastShownLayout.subscribe((id) => {
	if (!browser) return;
	try {
		if (id) localStorage.setItem(SHOWN_KEY, id);
		else localStorage.removeItem(SHOWN_KEY);
	} catch {
		// Storage refused: the choice lasts as long as the page.
	}
});
