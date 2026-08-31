import { describe, expect, it } from 'vitest';
import { describeChange, pluginDatumName } from './users.js';
import type { HistoryEntry } from './generated/index.js';

const entry = (path: string[]): HistoryEntry => ({
	id: 'op',
	user_id: 'someone',
	path,
	at: new Date().toISOString(),
	undoes: null,
	undoable: true
});

describe('describeChange', () => {
	it('names the ids it knows and shortens the ones it does not', () => {
		const id = '2f6b535b-9a71-4c39-9d95-6d6ab2f0f639';
		const names = new Map([[id, 'Spot 3']]);

		expect(describeChange(entry(['fixtures', id, 'name']), names)).toBe(
			'fixtures → Spot 3 → name'
		);
		// A thing deleted since keeps its short id, which is honest: it is gone
		// and there is nothing left to call it.
		expect(describeChange(entry(['fixtures', id, 'name']))).toBe('fixtures → 2f6b53 → name');
	});
});

describe('pluginDatumName', () => {
	it('says which plugin, which store and which key', () => {
		expect(
			pluginDatumName({ plugin_id: 'macros', store: 'saved', key: 'opening' })
		).toBe('macros · saved · opening');
	});

	it('gives a store row something to be called in the history', () => {
		// The row id is a hash of what it names, so without a name this entry is
		// the one line in the list an operator cannot act on.
		const id = '11111111-2222-3333-4444-555555555555';
		const names = new Map([
			[id, pluginDatumName({ plugin_id: 'macros', store: 'saved', key: 'opening' })]
		]);

		expect(describeChange(entry(['plugin_data', id, 'value']), names)).toBe(
			'plugin data → macros · saved · opening → value'
		);
		expect(describeChange(entry(['plugin_data', id, 'value']))).toBe(
			'plugin data → 111111 → value'
		);
	});
});
