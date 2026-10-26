/**
 * A folder, as an object: renamed, deleted and moved through the one context menu
 * every pool object has, and found by ⌘K like the rest. A tap opens or closes it.
 * Deleting one deletes nothing in it — see `stores/folders.ts`.
 */

import FolderIcon from '@lucide/svelte/icons/folder';

import type { Folder } from '$lib/generated/index.js';
import { moving, registerKind } from '$lib/objects.js';
import { deleteFolder, folders, newFolder, renameFolder, toggleFolder } from '$lib/stores/folders.js';

registerKind<Folder>({
	kind: 'folder',
	label: 'Folder',
	category: 'program',
	icon: FolderIcon,
	rows: folders,
	actLabel: 'Open or close',
	act: (folder) => toggleFolder(folder.id),
	rename: (folder, name) => renameFolder(folder.id, name),
	remove: (folder) => deleteFolder(folder.id),
	verbs: (folder) => [
		{ id: 'new-inside', label: 'New folder inside', structural: true, run: () => newFolder(folder.kind, folder.id) },
		{ id: 'move-folder', label: 'Move to folder…', structural: true, run: () => moving.set({ kind: 'folder', id: folder.id }) }
	]
});
