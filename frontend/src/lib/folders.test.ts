import { describe, expect, it } from 'vitest';

import type { Folder } from './generated/index.js';
import { buildTree, byFolder, canMoveFolder, isWithin, countIn, dissolve, flatten, folderPath } from './folders.js';

const folder = (id: string, name: string, parent: string | null = null, kind = 'timeline'): Folder => ({
	id,
	name,
	parent,
	kind
});

const ALL: Folder[] = [
	folder('act1', 'Act 1'),
	folder('scares', 'Scares', 'act1'),
	folder('act2', 'Act 2'),
	folder('p', 'Warm', null, 'preset')
];

describe('a pool as a tree', () => {
	it('puts each object in its folder, folders by name, and the rest at the top', () => {
		const rows = [
			{ id: 'a', folder: 'scares' },
			{ id: 'b', folder: null },
			{ id: 'c', folder: 'act1' },
			{ id: 'd' }
		];
		const tree = buildTree(ALL, 'timeline', rows);
		expect(tree.folders.map((n) => n.folder.name)).toEqual(['Act 1', 'Act 2']);
		expect(tree.rows.map((r) => r.id)).toEqual(['b', 'd']);
		const act1 = tree.folders[0];
		expect(act1.rows.map((r) => r.id)).toEqual(['c']);
		expect(act1.folders[0].rows.map((r) => r.id)).toEqual(['a']);
		expect(countIn(act1)).toBe(2);
	});

	it('never hides an object because its folder is gone or of another kind', () => {
		const tree = buildTree(ALL, 'timeline', [
			{ id: 'x', folder: 'deleted-on-a-peer' },
			{ id: 'y', folder: 'p' }
		]);
		expect(tree.rows.map((r) => r.id)).toEqual(['x', 'y']);
	});

	it('draws a folder whose parents loop at the top rather than nowhere', () => {
		const looped = [folder('a', 'A', 'b'), folder('b', 'B', 'a')];
		const tree = buildTree(looped, 'timeline', []);
		expect(tree.folders.map((n) => n.folder.id).sort()).toEqual(['a', 'b']);
	});
});

describe('moving and deleting', () => {
	it('will not put a folder inside itself or anything in it, or in another kind', () => {
		expect(canMoveFolder(ALL, 'act1', 'scares')).toBe(false);
		expect(canMoveFolder(ALL, 'act1', 'act1')).toBe(false);
		expect(canMoveFolder(ALL, 'scares', 'act2')).toBe(true);
		expect(canMoveFolder(ALL, 'scares', null)).toBe(true);
		expect(canMoveFolder(ALL, 'scares', 'p')).toBe(false);
	});

	it('deleting a folder moves what is in it up, and deletes nothing in it', () => {
		const plan = dissolve(ALL, 'act1', [
			{ id: 'a', folder: 'act1' },
			{ id: 'b', folder: 'scares' }
		]);
		expect(plan).toEqual({ parent: null, rows: ['a'], folders: ['scares'] });
		expect(dissolve(ALL, 'scares', [{ id: 'b', folder: 'scares' }]).parent).toBe('act1');
	});
});

describe('where a folder is', () => {
	it('reads as a path, and indents in a picker', () => {
		expect(folderPath(ALL, 'scares')).toBe('Act 1 / Scares');
		expect(folderPath(ALL, null)).toBe('');
		expect(isWithin(ALL, 'scares', 'act1')).toBe(true);
		expect(isWithin(ALL, 'act1', 'scares')).toBe(false);
		expect(isWithin(ALL, null, 'act1')).toBe(false);
		expect(flatten(ALL, 'timeline').map((f) => [f.folder.name, f.depth])).toEqual([
			['Act 1', 0],
			['Scares', 1],
			['Act 2', 0]
		]);
	});
});

describe('a list too narrow for the tree', () => {
	it('groups by path, the top level first', () => {
		const groups = byFolder([{ id: 'a', folder: 'act2' }, { id: 'b' }, { id: 'c', folder: 'scares' }], ALL);
		expect(groups.map((g) => [g.path, g.rows.map((r) => r.id)])).toEqual([
			['', ['b']],
			['Act 1 / Scares', ['c']],
			['Act 2', ['a']]
		]);
	});
});
