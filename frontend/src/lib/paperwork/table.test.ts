/**
 * What a table cell and a total say — and above all what they refuse to say.
 *
 * The arithmetic is the station's and is tested there. What is tested here is the
 * *wording*, because the browser is the half that turns a `Totals` into text and the
 * one thing it must never do is print a bare number where the station would have
 * printed a `≥`. The strings below are the same strings
 * `crates/pult-schema/src/types/paperwork/tests.rs` asserts.
 */

import { describe, expect, it } from 'vitest';

import type { Totals } from '../generated/index.js';
import { toCsv } from './csv.js';
import { cellText, powerLabel, weightLabel } from './table.js';

const NOTHING: Totals = {
	items: 0,
	weight_kg: 0,
	weight_known: 0,
	weight_unknown: 0,
	weight_nominal: false,
	power_w: 0,
	power_known: 0,
	power_unknown: 0
};

describe('a weight total', () => {
	it('is stated flatly when everything was counted', () => {
		expect(weightLabel({ ...NOTHING, items: 2, weight_kg: 50, weight_known: 2 })).toBe('50.0 kg');
	});

	it('is a floor when something was not', () => {
		expect(
			weightLabel({ ...NOTHING, items: 3, weight_kg: 50, weight_known: 2, weight_unknown: 1 })
		).toBe('≥ 50.0 kg');
	});

	it('says so when it rests on the catalogue', () => {
		expect(
			weightLabel({ ...NOTHING, items: 2, weight_kg: 53, weight_known: 2, weight_nominal: true })
		).toBe('53.0 kg nominal');
	});

	it('is a dash and never a zero when nothing had a figure', () => {
		// Zero would read as a rig that weighs nothing, which is the one thing a
		// loading table must not be able to say.
		expect(weightLabel({ ...NOTHING, items: 4, weight_unknown: 4 })).toBe('—');
	});

	it('drops the decimal above a hundred kilograms, where it is noise', () => {
		expect(weightLabel({ ...NOTHING, items: 1, weight_kg: 412.4, weight_known: 1 })).toBe(
			'412 kg'
		);
	});
});

describe('a power total', () => {
	it('goes to kilowatts above a thousand', () => {
		expect(powerLabel({ ...NOTHING, power_w: 1030, power_known: 4 })).toBe('1.03 k W');
	});

	it('is a floor when a type did not say what it draws', () => {
		expect(powerLabel({ ...NOTHING, power_w: 940, power_known: 2, power_unknown: 1 })).toBe(
			'≥ 940 W'
		);
	});
});

describe('a cell', () => {
	it('prints a whole number without a decimal', () => {
		expect(cellText({ type: 'Number', value: 12 })).toBe('12');
	});

	it('prints a fraction to one place', () => {
		expect(cellText({ type: 'Number', value: 13.53 })).toBe('13.5');
	});

	it('prints an em dash for nothing, so a gap is visibly a gap', () => {
		expect(cellText({ type: 'Blank' })).toBe('—');
	});
});

describe('the CSV', () => {
	const table = {
		kind: 'Loading' as const,
		columns: [
			{ title: 'Item', numeric: false },
			{ title: 'kg', numeric: true }
		],
		groups: [
			{
				name: 'Front truss',
				rows: [
					[
						{ type: 'Text' as const, value: 'Spot 1' },
						{ type: 'Number' as const, value: 20 }
					],
					[
						{ type: 'Text' as const, value: 'Tube 1' },
						{ type: 'Blank' as const }
					]
				],
				totals: { ...NOTHING, items: 2, weight_kg: 20, weight_known: 1, weight_unknown: 1 }
			}
		],
		totals: { ...NOTHING, items: 2, weight_kg: 20, weight_known: 1, weight_unknown: 1 },
		notes: ['1 item has no weight: Tube 1']
	};

	it('carries the group as a column a spreadsheet can pivot on', () => {
		const csv = toCsv(table);
		expect(csv.split('\r\n')[0]).toBe('Group,Item,kg');
		expect(csv).toContain('Front truss,Spot 1,20');
	});

	it('carries the notes, or a SUM over the column becomes a total nobody checked', () => {
		const csv = toCsv(table);
		expect(csv).toContain('# 1 item has no weight: Tube 1');
	});

	it('carries the same floor the drawing does', () => {
		expect(toCsv(table)).toContain('≥ 20.0 kg');
	});

	it('quotes a fixture name with a comma in it', () => {
		const csv = toCsv({
			...table,
			groups: [
				{
					name: '',
					rows: [
						[
							{ type: 'Text' as const, value: 'Spot 1, house left' },
							{ type: 'Number' as const, value: 1 }
						]
					],
					totals: NOTHING
				}
			]
		});
		expect(csv).toContain('"Spot 1, house left"');
	});
});
