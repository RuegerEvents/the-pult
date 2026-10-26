/**
 * Objects, and the one grammar every panel speaks about them.
 *
 * A group, a preset, an effect, a surface: each is a thing somebody recalls, paints,
 * renames, duplicates and deletes, and until this existed each panel had its own idea
 * of which click did which. Now there is one:
 *
 * - **Tap acts** — recall, apply, Go, play.
 * - **Select** (⌘/Ctrl-click, or the corner dot on a touch screen or a pen) makes it
 *   the *subject*, and the Inspector panel edits it, whichever panel it lives in.
 * - **Long-press or right-click** opens one context menu, with the same verbs in the
 *   same words everywhere.
 * - **Drag it onto something** to assign it there.
 * - **Drag sideways** on a card that has a level to set the level.
 * - **⌘K** finds any of them by name.
 *
 * A panel *registers* each kind it owns — how to list its rows, what tapping one
 * does, what its verbs are, which component inspects it — and the card, the context
 * menu, the Inspector and the palette ask the registry. None of them knows what a
 * preset is, which is the point: a new kind is one `registerKind` beside the panel
 * that owns it, and nothing enumerates kinds anywhere else.
 *
 * The subject is this browser's, like the fixture selection and for the same reason:
 * two operators at two screens are looking at two different things. It is *not* the
 * fixture selection, which is a question about the rig (`stores/selection.ts`); this
 * is which pool object is being edited.
 */

import type { Component } from 'svelte';
import { derived, get, writable, type Readable } from 'svelte/store';

import type { Category } from '$lib/layout/categories.js';

/** Which object: its kind's name and its id. */
export type ObjectRef = { kind: string; id: string };

export const sameRef = (a: ObjectRef, b: ObjectRef): boolean => a.kind === b.kind && a.id === b.id;

/**
 * Something a context menu offers. `structural` verbs change what the show *is* —
 * rename, color, duplicate, delete, store — and the Show lock hides them; the others
 * are acts and stay.
 */
export type Verb = {
	id: string;
	label: string;
	structural: boolean;
	danger?: boolean;
	/** Greyed out rather than hidden, with the reason as its title. */
	disabled?: string | false;
	run: () => unknown;
};

/** A row any kind's list hands out: at least an id and a name. */
export type ObjectRow = { id: string; name: string; color?: string | null; folder?: string | null };

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type ObjectKind<Row extends ObjectRow = any> = {
	/** The registry key, e.g. `"preset"`. */
	kind: string;
	/** One of them, in the menu's words: "Preset". */
	label: string;
	category: Category;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	icon: Component<any>;
	/** Every row of this kind, live. */
	rows: Readable<Row[]>;
	/** What a tap does, in a word for the palette and the menu ("Recall", "Apply"). */
	actLabel?: string;
	act?: (row: Row) => unknown;
	rename?: (row: Row, name: string) => Promise<unknown>;
	recolor?: (row: Row, color: string | null) => Promise<unknown>;
	duplicate?: (row: Row) => Promise<unknown>;
	remove?: (row: Row) => Promise<unknown>;
	/** Verbs of the kind's own, beyond the shared ones (Update, Show in the sheet…). */
	verbs?: (row: Row) => Verb[];
	/** Edits one row, or several of this kind at once. */
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	inspector?: Component<any>;
	/** Where it lives, so "go to" can bring that panel forward. */
	panel?: string;
	/**
	 * The show collection its rows are, where they are one — which is what lets it be
	 * put in a folder (`stores/folders.ts` writes the row's `folder`). Left out for a
	 * kind whose rows live inside another object, like a timeline's clips.
	 */
	table?: string;
};

const registry = writable<Map<string, ObjectKind>>(new Map());

/** Every kind registered right now. */
export const objectKinds: Readable<Map<string, ObjectKind>> = { subscribe: registry.subscribe };

/** Add a kind, replacing one of the same name. Answers a function that takes it away. */
export function registerKind<Row extends ObjectRow>(kind: ObjectKind<Row>): () => void {
	registry.update((all) => new Map(all).set(kind.kind, kind as ObjectKind));
	return () =>
		registry.update((all) => {
			if (all.get(kind.kind) !== (kind as ObjectKind)) return all;
			const next = new Map(all);
			next.delete(kind.kind);
			return next;
		});
}

export const kindOf = (kind: string): ObjectKind | undefined => get(registry).get(kind);

/** The live row behind a ref, or undefined when it has gone. */
export function rowOf(ref: ObjectRef): ObjectRow | undefined {
	const kind = kindOf(ref.kind);
	return kind ? get(kind.rows).find((r) => r.id === ref.id) : undefined;
}

// ── The shared verbs ──────────────────────────────────────────────────────────

/** Set from the menu's Rename, read by the card that should become a text field. */
export const renaming = writable<ObjectRef | null>(null);

/** Set from the menu's Move to folder…, read by the picker at the root of the app. */
export const moving = writable<ObjectRef | null>(null);

