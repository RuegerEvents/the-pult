/**
 * The workspace: which layout is on screen, and what is being done to it.
 *
 * Layouts themselves are show data — a PERSISTED `layouts` collection, so an
 * arrangement made at the tech table is there at the next call and on the console
 * next to it. *Which* one this browser is looking at is not: two operators at two
 * screens plainly want different tiles up, so that lives here and in `localStorage`.
 *
 * Rearranging does not save. A tile dragged somewhere marks the layout dirty and the
 * arrangement is remembered locally, but the show is only written when *Save* is
 * pressed — otherwise a busk on a spare screen would rewrite the layout everyone
 * else is using.
 */

import { browser } from '$app/environment';
import { get, writable } from 'svelte/store';
import type { Layout, LayoutNode } from '$lib/generated/index.js';
import { movePanel, removePanel, resize, addTab, tidy, type DropSide, type Path } from '$lib/layout.js';
import { DEFAULT_PRESET, presetByKey } from '$lib/layout/presets.js';
import { collection, showData } from './show.js';

export type Active = { kind: 'preset'; key: string } | { kind: 'show'; id: string };

const STORAGE_KEY = 'pult.layout';

export const layouts = collection('layouts');

export const tree = writable<LayoutNode>(DEFAULT_PRESET.tree);
export const active = writable<Active>({ kind: 'preset', key: DEFAULT_PRESET.key });
export const dirty = writable(false);
/** One panel filling the workspace, for a moment of needing all the room. */
export const maximised = writable<string | null>(null);

// ── Remembering it locally ────────────────────────────────────────────────────

function remember() {
	if (!browser) return;
	try {
		localStorage.setItem(
			STORAGE_KEY,
			JSON.stringify({ active: get(active), tree: get(tree), dirty: get(dirty) })
		);
	} catch {
		// A browser with storage turned off still works; it just opens on the default.
	}
}

/** Put back what this browser was looking at. Falls back to the first preset. */
export function restoreLayout(): void {
	if (!browser) return;
	try {
		const saved = localStorage.getItem(STORAGE_KEY);
		if (!saved) return;
		const parsed = JSON.parse(saved) as { active?: Active; tree?: LayoutNode; dirty?: boolean };
		if (!parsed?.tree || (parsed.tree.type !== 'Split' && parsed.tree.type !== 'Tabs')) return;
		tree.set(tidy(parsed.tree));
		if (parsed.active) active.set(parsed.active);
		dirty.set(parsed.dirty === true);
	} catch {
		// Anything unreadable is not worth a broken workspace.
	}
}

/** The tree changed by hand: keep it, mark it unsaved. */
function change(next: LayoutNode): void {
	tree.set(next);
	dirty.set(true);
	remember();
}

// ── Choosing one ──────────────────────────────────────────────────────────────

export function applyPreset(key: string): void {
	const preset = presetByKey(key);
	if (!preset) return;
	tree.set(preset.tree);
	active.set({ kind: 'preset', key });
	dirty.set(false);
	maximised.set(null);
	remember();
}

export function applyLayout(layout: Layout): void {
	tree.set(tidy(layout.tree));
	active.set({ kind: 'show', id: layout.id });
	dirty.set(false);
	maximised.set(null);
	remember();
}

export function resetLayout(): void {
	const current = get(active);
	if (current.kind === 'preset') return applyPreset(current.key);
	const layout = get(layouts).find((l) => l.id === current.id);
	if (layout) applyLayout(layout);
	else applyPreset(DEFAULT_PRESET.key);
}

// ── Saving one ────────────────────────────────────────────────────────────────

export async function save(): Promise<void> {
	const current = get(active);
	// A preset is not a place to save to: it is the way back from having saved.
	if (current.kind !== 'show') return;
	await showData().layouts.byId(current.id).tree.set(get(tree));
	dirty.set(false);
	remember();
}

export async function saveAs(name: string): Promise<void> {
	const id = crypto.randomUUID();
	await showData().layouts.create({ id, name, tree: get(tree) });
	active.set({ kind: 'show', id });
	dirty.set(false);
	remember();
}

export async function rename(id: string, name: string): Promise<void> {
	await showData().layouts.byId(id).name.set(name);
}

export async function removeLayout(id: string): Promise<void> {
	await showData().layouts.byId(id).delete();
	const current = get(active);
	if (current.kind === 'show' && current.id === id) applyPreset(DEFAULT_PRESET.key);
}

// ── Rearranging ───────────────────────────────────────────────────────────────

export const openPanel = (path: Path, panel: string) => change(addTab(get(tree), path, panel));
export const closePanel = (path: Path, panel: string) => change(removePanel(get(tree), path, panel));
export const dragGutter = (path: Path, gutter: number, delta: number) =>
	change(resize(get(tree), path, gutter, delta));

// ── Dragging a tab ────────────────────────────────────────────────────────────

export const dragging = writable<{ path: Path; panel: string } | null>(null);
/** The drop zone under the pointer, as `"0.1|left"`. */
export const dropTarget = writable<string | null>(null);

export const dropId = (path: Path, side: DropSide) => `${path.join('.')}|${side}`;

/**
 * Follow a tab being dragged, and drop it where it lands.
 *
 * The zone under the pointer is found with `elementFromPoint` rather than by
 * listening on the zones themselves. A touch drag is implicitly captured by the
 * element it started on, so enter and leave events never reach anything else — one
 * mechanism that works for a mouse and a finger alike is worth more than two that
 * each work for one.
 */
export function beginTabDrag(path: Path, panel: string, event: PointerEvent): void {
	const from = { x: event.clientX, y: event.clientY };
	let started = false;

	const move = (e: PointerEvent) => {
		if (!started) {
			// A few pixels of slack, so clicking a tab selects it rather than moving it.
			if (Math.hypot(e.clientX - from.x, e.clientY - from.y) < 5) return;
			started = true;
			dragging.set({ path, panel });
		}
		dropTarget.set(zoneAt(e.clientX, e.clientY));
	};

	const up = (e: PointerEvent) => {
		window.removeEventListener('pointermove', move);
		window.removeEventListener('pointerup', up);
		const landed = started ? zoneAt(e.clientX, e.clientY) : null;
		dragging.set(null);
		dropTarget.set(null);
		if (!landed) return;
		const [text, side] = landed.split('|');
		const to = text === '' ? [] : text.split('.').map(Number);
		change(movePanel(get(tree), { path, panel }, to, side as DropSide));
	};

	window.addEventListener('pointermove', move);
	window.addEventListener('pointerup', up);
}

function zoneAt(x: number, y: number): string | null {
	return (
		document.elementFromPoint(x, y)?.closest('[data-drop]')?.getAttribute('data-drop') ?? null
	);
}
