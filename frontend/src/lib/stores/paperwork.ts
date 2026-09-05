/**
 * The sheet set, this browser's place in it, and the writes that change it.
 *
 * The sheets themselves are the show's — a PERSISTED `sheets` collection, seeded into a
 * new show — so they replicate, undo and travel like anything else. What lives here is
 * one operator's: which sheet they are looking at, which block they have hold of, and
 * how far they have zoomed the preview. The same split as `layouts`, for the same
 * reason: an arrangement is the show's and a scroll position is not.
 *
 * # Every verb is one act
 *
 * Editing a sheet is ordinary path writes, the way `stores/editor.ts` builds a rig:
 * there is no engine command for "move a viewport", because moving one is a write to a
 * `blocks` array the schema already knows how to store. What makes a drag *one* act is
 * `gesture.ts`, which is what makes it one Ctrl-Z — and a drag across a sheet is a
 * write per animation frame, so without it a nudge would take fifty presses to undo.
 *
 * # A block list is one field, not a collection
 *
 * `Sheet::blocks` is a `Vec` in one column. So every edit to a block — moving it,
 * changing its scale, adding one — rewrites the whole array. That is deliberate and it
 * is what `Fixture::home_values` already does: a sheet has a handful of blocks, the
 * array is the unit somebody thinks in, and making each block a row would buy ordering
 * problems in exchange for nothing.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import type {
	LabelField,
	Paper,
	Sheet,
	SheetBlock,
	TableKind,
	ViewPreset
} from '../generated/index.js';
import { beginGesture, endGesture } from './gesture.js';
import { collection, showData } from './show.js';

/** Every sheet, in the order the export writes them. */
export const sheets: Readable<Sheet[]> = derived(collection('sheets'), ($sheets) =>
	[...$sheets].sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name))
);

/** Which sheet the panel is showing. Null until there is one. */
export const currentSheet = writable<string | null>(null);

/**
 * Which block on it is selected, by its index in `blocks`.
 *
 * An index rather than an id, because a block has none: it is a position in an array,
 * and giving it an id would be inventing identity for something nothing else refers
 * to. The cost is that deleting a block renumbers the ones after it, which is why
 * every verb below that removes one clears the selection rather than trying to follow
 * it — guessing wrong there means an operator editing a block they are not looking at.
 */
export const selectedBlock = writable<number | null>(null);

/** How big the preview is drawn, as a multiple of its real size on paper. */
export const previewZoom = writable(1);

/** Everything between here and the end is one act, and so one Ctrl-Z. */
async function asOneAct<T>(work: () => Promise<T>): Promise<T> {
	beginGesture();
	try {
		return await work();
	} finally {
		endGesture();
	}
}

function sheetById(id: string): Sheet | undefined {
	return get(sheets).find((sheet) => sheet.id === id);
}

// ── Sheets ───────────────────────────────────────────────────────────────────

/** A new, empty sheet at the end of the set. */
export async function addSheet(name = 'New sheet'): Promise<string> {
	const existing = get(sheets);
	const id = crypto.randomUUID();
	await asOneAct(async () => {
		await showData().sheets.create({
			id,
			name: uniqueSheetName(name, existing),
			sort_order: existing.length,
			paper: 'A3',
			landscape: true,
			frame: true,
			title_block: true,
			blocks: []
		});
	});
	return id;
}

/** A copy of a sheet, straight after it. */
export async function duplicateSheet(id: string): Promise<string | null> {
	const sheet = sheetById(id);
	if (!sheet) return null;
	const copy = crypto.randomUUID();
	await asOneAct(async () => {
		await showData().sheets.create({
			...sheet,
			id: copy,
			name: uniqueSheetName(sheet.name, get(sheets)),
			// Straight after the original rather than at the end: a duplicate is made to
			// be a variant of the thing beside it, and a set that reorders itself under
			// the operator is a set they have to re-find their place in.
			sort_order: sheet.sort_order + 1,
			blocks: structuredClone(sheet.blocks)
		});
		await renumberAfter(sheet.sort_order, copy);
	});
	return copy;
}

export async function deleteSheet(id: string): Promise<void> {
	await asOneAct(async () => {
		await showData().sheets.byId(id).delete();
	});
	selectedBlock.set(null);
}

/** Move a sheet one place up or down the set. */
export async function moveSheet(id: string, by: -1 | 1): Promise<void> {
	const order = get(sheets);
	const at = order.findIndex((sheet) => sheet.id === id);
	const to = at + by;
	if (at < 0 || to < 0 || to >= order.length) return;
	const moved = [...order];
	moved.splice(to, 0, ...moved.splice(at, 1));
	await asOneAct(async () => {
		// Every row rewritten rather than the two swapped, because `sort_order` is only
		// meaningful as a whole: a set that has been reordered a few times has gaps and
		// ties in it, and swapping two numbers inside that does not always move anything.
		for (const [index, sheet] of moved.entries()) {
			if (sheet.sort_order !== index) {
				await showData().sheets.byId(sheet.id).sort_order.set(index);
			}
		}
	});
}

