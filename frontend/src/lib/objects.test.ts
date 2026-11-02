import { describe, expect, it } from 'vitest';
import { get, writable } from 'svelte/store';

import {
	copyName,
	registerKind,
	renaming,
	selectSubject,
	subject,
	subjectRows,
	verbsFor,
	type ObjectRow
} from './objects.js';

const stub = {} as never;

function kind(name: string, rows: ObjectRow[]) {
	const store = writable(rows);
	const calls: string[] = [];
	const off = registerKind({
		kind: name,
		label: name,
		category: 'program',
		icon: stub,
		rows: store,
		act: (row) => calls.push(`act ${row.id}`),
		rename: async (row, to) => void calls.push(`rename ${row.id} ${to}`),
		remove: async (row) => void calls.push(`remove ${row.id}`),
		verbs: () => [{ id: 'show', label: 'Show', structural: false, run: () => calls.push('show') }]
	});
	return { store, calls, off };
}

describe('the context menu verbs', () => {
	it('lists acts first, the kind’s own next, and Delete last', () => {
		const { off } = kind('thing', [{ id: 'a', name: 'A' }]);
		const ids = verbsFor({ kind: 'thing', id: 'a' }, false).map((v) => v.id);
		expect(ids[0]).toBe('act');
		expect(ids).toContain('show');
		expect(ids.at(-1)).toBe('delete');
		off();
	});

	it('offers only acts under the Show lock', () => {
		const { off } = kind('thing', [{ id: 'a', name: 'A' }]);
		const verbs = verbsFor({ kind: 'thing', id: 'a' }, true);
		expect(verbs.every((v) => !v.structural)).toBe(true);
		expect(verbs.map((v) => v.id)).not.toContain('delete');
		expect(verbs.map((v) => v.id)).not.toContain('rename');
		off();
	});

	it('Rename puts the card into renaming rather than asking anywhere else', () => {
		const { off } = kind('thing', [{ id: 'a', name: 'A' }]);
		verbsFor({ kind: 'thing', id: 'a' }, false).find((v) => v.id === 'rename')!.run();
		expect(get(renaming)).toEqual({ kind: 'thing', id: 'a' });
		renaming.set(null);
		off();
	});

	it('has nothing to offer for an object that has gone', () => {
		const { off } = kind('thing', [{ id: 'a', name: 'A' }]);
		expect(verbsFor({ kind: 'thing', id: 'gone' }, false)).toEqual([]);
		off();
	});
});

describe('the subject', () => {
	it('is one object, or several with ⌘', () => {
		selectSubject({ kind: 'k', id: '1' });
		selectSubject({ kind: 'k', id: '2' }, true);
		expect(get(subject)).toHaveLength(2);
		selectSubject({ kind: 'k', id: '1' }, true);
		expect(get(subject)).toEqual([{ kind: 'k', id: '2' }]);
		selectSubject({ kind: 'k', id: '3' });
		expect(get(subject)).toEqual([{ kind: 'k', id: '3' }]);
	});

	it('groups by kind and forgets rows that were deleted', () => {
		const a = kind('alpha', [{ id: '1', name: 'one' }, { id: '2', name: 'two' }]);
		const b = kind('beta', [{ id: '9', name: 'nine' }]);
		subject.set([
			{ kind: 'alpha', id: '1' },
			{ kind: 'alpha', id: '2' },
			{ kind: 'beta', id: '9' }
		]);
		let groups = get(subjectRows);
		expect(groups.map((g) => [g.kind.kind, g.rows.length])).toEqual([['alpha', 2], ['beta', 1]]);
		a.store.set([{ id: '1', name: 'one' }]);
		groups = get(subjectRows);
		expect(groups[0].rows.map((r) => r.id)).toEqual(['1']);
		a.off();
		b.off();
		subject.set([]);
	});
});

describe('naming a copy', () => {
	it('counts up past the copies that already exist', () => {
		expect(copyName('Fire', ['Fire'])).toBe('Fire copy');
		expect(copyName('Fire', ['Fire', 'Fire copy', 'Fire copy 2'])).toBe('Fire copy 3');
	});
});
