/**
 * Folders in the pools: the rows, which are open on this screen, and the writes.
 *
 * The arithmetic is `$lib/folders.ts`; this is the half with hands. Every act is one
 * write where it can be — filing an object, renaming or moving a folder — and deleting
 * a folder, which moves everything in it up first, is one gesture, so each is one
 * Ctrl-Z. Which folders are open is this browser's, in `localStorage` beside the
 * layout, because two operators looking at one pool are looking at it differently.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import type { Folder } from '$lib/generated/index.js';
import { dissolve, folderPath } from '$lib/folders.js';
import { kindOf, type ObjectRef, type ObjectRow } from '$lib/objects.js';
import { beginGesture, endGesture } from './gesture.js';
import { collection, showData } from './show.js';
import { uuid } from '$lib/uuid.js';

export const folders = collection('folders') as Readable<Folder[]>;

const OPEN_KEY = 'pult.folders.closed';

function readClosed(): Set<string> {
	try {
		return new Set(JSON.parse(localStorage.getItem(OPEN_KEY) ?? '[]') as string[]);
	} catch {
		return new Set();
	}
}

/** The folders closed on this screen. Open is what a folder is until somebody closes it. */
export const closedFolders = writable<Set<string>>(typeof localStorage !== 'undefined' ? readClosed() : new Set());

export function toggleFolder(id: string): void {
	closedFolders.update((closed) => {
		const next = new Set(closed);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		try {
			localStorage.setItem(OPEN_KEY, JSON.stringify([...next]));
		} catch {
			// A private window: open and closed simply are not remembered.
		}
		return next;
	});
}

/** Where an object's folder is, as a path, for ⌘K. Empty for none. */
export const pathOf: Readable<(folder: string | null | undefined) => string> = derived(
	folders,
	($folders) => (folder: string | null | undefined) => folderPath($folders, folder)
);

/** The collection proxy a kind's rows live in, where they live in one. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function tableOf(kind: string): any {
	const table = kindOf(kind)?.table;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	return table ? (showData() as any)[table] : undefined;
}

/** Put an object in a folder, or at the top level with `null`. One write. */
export async function fileObject(ref: ObjectRef, folder: string | null): Promise<void> {
	const table = tableOf(ref.kind);
	if (!table) return;
	await table.byId(ref.id).folder.set(folder);
}

/** A new folder for `kind`, inside `parent` (or at the top). Answers its id. */
export async function newFolder(kind: string, parent: string | null, name = 'New folder'): Promise<string> {
	const id = uuid();
	await showData().folders.create({ id, name, parent, kind });
	return id;
}

export async function renameFolder(id: string, name: string): Promise<void> {
	await showData().folders.byId(id).name.set(name);
}

/** Move a folder under another of its kind, or to the top. One write. */
export async function moveFolder(id: string, parent: string | null): Promise<void> {
	await showData().folders.byId(id).parent.set(parent);
}

/**
 * Delete a folder and nothing in it: its objects and its folders move up to its
 * parent first, and all of it is one gesture.
 */
export async function deleteFolder(id: string): Promise<void> {
	const all = get(folders);
	const folder = all.find((f) => f.id === id);
	if (!folder) return;
	const kind = kindOf(folder.kind);
	const rows = kind ? (get(kind.rows) as ObjectRow[]) : [];
	const plan = dissolve(all, id, rows);
	beginGesture();
	try {
		for (const row of plan.rows) await fileObject({ kind: folder.kind, id: row }, plan.parent);
		for (const child of plan.folders) await moveFolder(child, plan.parent);
		await showData().folders.byId(id).delete();
	} finally {
		endGesture();
	}
}