/** Push everything at or after a position down one, leaving room for `except`. */
async function renumberAfter(from: number, except: string): Promise<void> {
	for (const sheet of get(sheets)) {
		if (sheet.id === except || sheet.sort_order <= from) continue;
		await showData().sheets.byId(sheet.id).sort_order.set(sheet.sort_order + 1);
	}
}

/** One of a sheet's own fields. */
export async function setSheetField<K extends keyof Sheet>(
	id: string,
	field: K,
	value: Sheet[K]
): Promise<void> {
	await asOneAct(async () => {
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		await (showData().sheets.byId(id) as any)[field].set(value);
	});
}

// ── Blocks ───────────────────────────────────────────────────────────────────

/**
 * Rewrite one block of a sheet.
 *
 * `gesture` is `false` for a write that is *part* of a gesture somebody else opened —
 * a drag calls this once a frame and must not open and close one each time, or the
 * drag becomes fifty acts and the tail of it lands outside the last.
 */
export async function writeBlock(
	sheetId: string,
	index: number,
	block: SheetBlock,
	gesture = true
): Promise<void> {
	const sheet = sheetById(sheetId);
	if (!sheet || index < 0 || index >= sheet.blocks.length) return;
	const blocks = [...sheet.blocks];
	blocks[index] = block;
	const write = () => showData().sheets.byId(sheetId).blocks.set(blocks);
	await (gesture ? asOneAct(write) : write());
}

/** Add a block to a sheet, and select it. */
export async function addBlock(sheetId: string, block: SheetBlock): Promise<void> {
	const sheet = sheetById(sheetId);
	if (!sheet) return;
	await asOneAct(async () => {
		await showData().sheets.byId(sheetId).blocks.set([...sheet.blocks, block]);
	});
	selectedBlock.set(sheet.blocks.length);
}

export async function removeBlock(sheetId: string, index: number): Promise<void> {
	const sheet = sheetById(sheetId);
	if (!sheet) return;
	const blocks = sheet.blocks.filter((_, at) => at !== index);
	await asOneAct(async () => {
		await showData().sheets.byId(sheetId).blocks.set(blocks);
	});
	// Cleared rather than moved to a neighbour: the indices after this one have all
	// shifted, and a selection that quietly became a different block is an operator
	// editing something they are not looking at.
	selectedBlock.set(null);
}

/** A block of each kind, with defaults worth having. */
export function newViewport(rect: {
	x: number;
	y: number;
	w: number;
	h: number;
}): SheetBlock {
	return {
		type: 'Viewport',
		rect,
		title: 'Plan',
		view: 'Plan' as ViewPreset,
		projection: 'Orthographic',
		scale: { type: 'Fit' },
		style: { type: 'Drafting', lines: 'Hidden', ink: 'Mono' },
		layers: null,
		labels: [],
		scale_bar: true,
		orientation_mark: true,
		// A new viewport opens as a plan, and a plan is where a rigger reads spacings
		// off — the same rule the seeded sheets follow.
		dimensions: { datum: 'Left', running: true, above: false }
	};
}

export function newTable(rect: { x: number; y: number; w: number; h: number }): SheetBlock {
	return {
		type: 'Table',
		rect,
		title: 'Patch',
		kind: 'Patch' as TableKind,
		grouping: 'Structure',
		rows: null,
		layers: null
	};
}

export function newText(rect: { x: number; y: number; w: number; h: number }): SheetBlock {
	return { type: 'Text', rect, text: 'Note', size_mm: 2.5, bold: false };
}

// ── Geometry ─────────────────────────────────────────────────────────────────

/** Which edges a grab moves. A move is all four. */
export type Grip = 'move' | 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

/** The smallest a block may be dragged to, in millimetres. */
export const MIN_BLOCK_MM = 10;

/**
 * How much of a block must stay on the paper, in millimetres.
 *
 * A block dragged entirely past the edge is a block nobody can grab again, and the only
 * way back would be to type its coordinates — which is exactly the thing dragging exists
 * to avoid. Partly off the page is allowed, because arranging a sheet legitimately goes
 * through that state.
 */
export const KEEP_ON_PAPER_MM = 8;

/**
 * Where a grip takes a rectangle, given a drag in millimetres.
 *
 * Pure and here rather than in the component so it can be tested, because the cases that
 * matter are not the ordinary one. Dragging a left edge *past* the right edge has to
 * stop rather than turn the rectangle inside out — a negative width draws as nothing and
 * can never be grabbed again to fix it — and a move has to leave a corner of the block on
 * the paper for the same reason.
 *
 * `paper` is the sheet's own size. Omitted, a move is unclamped, which is what the tests
 * for the resize cases want.
 */
