/**
 * A {@link Table} from the station, drawn onto a sheet.
 *
 * The arithmetic is not here and must not be: `pult_schema::types::paperwork` computes
 * every row, subtotal and note, and this draws what it is given. That split is the
 * whole reason the tables live on the station — a plugin, the command line and this
 * renderer all read the same numbers, and none of them can come to a different total.
 *
 * What *is* decided here is how a total is written, and that comes back over the wire
 * as well: `Totals::weight_label` produced the `≥` and the word *nominal*, so a cell
 * saying "≥ 412 kg nominal" says so because the station said so.
 */

import type { Cell, Table, Totals } from '../generated/index.js';
import {
	BLACK,
	WEIGHT,
	stroke,
	type DrawItem,
	type Mm,
	type RectMm
} from './drawing.js';
import type { Metrics } from './font.js';

const ROW: Mm = 4.2;
const TEXT: Mm = 2.4;
const HEAD: Mm = 2.6;
const PAD: Mm = 1.2;

/** What a cell says, in the one place that decides. */
export function cellText(cell: Cell): string {
	if (cell.type === 'Text') return cell.value;
	if (cell.type === 'Number') {
		// Whole numbers plain, everything else to one place: a count of 12 should not
		// read "12.0", and 13.5 kg should not read "14".
		return Number.isInteger(cell.value) ? String(cell.value) : cell.value.toFixed(1);
	}
	// An em dash rather than a blank, so a missing figure is visibly missing and not
	// mistaken for a column that ran off the edge of the paper.
	return '—';
}

/**
 * Column widths: the widest thing in each, then squeezed to fit.
 *
 * Measured against the same metrics the text will be drawn with, which is why they are
 * passed in rather than guessed at — a table laid out against a font the page does not
 * have is a table whose columns overlap on somebody else's machine.
 */
function widths(table: Table, rect: RectMm, metrics: Metrics): Mm[] {
	const wanted = table.columns.map((column, index) => {
		let widest = metrics.widthOf(column.title, HEAD, true);
		for (const group of table.groups) {
			for (const row of group.rows) {
				const cell = row[index];
				if (!cell) continue;
				widest = Math.max(widest, metrics.widthOf(cellText(cell), TEXT));
			}
		}
		return widest + PAD * 2;
	});
	const total = wanted.reduce((a, b) => a + b, 0);
	if (total <= rect.w) return wanted;
	// Everything shrinks in proportion rather than the last column absorbing it.
	// Overrunning cells are then clipped by the block's own rectangle, which is
	// visible; a column silently 3 mm too narrow is not.
	return wanted.map((w) => (w * rect.w) / total);
}

