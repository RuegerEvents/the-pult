/**
 * A table, as a file somebody can open in a spreadsheet.
 *
 * A rider goes to a supplier and a supplier quotes from a spreadsheet, so a picture of
 * a table is the wrong shape for half of what paperwork is for. The rows are the same
 * rows the PDF draws — both come from `paperwork.tables` — so the two cannot disagree
 * about what the rig weighs.
 *
 * **The notes are in the file.** A CSV of a loading table whose "3 items have no
 * weight" line stayed behind on the PDF is exactly the way a floor becomes a total: a
 * spreadsheet's own SUM over the column would produce a confident number nobody
 * checked. So the notes are written under the rows, prefixed with `#`, and the total
 * row carries the same `≥` the drawing does.
 */

import type { Table } from '../generated/index.js';
import { cellText, powerLabel, weightLabel } from './table.js';

/** One field, quoted where it has to be. */
function field(value: string): string {
	return /[",\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

function row(values: string[]): string {
	return values.map(field).join(',');
}

/**
 * One table as CSV text.
 *
 * A `Group` column rather than group heading rows, because a heading row in a CSV is a
 * row a spreadsheet sorts into the middle of the data. Subtotals are dropped for the
 * same reason and the reader gets a column they can pivot on instead — which is what
 * anybody doing arithmetic on this actually wants.
 */
export function toCsv(table: Table): string {
	const grouped = table.groups.some((group) => group.name.length > 0);
	const lines: string[] = [];
	lines.push(row([...(grouped ? ['Group'] : []), ...table.columns.map((c) => c.title)]));

	for (const group of table.groups) {
		for (const cells of group.rows) {
			lines.push(
				row([...(grouped ? [group.name] : []), ...cells.map((cell) => cellText(cell))])
			);
		}
	}

	lines.push('');
	lines.push(row(['# Total items', String(table.totals.items)]));
	lines.push(row(['# Total weight', weightLabel(table.totals)]));
	lines.push(row(['# Total power', powerLabel(table.totals)]));
	for (const note of table.notes) lines.push(row([`# ${note}`]));

	return lines.join('\r\n') + '\r\n';
}

/** A file name a spreadsheet will not argue with. */
export function csvName(showName: string, title: string): string {
	const clean = (text: string) =>
		text
			.replace(/[^A-Za-z0-9 _-]/g, '')
			.trim()
			.replace(/\s+/g, '-');
	return `${clean(showName) || 'Show'}-${clean(title) || 'Table'}.csv`;
}