export function resizedRect(
	from: { x: number; y: number; w: number; h: number },
	grip: Grip,
	dx: number,
	dy: number,
	paper?: { w: number; h: number }
): { x: number; y: number; w: number; h: number } {
	if (grip === 'move') {
		const moved = { ...from, x: from.x + dx, y: from.y + dy };
		if (!paper) return moved;
		return {
			...moved,
			x: clamp(moved.x, KEEP_ON_PAPER_MM - from.w, paper.w - KEEP_ON_PAPER_MM),
			y: clamp(moved.y, KEEP_ON_PAPER_MM - from.h, paper.h - KEEP_ON_PAPER_MM)
		};
	}
	let { x, y, w, h } = from;
	if (grip.includes('w')) {
		const by = Math.min(dx, w - MIN_BLOCK_MM);
		x += by;
		w -= by;
	}
	if (grip.includes('e')) w = Math.max(MIN_BLOCK_MM, w + dx);
	if (grip.includes('n')) {
		const by = Math.min(dy, h - MIN_BLOCK_MM);
		y += by;
		h -= by;
	}
	if (grip.includes('s')) h = Math.max(MIN_BLOCK_MM, h + dy);
	return { x, y, w, h };
}

function clamp(value: number, low: number, high: number): number {
	return Math.min(Math.max(value, low), high);
}

/**
 * A name nothing else in the set has.
 *
 * Exported for the tests, and because "Plan 2" beside "Plan" is a rule worth being able
 * to check rather than a detail of one caller.
 */
export function uniqueSheetName(wanted: string, existing: { name: string }[]): string {
	const taken = new Set(existing.map((sheet) => sheet.name));
	if (!taken.has(wanted)) return wanted;
	for (let n = 2; ; n++) {
		const tried = `${wanted} ${n}`;
		if (!taken.has(tried)) return tried;
	}
}

// ── Options a control needs ──────────────────────────────────────────────────

export const PAPERS: Paper[] = ['A4', 'A3', 'A2', 'A1'];

export const VIEWS: { value: ViewPreset; label: string }[] = [
	{ value: 'Plan', label: 'Plan' },
	{ value: 'Front', label: 'Front' },
	{ value: 'Section', label: 'Section' },
	{ value: 'ThreeQuarter', label: 'Three-quarter' },
	{ value: 'Focus', label: 'Focus' }
];

/** The ratios a rule is cut for. The same list `pult-schema` holds. */
export const SCALES = [10, 20, 25, 50, 100, 200, 500, 1000];

export const LABEL_FIELDS: { value: LabelField; label: string }[] = [
	{ value: 'Name', label: 'Name' },
	{ value: 'Number', label: 'Number' },
	{ value: 'Unit', label: 'Unit' },
	{ value: 'Address', label: 'Address' },
	{ value: 'TypeName', label: 'Type' },
	{ value: 'TypeShortName', label: 'Type (short)' },
	{ value: 'Mode', label: 'Mode' }
];

export const TABLE_KINDS: { value: TableKind; label: string; blurb: string }[] = [
	{ value: 'Patch', label: 'Patch', blurb: 'One row per fixture: type, mode, address, place.' },
	{
		value: 'Loading',
		label: 'Loading',
		blurb: 'What hangs where and what it weighs — the structure included.'
	},
	{ value: 'Power', label: 'Power', blurb: 'What it draws.' },
	{ value: 'Counts', label: 'Counts', blurb: 'One row per fixture type. The rider’s table.' }
];

export const GROUPINGS = [
	{ value: 'Structure', label: 'By truss' },
	{ value: 'Layer', label: 'By layer' },
	{ value: 'Class', label: 'By class' },
	{ value: 'FixtureType', label: 'By type' },
	{ value: 'Flat', label: 'Flat' }
] as const;

// ── Files ────────────────────────────────────────────────────────────────────

/**
 * A file name for an export.
 *
 * The show's name, cleaned to something every operating system will accept — a
 * production called "Kelter: The Greatest Showman / v6" is a real name and not a real
 * file name.
 */
export function exportName(showName: string, extension: string): string {
	const clean =
		showName
			.replace(/[^A-Za-z0-9 _-]/g, '')
			.trim()
			.replace(/\s+/g, '-') || 'Show';
	return `${clean}-Paperwork.${extension}`;
}

/** Hand a file to the browser. */
export function download(bytes: Uint8Array | string, name: string, mime: string): void {
	const blob = new Blob([bytes as BlobPart], { type: mime });
	const url = URL.createObjectURL(blob);
	const link = document.createElement('a');
	link.href = url;
	link.download = name;
	link.click();
	// Revoked on the next turn rather than at once: revoking synchronously after
	// `click()` races the browser's own read of the URL in some builds, and the failure
	// is an empty file.
	setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
