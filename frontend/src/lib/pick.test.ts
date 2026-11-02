import { describe, expect, it } from 'vitest';

import type { Folder } from './generated/index.js';
import { fold, matches, pickGroups, pickGroupsBy } from './pick.js';

const folder = (id: string, name: string, parent: string | null = null): Folder =>
	({ id, name, parent, kind: 'sequence' }) as Folder;

describe('matches', () => {
	it('takes every word, in any order, from the name or the folder', () => {
		expect(matches('scare act', 'Clown scare', 'Act 2')).toBe(true);
		expect(matches('scare act 3', 'Clown scare', 'Act 2')).toBe(false);
	});

	it('ignores case and accents', () => {
		expect(matches('aussen', 'Außenszenerie', '')).toBe(true);
		expect(fold('Éclairage')).toBe('eclairage');
	});

	it('lets everything through for nothing typed', () => {
		expect(matches('  ', 'Anything', '')).toBe(true);
	});
});

describe('pickGroups', () => {
	const folders = [folder('a', 'Act 1'), folder('b', 'Scares', 'a')];
	const rows = [
		{ id: '1', name: 'House', folder: null },
		{ id: '2', name: 'Clowns', folder: 'b' },
		{ id: '3', name: 'Donkey', folder: 'a' }
	];
	const label = (row: { name: string }) => row.name;

	it('puts the unfiled first and each folder under its path', () => {
		const groups = pickGroups(rows, folders, '', label);
		expect(groups.map((g) => g.path)).toEqual(['', 'Act 1', 'Act 1 / Scares']);
	});

	it('drops a folder with nothing in it that matches', () => {
		const groups = pickGroups(rows, folders, 'scares', label);
		expect(groups).toEqual([{ path: 'Act 1 / Scares', rows: [rows[1]] }]);
	});
});

describe('pickGroupsBy', () => {
	it('groups by what it is told, in the order the rows came, and searches the group too', () => {
		const cues = [
			{ id: '1', name: 'Blackout', seq: 'Pumpkin' },
			{ id: '2', name: 'Scare', seq: 'Clowns' },
			{ id: '3', name: 'Reset', seq: 'Pumpkin' }
		];
		const all = pickGroupsBy(cues, '', (c) => c.name, (c) => c.seq);
		expect(all.map((g) => [g.path, g.rows.map((r) => r.id)])).toEqual([
			['Pumpkin', ['1', '3']],
			['Clowns', ['2']]
		]);
		expect(pickGroupsBy(cues, 'clowns', (c) => c.name, (c) => c.seq).map((g) => g.path)).toEqual(['Clowns']);
	});
});