/** Draw a table into its block. */
export function drawTable(
	table: Table | undefined,
	rect: RectMm,
	title: string,
	metrics: Metrics
): DrawItem[] {
	const items: DrawItem[] = [];
	let y = rect.y;

	if (title) {
		items.push({
			kind: 'text',
			at: { x: rect.x, y },
			text: title,
			size: 3.4,
			bold: true,
			baseline: 'top'
		});
		y += 5.5;
	}

	if (!table) {
		items.push({
			kind: 'text',
			at: { x: rect.x, y },
			text: 'This table has not been fetched from the station.',
			size: TEXT,
			color: '#666666',
			baseline: 'top'
		});
		return items;
	}

	const columnWidths = widths(table, rect, metrics);
	const xs: Mm[] = [];
	let x = rect.x;
	for (const w of columnWidths) {
		xs.push(x);
		x += w;
	}
	const right = x;

	const header = () => {
		table.columns.forEach((column, index) => {
			items.push({
				kind: 'text',
				at: {
					x: column.numeric ? xs[index] + columnWidths[index] - PAD : xs[index] + PAD,
					y
				},
				text: column.title,
				size: HEAD,
				bold: true,
				anchor: column.numeric ? 'end' : 'start',
				baseline: 'top'
			});
		});
		y += ROW;
		items.push({
			kind: 'path',
			points: [
				[
					{ x: rect.x, y: y - 1 },
					{ x: right, y: y - 1 }
				]
			],
			stroke: stroke(WEIGHT.thin, BLACK)
		});
	};
	header();

	// Room has to be kept for the notes, which are the part that must not be cut off:
	// a loading table whose "3 items have no weight" line fell off the bottom of the
	// page is exactly the failure the notes exist to prevent.
	const noteHeight = table.notes.length * 3.2 + (table.notes.length ? 2 : 0);
	const bottom = rect.y + rect.h - noteHeight;
	let dropped = 0;

	for (const group of table.groups) {
		if (group.name) {
			if (y + ROW > bottom) {
				dropped += group.rows.length;
				continue;
			}
			items.push({
				kind: 'text',
				at: { x: rect.x + PAD, y },
				text: group.name,
				size: TEXT,
				bold: true,
				baseline: 'top'
			});
			y += ROW;
		}
		for (const row of group.rows) {
			if (y + ROW > bottom) {
				dropped++;
				continue;
			}
			row.forEach((cell, index) => {
				const column = table.columns[index];
				if (!column) return;
				items.push({
					kind: 'text',
					at: {
						x: column.numeric ? xs[index] + columnWidths[index] - PAD : xs[index] + PAD,
						y
					},
					text: cellText(cell),
					size: TEXT,
					anchor: column.numeric ? 'end' : 'start',
					baseline: 'top'
				});
			});
			y += ROW;
		}
		if (group.name && y + ROW <= bottom) {
			// Labelled, not bare. An unlabelled figure reads as a subtotal only when it
			// *is* a figure: a power table where nothing has a wattage printed a lone
			// em-dash floating under each group, which reads as a fault.
			items.push(...totalsRow(group.totals, rect.x, right, y, table.kind, group.name));
			y += ROW + 1;
		}
	}

	// The grand total, always, even when rows were dropped — it is a figure about the
	// whole table and not about the part of it that fitted.
	if (y + ROW <= rect.y + rect.h) {
		items.push({
			kind: 'path',
			points: [
				[
					{ x: rect.x, y: y - 0.5 },
					{ x: right, y: y - 0.5 }
				]
			],
			stroke: stroke(WEIGHT.medium, BLACK)
		});
		items.push(...totalsRow(table.totals, rect.x, right, y + 0.5, table.kind, 'Total'));
		y += ROW + 1;
	}

	if (dropped > 0) {
		items.push({
			kind: 'text',
			at: { x: rect.x, y },
			text: `${dropped} more ${dropped === 1 ? 'row does' : 'rows do'} not fit on this sheet. The totals above count them.`,
			size: 2.2,
			color: '#666666',
			baseline: 'top'
		});
		y += 3.2;
	}

	for (const line of table.notes) {
		items.push({
			kind: 'text',
			at: { x: rect.x, y },
			text: line,
			size: 2.2,
			color: '#666666',
			baseline: 'top'
		});
		y += 3.2;
	}

	return items;
}

/**
 * A subtotal or a grand total, written the way the station wrote it.
 *
 * A power table prints the power label and a loading table the weight one; a patch or
 * counts table prints how many things it counted, which is the figure somebody
 * actually reads off one.
 */
function totalsRow(
	totals: Totals,
	left: Mm,
	right: Mm,
	y: Mm,
	kind: string,
	label = ''
): DrawItem[] {
	const figure =
		kind === 'Power'
			? powerLabel(totals)
			: kind === 'Loading'
				? weightLabel(totals)
				: `${totals.items} ${totals.items === 1 ? 'fixture' : 'fixtures'}`;
	return [
		{
			kind: 'text',
			at: { x: left + PAD, y },
			text: label,
			size: TEXT,
			bold: true,
			baseline: 'top'
		},
		{
			kind: 'text',
			at: { x: right - PAD, y },
			text: figure,
			size: TEXT,
			bold: true,
			anchor: 'end',
			baseline: 'top'
		}
	];
}

/**
 * The weight, spelled the way `Totals::weight_label` spells it.
 *
 * A second implementation of a rule the station already has, which is normally the
 * thing to avoid — and here it is deliberate and narrow: the *wire* carries the
 * numbers, so the browser can only get this wrong in one direction, and the Rust test
 * `a_weight_missing_one_figure_is_a_floor` and this function are checked against each
 * other by `table.test.ts`. What must never happen is a browser that prints a bare
 * number where the station would have printed `≥`.
 */
export function weightLabel(totals: Totals): string {
	if (totals.weight_known === 0) return '—';
	const value = totals.weight_kg >= 100 ? totals.weight_kg.toFixed(0) : totals.weight_kg.toFixed(1);
	return `${totals.weight_unknown > 0 ? '≥ ' : ''}${value} kg${totals.weight_nominal ? ' nominal' : ''}`;
}

/** The same, for power. */
export function powerLabel(totals: Totals): string {
	if (totals.power_known === 0) return '—';
	const value =
		totals.power_w >= 1000 ? `${(totals.power_w / 1000).toFixed(2)} k` : totals.power_w.toFixed(0);
	return `${totals.power_unknown > 0 ? '≥ ' : ''}${value} W`;
}
