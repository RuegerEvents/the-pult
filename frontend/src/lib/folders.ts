/**
 * Folders in the pools, as arithmetic: the tree a pool draws, the path a palette
 * search reads, which moves are allowed, and what deleting one comes to.
 *
 * A folder is a `folders` row holding one kind of object, with a parent; an object
 * names its folder (`pult_schema::types::folder`). Pure, so the rules are tested
 * without a pool on screen: `stores/folders.ts` is the half that writes.
 */

import type { Folder } from './generated/index.js';

/** What a pool row has to carry for a folder to hold it. */
export type Filed = { id: string; folder?: string | null };

/** One folder in a drawn pool: the folders inside it, then the objects. */
export type FolderNode<Row extends Filed> = { folder: Folder; folders: FolderNode<Row>[]; rows: Row[] };

/** A pool as drawn: its top-level folders, then the objects in no folder. */
export type Tree<Row extends Filed> = { folders: FolderNode<Row>[]; rows: Row[] };

/** The folders of one kind, which are the only ones its objects can be in. */
export const foldersOf = (folders: Folder[], kind: string): Folder[] => folders.filter((f) => f.kind === kind);

/**
 * The pool as a tree. Objects keep the order they came in, which is the pool's own;
 * folders sort by name. An object whose folder is not here — deleted on a peer, or of
 * another kind — is at the top level, and so is a folder whose parent is not here or
 * whose parents loop back to it: nothing is ever lost from view because its folder is.
 */
export function buildTree<Row extends Filed>(all: Folder[], kind: string, rows: Row[]): Tree<Row> {
	const folders = foldersOf(all, kind);
	const byId = new Map(folders.map((f) => [f.id, f]));
	const nodes = new Map<string, FolderNode<Row>>(folders.map((f) => [f.id, { folder: f, folders: [], rows: [] }]));
	const top: Tree<Row> = { folders: [], rows: [] };
	for (const f of [...folders].sort((a, b) => a.name.localeCompare(b.name))) {
		const node = nodes.get(f.id)!;
		const parent = f.parent && !loops(byId, f.id) ? nodes.get(f.parent) : undefined;
		(parent ? parent.folders : top.folders).push(node);
	}
	for (const row of rows) {
		const node = row.folder ? nodes.get(row.folder) : undefined;
		(node ? node.rows : top.rows).push(row);
	}
	return top;
}

/** Whether walking up from `id` comes back to it. */
function loops(byId: Map<string, Folder>, id: string): boolean {
	const seen = new Set<string>();
	let at: string | null | undefined = id;
	while (at) {
		if (seen.has(at)) return true;
		seen.add(at);
		at = byId.get(at)?.parent;
	}
	return false;
}

/** How many objects a folder holds, its folders' included. */
export function countIn<Row extends Filed>(node: FolderNode<Row>): number {
	return node.rows.length + node.folders.reduce((n, f) => n + countIn(f), 0);
}

/** Whether something filed in `folder` is in `ancestor`, directly or further down. */
export function isWithin(all: Folder[], folder: string | null | undefined, ancestor: string): boolean {
	const byId = new Map(all.map((f) => [f.id, f]));
	const seen = new Set<string>();
	let at = folder ?? null;
	while (at && !seen.has(at)) {
		if (at === ancestor) return true;
		seen.add(at);
		at = byId.get(at)?.parent ?? null;
	}
	return false;
}

/** "Act 1 / Scares": where a folder is, for a palette search and a menu. Empty for none. */
export function folderPath(all: Folder[], id: string | null | undefined): string {
	const byId = new Map(all.map((f) => [f.id, f]));
	const names: string[] = [];
	const seen = new Set<string>();
	let at = id ?? null;
	while (at && !seen.has(at)) {
		seen.add(at);
		const f = byId.get(at);
		if (!f) break;
		names.unshift(f.name);
		at = f.parent ?? null;
	}
	return names.join(' / ');
}

/**
 * Whether a folder may go under `parent`: not under itself or anything inside it, and
 * only among folders of its own kind. `null` is the top level, which is always fine.
 */
export function canMoveFolder(all: Folder[], id: string, parent: string | null): boolean {
	if (parent === null) return true;
	if (parent === id) return false;
	const byId = new Map(all.map((f) => [f.id, f]));
	const moving = byId.get(id);
	const target = byId.get(parent);
	if (!moving || !target || moving.kind !== target.kind) return false;
	const seen = new Set<string>();
	let at: string | null | undefined = parent;
	while (at && !seen.has(at)) {
		if (at === id) return false;
		seen.add(at);
		at = byId.get(at)?.parent;
	}
	return true;
}

/**
 * What deleting a folder comes to: everything in it moves up to its parent — the
 * objects and the folders alike — and then the folder goes. Nothing in it is deleted.
 */
export function dissolve<Row extends Filed>(
	all: Folder[],
	id: string,
	rows: Row[]
): { parent: string | null; rows: string[]; folders: string[] } {
	const folder = all.find((f) => f.id === id);
	return {
		parent: folder?.parent ?? null,
		rows: rows.filter((r) => r.folder === id).map((r) => r.id),
		folders: all.filter((f) => f.parent === id).map((f) => f.id)
	};
}

/**
 * The objects grouped by where they are filed, top level first and then by path, for a
 * list too narrow for the tree: a `<select>`'s option groups. Order within a group is
 * the pool's own.
 */
export function byFolder<Row extends Filed>(rows: Row[], all: Folder[]): { path: string; rows: Row[] }[] {
	const groups = new Map<string, Row[]>();
	for (const row of rows) {
		const path = folderPath(all, row.folder);
		groups.set(path, [...(groups.get(path) ?? []), row]);
	}
	return [...groups.entries()]
		.sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b)))
		.map(([path, rows]) => ({ path, rows }));
}

/** The folders of a kind as an indented list, for a picker: depth-first, by name. */
export function flatten(all: Folder[], kind: string): { folder: Folder; depth: number; path: string }[] {
	const out: { folder: Folder; depth: number; path: string }[] = [];
	const walk = (nodes: FolderNode<Filed>[], depth: number) => {
		for (const node of nodes) {
			out.push({ folder: node.folder, depth, path: folderPath(all, node.folder.id) });
			walk(node.folders, depth + 1);
		}
	};
	walk(buildTree(all, kind, []).folders, 0);
	return out;
}