/**
 * The menu for one object: the kind's own verbs first, then the shared ones in a
 * fixed order, so Delete is always last and always in the same place under a thumb.
 * Structural verbs are left out when `locked`.
 */
export function verbsFor(ref: ObjectRef, locked: boolean): Verb[] {
	const kind = kindOf(ref.kind);
	const row = rowOf(ref);
	if (!kind || !row) return [];
	const verbs: Verb[] = [];
	if (kind.act) {
		verbs.push({ id: 'act', label: kind.actLabel ?? 'Use', structural: false, run: () => kind.act!(row) });
	}
	verbs.push({ id: 'inspect', label: 'Inspect', structural: false, run: () => selectSubject(ref) });
	verbs.push(...(kind.verbs?.(row) ?? []));
	if (kind.rename) {
		verbs.push({ id: 'rename', label: 'Rename', structural: true, run: () => renaming.set(ref) });
	}
	if (kind.duplicate) {
		verbs.push({ id: 'duplicate', label: 'Duplicate', structural: true, run: () => kind.duplicate!(row) });
	}
	if (kind.table) {
		verbs.push({ id: 'move', label: 'Move to folder…', structural: true, run: () => moving.set(ref) });
	}
	if (kind.remove) {
		verbs.push({ id: 'delete', label: 'Delete', structural: true, danger: true, run: () => kind.remove!(row) });
	}
	return locked ? verbs.filter((v) => !v.structural) : verbs;
}

/** The swatches the Color verb offers. `null` is "the category's own". */
export const SWATCHES: (string | null)[] = [
	null,
	'#ef4444',
	'#f97316',
	'#eab308',
	'#22c55e',
	'#14b8a6',
	'#3b82f6',
	'#8b5cf6',
	'#ec4899',
	'#a3a3a3'
];

// ── The subject ───────────────────────────────────────────────────────────────

/** What the Inspector is editing. Several, when they were ⌘-clicked together. */
export const subject = writable<ObjectRef[]>([]);

/** Make `ref` the subject, or with `add` put it beside (or take it out of) the rest. */
export function selectSubject(ref: ObjectRef, add = false): void {
	subject.update((current) => {
		if (!add) return [ref];
		return current.some((r) => sameRef(r, ref))
			? current.filter((r) => !sameRef(r, ref))
			: [...current, ref];
	});
}

export const clearSubject = () => subject.set([]);

export const isSubject = (refs: ObjectRef[], ref: ObjectRef) => refs.some((r) => sameRef(r, ref));

/**
 * The subject grouped by kind, dropping refs whose rows have gone (deleted here or
 * by a peer). What the Inspector draws from.
 */
export type SubjectGroup = { kind: ObjectKind; rows: ObjectRow[] };

export const subjectRows: Readable<SubjectGroup[]> = derived(
	[subject, registry],
	([$subject, $registry], set: (value: SubjectGroup[]) => void) => {
		// Every kind's rows are stores of their own; follow the ones the subject names.
		const kinds = [...new Set($subject.map((r) => r.kind))]
			.map((k) => $registry.get(k))
			.filter((k): k is ObjectKind => !!k);
		if (kinds.length === 0) {
			set([]);
			return;
		}
		return derived(
			kinds.map((k) => k.rows) as Readable<ObjectRow[]>[],
			(lists: ObjectRow[][]): SubjectGroup[] =>
				kinds
					.map((kind, i) => ({
						kind,
						rows: lists[i].filter((row: ObjectRow) =>
							$subject.some((r) => r.kind === kind.kind && r.id === row.id)
						)
					}))
					.filter((group) => group.rows.length > 0)
		).subscribe(set);
	},
	[] as SubjectGroup[]
);

// ── Dragging one onto something ───────────────────────────────────────────────

/** The data transfer type a dragged card carries. */
export const OBJECT_MIME = 'application/x-pult-object';

export function readDropped(event: DragEvent): ObjectRef | null {
	const text = event.dataTransfer?.getData(OBJECT_MIME);
	if (!text) return null;
	try {
		const ref = JSON.parse(text) as ObjectRef;
		return typeof ref?.kind === 'string' && typeof ref?.id === 'string' ? ref : null;
	} catch {
		return null;
	}
}

/** "Brake copy", "Brake copy 2"… — a name nothing in `taken` already has. */
export function copyName(name: string, taken: string[]): string {
	const base = `${name} copy`;
	if (!taken.includes(base)) return base;
	for (let n = 2; ; n++) if (!taken.includes(`${base} ${n}`)) return `${base} ${n}`;
}

// ── The context menu ──────────────────────────────────────────────────────────

/** The one open context menu, if any: whose, and where on the screen. */
export const menu = writable<{ ref: ObjectRef; x: number; y: number } | null>(null);

export function openMenu(ref: ObjectRef, x: number, y: number): void {
	menu.set({ ref, x, y });
}

export const closeMenu = () => menu.set(null);
