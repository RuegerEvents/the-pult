/**
 * A `Sheet` row, composed into a {@link Drawing}.
 *
 * Synchronous and pure over what it is given, which is the point: everything that has
 * to be waited for — the tables from the station, the rasters from an offscreen
 * renderer, a thumbnail's bytes, every mesh the drawing needs — is resolved by the
 * caller and handed in. So composing a sheet is a function of the show and nothing
 * else, and a test can assert what lands where without a canvas, a socket or a clock.
 *
 * # A sheet says what it is not showing
 *
 * The rule the tables follow, applied to the drawing: a plan that could not place
 * twelve fixtures prints "12 fixtures are not placed" in its margin rather than quietly
 * drawing fewer heads than the patch says exist. A drawing whose absences are invisible
 * is worse than no drawing, because somebody counts the heads on it.
 */

import type {
	Fixture,
	FixtureType,
	Layer,
	Production,
	SceneObject,
	Sheet,
	Table,
	Vec3,
	ViewportBlock
} from '../generated/index.js';
import { byId, worldTransform } from '../scene.js';
import {
	BLACK,
	WEIGHT,
	WHITE,
	clipAll,
	page,
	rectPath,
	stroke,
	type DrawItem,
	type Drawing,
	type Mm,
	type PathItem,
	type Pt,
	type RectMm
} from './drawing.js';
import type { Metrics } from './font.js';
import { labelItems, layOutLabels, type LabelRequest } from './labels.js';
import {
	basisFor,
	depthOf,
	extentIn,
	fittedDenominator,
	mmPerMetre,
	silhouette,
	toPage,
	wireframe,
	painted,
	type Axis,
	type Camera,
	type Drawable
} from './project.js';
import { chainFor } from './dimensions.js';
import { placeSymbol, resolveSymbol, symbolPaths, thumbnailSize } from './symbols.js';
import { drawTable } from './table.js';

/** How a sheet is laid out inside its paper. */
const FRAME_MARGIN: Mm = 10;
const TITLE_BLOCK: RectMm = { x: 252, y: 245, w: 150, h: 34 };

/** Everything a sheet needs that had to be fetched or measured first. */
export interface SheetContext {
	metrics: Metrics;
	showName: string;
	production: Production;
	/** Which of how many, for the title block. */
	sheetNumber: number;
	sheetCount: number;

	fixtures: Fixture[];
	fixtureTypes: FixtureType[];
	sceneObjects: SceneObject[];
	layers: Layer[];

	/**
	 * How big each scene object is, in metres, as a box.
	 *
	 * Measured by the browser: a catalogue piece from its own dimensions, a drawing's
	 * mesh from the bounds `geometry.ts` took when it loaded it. **A box rather than
	 * the mesh itself**, which is the one honest limitation of a drafting viewport and
	 * is worth being plain about: a truss draws as its outline rather than as its
	 * lattice. That is what a plan shows at any scale a plan is drawn at, it is exact
	 * for a deck, a flat and a wall, and it costs eight points instead of two thousand
	 * per piece. A rig of ninety sections is 720 points rather than 180,000.
	 */
	sizes: Map<string, Vec3>;

	/** The computed tables, keyed by the index of their block on this sheet. */
	tables: Map<number, Table>;
	/** Rendered pictures, keyed the same way, with the dpi actually achieved. */
	rasters: Map<number, { data: Uint8Array; dpi: number }>;
	/** A fixture type's thumbnail, rasterised, keyed by type id. */
	thumbnails: Map<string, Uint8Array>;
}

/** Compose one sheet. */
export function composeSheet(sheet: Sheet, ctx: SheetContext): Drawing {
	const landscape = sheet.landscape;
	const [w, h] = paperSize(sheet.paper, landscape);
	const drawing = page(w, h);

	if (sheet.frame) drawing.items.push(...frameItems(w, h));
	if (sheet.title_block) drawing.items.push(...titleBlockItems(sheet, ctx));

	sheet.blocks.forEach((block, index) => {
		if (block.type === 'Viewport') {
			drawing.items.push(...viewportItems(block, ctx, index));
		} else if (block.type === 'Table') {
			const table = ctx.tables.get(index);
			drawing.items.push(
				...drawTable(table, block.rect as RectMm, block.title, ctx.metrics)
			);
		} else {
			drawing.items.push({
				kind: 'text',
				at: { x: block.rect.x, y: block.rect.y },
				text: block.text,
				size: block.size_mm,
				bold: block.bold,
				baseline: 'top'
			});
		}
	});

	return drawing;
}

function paperSize(paper: string, landscape: boolean): [Mm, Mm] {
	const portrait: Record<string, [Mm, Mm]> = {
		A4: [210, 297],
		A3: [297, 420],
		A2: [420, 594],
		A1: [594, 841]
	};
	const [pw, ph] = portrait[paper] ?? portrait.A3;
	return landscape ? [ph, pw] : [pw, ph];
}

// ── The furniture ────────────────────────────────────────────────────────────

/**
 * The border and its A/B–1/2/3 grid.
 *
 * Not decoration: it is how two people on a phone name the same part of a drawing.
 * The bands are sized so a division is never narrower than about 60 mm, which is what
 * makes "it's in B2" a useful thing to say rather than a coordinate.
 */
function frameItems(w: Mm, h: Mm): DrawItem[] {
	const m = FRAME_MARGIN;
	const band = 8;
	const inner: RectMm = { x: m, y: m, w: w - 2 * m, h: h - 2 * m };
	const items: DrawItem[] = [
		{ kind: 'path', points: [rectPath(inner)], stroke: stroke(WEIGHT.border, BLACK) }
	];

	const columns = Math.max(2, Math.round(inner.w / 140));
	const rows = Math.max(2, Math.round(inner.h / 140));
	const letters = 'ABCDEFGH';

	for (let i = 0; i < columns; i++) {
		const x = inner.x + (inner.w * (i + 0.5)) / columns;
		for (const y of [m - band / 2, h - m + band / 2]) {
			items.push({
				kind: 'text',
				at: { x, y },
				text: String(i + 1),
				size: 3.5,
				anchor: 'middle',
				baseline: 'middle'
			});
		}
		if (i > 0) {
			const edge = inner.x + (inner.w * i) / columns;
			items.push({
				kind: 'path',
				points: [
					[
						{ x: edge, y: m - band },
						{ x: edge, y: m }
					],
					[
						{ x: edge, y: h - m },
						{ x: edge, y: h - m + band }
					]
				],
				stroke: stroke(WEIGHT.thin, BLACK)
			});
		}
	}
	for (let i = 0; i < rows; i++) {
		const y = inner.y + (inner.h * (i + 0.5)) / rows;
		for (const x of [m - band / 2, w - m + band / 2]) {
			items.push({
				kind: 'text',
				at: { x, y },
				text: letters[i] ?? '?',
				size: 3.5,
				anchor: 'middle',
				baseline: 'middle'
			});
		}
		if (i > 0) {
			const edge = inner.y + (inner.h * i) / rows;
			items.push({
				kind: 'path',
				points: [
					[
						{ x: m - band, y: edge },
						{ x: m, y: edge }
					],
					[
						{ x: w - m, y: edge },
						{ x: w - m + band, y: edge }
					]
				],
				stroke: stroke(WEIGHT.thin, BLACK)
			});
		}
	}
	return items;
}

/**
 * The title block.
 *
 * Every line of it may be empty, and an empty one is **omitted rather than printed
 * blank** — a title block with a bare "Venue:" in it reads as a mistake, and one
 * without the line reads as a show that has not been given a venue yet.
 */
function titleBlockItems(sheet: Sheet, ctx: SheetContext): DrawItem[] {
	const r = TITLE_BLOCK;
	const p = ctx.production ?? ({} as Production);
	const split = r.x + r.w * 0.63;
	const items: DrawItem[] = [
		{ kind: 'path', points: [rectPath(r)], fill: WHITE, stroke: stroke(WEIGHT.medium, BLACK) },
		{
			kind: 'path',
			points: [
				[
					{ x: r.x, y: r.y + 7 },
					{ x: r.x + r.w, y: r.y + 7 }
				],
				[
					{ x: split, y: r.y + 7 },
					{ x: split, y: r.y + r.h }
				]
			],
			stroke: stroke(WEIGHT.thin, BLACK)
		},
		{
			kind: 'text',
			at: { x: r.x + 2, y: r.y + 1.8 },
			text: sheet.name,
			size: 3.4,
			bold: true,
			baseline: 'top'
		},
		{
			kind: 'text',
			at: { x: r.x + r.w - 2, y: r.y + 1.8 },
			text: `${ctx.sheetNumber}/${ctx.sheetCount}`,
			size: 2.6,
			anchor: 'end',
			baseline: 'top'
		}
	];

	const left = column(r.x + 2, r.y + 9, split - r.x - 4, [
		{ text: 'Production', size: 2, muted: true, heading: true },
		{ text: p.title || ctx.showName, size: 3, bold: true },
		{ text: p.venue, size: 2.4 },
		{ text: p.address, size: 2.2, muted: true },
		{ text: p.dates, size: 2.4, bold: true }
	]);
	const right = column(split + 2, r.y + 9, r.x + r.w - split - 4, [
		{ text: 'Drawing', size: 2, muted: true, heading: true },
		{ text: p.designer, size: 2.4, bold: true },
		{ text: p.contact, size: 2.2 },
		{ text: p.revision, size: 2.2, muted: true }
	]);
	return [...items, ...left, ...right];
}

function column(
	x: Mm,
	y: Mm,
	width: Mm,
	lines: Array<{ text: string; size: Mm; bold?: boolean; muted?: boolean; heading?: boolean }>
): DrawItem[] {
	void width;
	// A heading over nothing is the same mistake as a blank Venue line, one level up:
	// "Drawing" with an empty box under it reads as a title block that failed to
	// print, where no heading at all reads as a drawing nobody has signed yet.
	if (!lines.some((line) => !line.heading && line.text)) return [];

	const items: DrawItem[] = [];
	let at = y;
	for (const line of lines) {
		if (!line.text) continue;
		items.push({
			kind: 'text',
			at: { x, y: at },
			text: line.text,
			size: line.size,
			bold: line.bold,
			color: line.muted ? '#666666' : BLACK,
			baseline: 'top'
		});
		at += line.size * 1.45;
	}
	return items;
}

// ── Viewports ────────────────────────────────────────────────────────────────

const AXIS: Record<string, Axis> = {
	Plan: 'plan',
	Front: 'front',
	Section: 'section',
	ThreeQuarter: 'quarter',
	Focus: 'front'
};

/**
 * What a viewport's caption may claim about its size.
 *
 * `NTS` for a perspective view, obviously — but also for **every picture viewport**,
 * orthographic ones included, and that is the part worth writing down. A raster is
 * rendered by the rig renderer, which fits its own camera to the rig its own way; this
 * code did not choose that fit and does not know what it came to. An orthographic
 * picture therefore *has* a scale and this is not the thing that knows it, and the
 * distance between those two sentences is exactly the width of a wrong number on a
 * drawing somebody measures off. It printed `1:0` before this was written down.
 */
export function scaleLabel(
	projection: string,
	denominator: number,
	raster = false
): string {
	return projection === 'Perspective' || raster || denominator <= 0 ? 'NTS' : `1:${denominator}`;
}

function viewportItems(block: ViewportBlock, ctx: SheetContext, index: number): DrawItem[] {
	const rect = block.rect as RectMm;
	const frame: DrawItem[] = [
		{ kind: 'path', points: [rectPath(rect)], stroke: stroke(WEIGHT.thin, '#999999') }
	];

	const raster = ctx.rasters.get(index);
	if (block.style.type === 'Picture') {
		const body: DrawItem[] = raster
			? [{ kind: 'image', rect, data: raster.data }]
			: [note(rect, 'This view has not been rendered.', ctx.metrics)];
		return [...frame, ...body, ...captionItems(block, rect, 0, ctx, raster?.dpi)];
	}

	const axis = AXIS[block.view] ?? 'plan';
	const basis = basisFor(axis);
	const objects = byId(ctx.sceneObjects);
	const shown = visible(ctx, block);

	// Every world point that has to fit, which is what decides the scale. Fixtures as
	// points and objects as their eight corners: a truss half out of frame is a truss
	// the plan does not show.
	const worldPoints: Vec3[] = [];
	for (const object of shown.objects) {
		worldPoints.push(...corners(object, ctx, objects));
	}
	for (const fixture of shown.fixtures) {
		const at = fixtureWorld(fixture, objects);
		if (at) worldPoints.push(at);
	}

	const extent = extentIn(worldPoints, basis);
	const fitted = fittedDenominator({ across: extent.across, up: extent.up }, rect);
	const denominator =
		block.scale.type === 'Ratio' ? Math.max(block.scale.denominator, 1) : snap(fitted);
	const camera: Camera = {
		basis,
		centre: extent.centre,
		scale: mmPerMetre(denominator),
		rect
	};

	const drawables: Drawable[] = [];
	const wire = block.style.lines === 'Wireframe';

	for (const object of shown.objects) {
		const box = corners(object, ctx, objects);
		if (box.length === 0) continue;
		const pagePoints = box.map((p) => toPage(p, camera));
		const depth = box.reduce((sum, p) => sum + depthOf(p, camera), 0) / box.length;
		drawables.push({
			depth,
			items: wire ? [wireframe(boxEdges(pagePoints))] : [silhouette(pagePoints)]
		});
	}

	const labelRequests: LabelRequest[] = [];
	let unplaced = 0;
	for (const fixture of shown.fixtures) {
		const at = fixtureWorld(fixture, objects);
		if (!at) {
			unplaced++;
			continue;
		}
		const type = ctx.fixtureTypes.find((t) => t.id === fixture.fixture_type_id);
		const source = resolveSymbol(type);
		const paths = symbolPaths(type, source, basis, camera.scale);
		const point = toPage(at, camera);
		const items: PathItem[] = placeSymbol(paths, point, headRotation(fixture, objects, axis));
		drawables.push({ depth: depthOf(at, camera) + 1e6, items });

		if (block.labels.length > 0) {
			labelRequests.push({ anchor: point, lines: labelLines(block, fixture, type) });
		}
	}

	const body: DrawItem[] = painted(drawables);

	if (labelRequests.length > 0) {
		const placed = layOutLabels(labelRequests, ctx.metrics, { bounds: inset(rect, 1) });
		for (const label of placed) body.push(...labelItems(label));
	}

	// Dimensions along each bar, for the crew hanging it. Drawn after the heads and
	// before the clip, so a chain that runs off the frame is cut like anything else.
	if (block.dimensions) {
		// Grouped by whatever each fixture actually hangs off, which is usually a
		// `Group`: `kit::truss_run` builds a run as a handle with sections under it and
		// clamps the lights to the handle. A group has no size of its own, so its length
		// is the span of the pieces in it — the first version of this iterated the
		// *pieces* instead and drew nothing at all, because no fixture is parented to
		// one.
		const byParent = new Map<string, Fixture[]>();
		for (const fixture of shown.fixtures) {
			if (!fixture.parent) continue;
			const list = byParent.get(fixture.parent);
			if (list) list.push(fixture);
			else byParent.set(fixture.parent, [fixture]);
		}
		for (const [parentId, onIt] of byParent) {
			const parent = objects.get(parentId);
			if (!parent) continue;
			const size = ctx.sizes.get(parentId) ?? spanOfChildren(parentId, ctx);
			if (!size) continue;
			const world = worldTransform(parent.transform, parent.parent, objects);
			body.push(
				...chainFor(
					onIt,
					size,
					(local) => applyTransform(world, local),
					camera,
					block.dimensions,
					ctx.metrics
				)
			);
		}
	}

	const inside = clipAll(body, rect);
	const notes: DrawItem[] = [];
	if (unplaced > 0) {
		notes.push(
			note(
				{ ...rect, y: rect.y + rect.h - 4 },
				`${unplaced} ${unplaced === 1 ? 'fixture is' : 'fixtures are'} not placed and are not drawn.`,
				ctx.metrics,
				'start'
			)
		);
	}

	return [
		...frame,
		...inside,
		...notes,
		...furniture(block, rect, denominator, camera),
		...captionItems(block, rect, denominator, ctx)
	];
}

/** The nearest standard scale that still fits: the same list `pult-schema` holds. */
const STANDARD_SCALES = [10, 20, 25, 50, 100, 200, 500, 1000];

export function snap(fitted: number): number {
	return STANDARD_SCALES.find((s) => s >= fitted) ?? STANDARD_SCALES[STANDARD_SCALES.length - 1];
}

function inset(r: RectMm, by: Mm): RectMm {
	return { x: r.x + by, y: r.y + by, w: r.w - 2 * by, h: r.h - 2 * by };
}

/** What this viewport is showing, after its layer filter. */
function visible(ctx: SheetContext, block: ViewportBlock) {
	const wanted = block.layers ? new Set(block.layers) : null;
	const keep = (layer: string | null) => !wanted || (layer !== null && wanted.has(layer));
	return {
		fixtures: ctx.fixtures.filter((f) => keep(f.layer)),
		objects: ctx.sceneObjects.filter((o) => keep(o.layer) && o.kind !== 'Group')
	};
}

/**
 * How long a handle is, measured across the pieces hanging under it.
 *
 * A `Group` has no geometry — it is the thing that moves a truss and its lights
 * together — so it has no size to dimension against. Its length is the extent of its
 * children along its own X, which for a run laid out by `kit::truss_run` is exactly the
 * length of the run. `null` for a handle with nothing measurable under it.
 */
function spanOfChildren(parentId: string, ctx: SheetContext): Vec3 | null {
	let low = Infinity;
	let high = -Infinity;
	for (const child of ctx.sceneObjects) {
		if (child.parent !== parentId) continue;
		const size = ctx.sizes.get(child.id);
		if (!size) continue;
		// The child's own turn is not applied: a run's sections lie along the handle's
		// X, and a section stood on end in a boom has its length in Y, where a chain
		// along the bar has nothing to say anyway.
		const at = child.transform.position.x;
		low = Math.min(low, at - size.x / 2);
		high = Math.max(high, at + size.x / 2);
	}
	return Number.isFinite(low) && high > low ? { x: high - low, y: 0, z: 0 } : null;
}

/** The eight world corners of an object's box. */
function corners(
	object: SceneObject,
	ctx: SheetContext,
	objects: Map<string, SceneObject>
): Vec3[] {
	const size = ctx.sizes.get(object.id);
	if (!size) return [];
	const world = worldTransform(object.transform, object.parent, objects);
	const out: Vec3[] = [];
	for (const sx of [-0.5, 0.5]) {
		for (const sy of [-0.5, 0.5]) {
			for (const sz of [-0.5, 0.5]) {
				out.push(applyTransform(world, { x: size.x * sx, y: size.y * sy, z: size.z * sz }));
			}
		}
	}
	return out;
}

/**
 * The twelve edges of a projected box, from the eight corners in the order
 * {@link corners} makes them: x outermost, then y, then z.
 */
function boxEdges(p: Pt[]): Array<[Pt, Pt]> {
	if (p.length !== 8) return [];
	const pairs: Array<[number, number]> = [
		[0, 1],
		[1, 3],
		[3, 2],
		[2, 0],
		[4, 5],
		[5, 7],
		[7, 6],
		[6, 4],
		[0, 4],
		[1, 5],
		[2, 6],
		[3, 7]
	];
	return pairs.map(([a, b]) => [p[a], p[b]] as [Pt, Pt]);
}

function applyTransform(
	t: { position: Vec3; rotation: Vec3; scale: Vec3 },
	local: Vec3
): Vec3 {
	const rad = (d: number) => (d * Math.PI) / 180;
	const [rx, ry, rz] = [rad(t.rotation.x), rad(t.rotation.y), rad(t.rotation.z)];
	let { x, y, z } = { x: local.x * t.scale.x, y: local.y * t.scale.y, z: local.z * t.scale.z };
	// XYZ Euler, the order `scene.ts` composes in.
	let s = Math.sin(rx);
	let c = Math.cos(rx);
	[y, z] = [y * c - z * s, y * s + z * c];
	s = Math.sin(ry);
	c = Math.cos(ry);
	[x, z] = [x * c + z * s, -x * s + z * c];
	s = Math.sin(rz);
	c = Math.cos(rz);
	[x, y] = [x * c - y * s, x * s + y * c];
	return { x: x + t.position.x, y: y + t.position.y, z: z + t.position.z };
}

function fixtureWorld(fixture: Fixture, objects: Map<string, SceneObject>): Vec3 | null {
	if (!fixture.position) return null;
	return worldTransform(fixture.position, fixture.parent, objects).position;
}

/**
 * How far round a head's symbol is turned on the page.
 *
 * Only a plan has an answer: on an elevation a head turned about the vertical is the
 * same silhouette, and turning its symbol would be inventing a rotation the drawing
 * cannot show.
 */
function headRotation(
	fixture: Fixture,
	objects: Map<string, SceneObject>,
	axis: Axis
): number {
	if (axis !== 'plan' || !fixture.position) return 0;
	return worldTransform(fixture.position, fixture.parent, objects).rotation.y;
}

function labelLines(block: ViewportBlock, fixture: Fixture, type: FixtureType | undefined): string[] {
	return block.labels.map((field) => {
		switch (field) {
			case 'Name':
				return fixture.name;
			case 'Number':
				return fixture.fixture_number === null ? '' : String(fixture.fixture_number);
			case 'Unit':
				return fixture.unit_number === null ? '' : String(fixture.unit_number);
			case 'Address':
				return addressOf(fixture);
			case 'TypeName':
				return type?.name ?? '';
			case 'TypeShortName':
				return type?.short_name || (type?.name ?? '');
			case 'Mode':
				return 'Dmx' in fixture.address ? fixture.address.Dmx.mode : '';
			default:
				return '';
		}
	});
}

/**
 * `1/271`, or `1/271 + 2/1` where a fixture's dimmer is on its own break.
 *
 * Mirrors `pult_schema::types::paperwork::address_of`, and for the same reason the
 * evaluator is compiled twice: the label on the plan and the cell in the patch table
 * are the same fact, and two spellings of it is a sheet that contradicts itself.
 */
export function addressOf(fixture: Fixture): string {
	const address = fixture.address as Record<string, unknown>;
	if ('Dmx' in address) {
		const dmx = address.Dmx as { breaks: Array<{ universe: number; address: number }> };
		return dmx.breaks.map((b) => `${b.universe}/${b.address}`).join(' + ');
	}
	if ('OpenHaunt' in address) {
		const node = address.OpenHaunt as { serial: string };
		return `node ${node.serial}`;
	}
	return '';
}

/** The scale bar and the upstage mark. */
function furniture(
	block: ViewportBlock,
	rect: RectMm,
	denominator: number,
	camera: Camera
): DrawItem[] {
	const items: DrawItem[] = [];
	if (block.scale_bar && block.projection !== 'Perspective') {
		items.push(...scaleBar({ x: rect.x + 3, y: rect.y + rect.h - 6 }, denominator));
	}
	if (block.orientation_mark) {
		// Far enough in that the arrow and its label are both inside the frame: the
		// mark is 6 mm long and its text sits a millimetre past the tip.
		items.push(...upstageMark({ x: rect.x + rect.w - 10, y: rect.y + 12 }, camera));
	}
	return items;
}

/**
 * A bar of a round number of metres, chosen so it is between 20 and 60 mm long.
 *
 * A drawing carries both a stated ratio and a bar because they fail differently: a
 * ratio is wrong if anybody rescales the file, and a bar is right whatever happens to
 * it — it is the one thing on the sheet a photocopier cannot lie about.
 */
export function scaleBar(at: Pt, denominator: number): DrawItem[] {
	const perMetre = mmPerMetre(denominator);
	const metres = [1, 2, 5, 10, 20, 50, 100].find((m) => m * perMetre >= 20) ?? 100;
	const length = metres * perMetre;
	const h = 1.4;
	return [
		{
			kind: 'path',
			points: [
				[
					{ x: at.x, y: at.y },
					{ x: at.x + length, y: at.y },
					{ x: at.x + length, y: at.y + h },
					{ x: at.x, y: at.y + h },
					{ x: at.x, y: at.y }
				],
				[
					{ x: at.x + length / 2, y: at.y },
					{ x: at.x + length / 2, y: at.y + h }
				]
			],
			stroke: stroke(WEIGHT.thin, BLACK)
		},
		{
			kind: 'path',
			points: [
				[
					{ x: at.x, y: at.y },
					{ x: at.x + length / 2, y: at.y },
					{ x: at.x + length / 2, y: at.y + h },
					{ x: at.x, y: at.y + h },
					{ x: at.x, y: at.y }
				]
			],
			fill: BLACK
		},
		{
			kind: 'text',
			at: { x: at.x + length, y: at.y - 0.6 },
			text: `${metres} m`,
			size: 2,
			anchor: 'end',
			baseline: 'baseline'
		}
	];
}

/** Which way is upstage, drawn as an arrow. */
function upstageMark(at: Pt, camera: Camera): DrawItem[] {
	// Upstage is −Z in world axes; where that ends up on the page is the projection's
	// business, which is why this asks the camera rather than assuming "up".
	const from = toPage({ x: 0, y: 0, z: 0 }, camera);
	const to = toPage({ x: 0, y: 0, z: -1 }, camera);
	const dx = to.x - from.x;
	const dy = to.y - from.y;
	const length = Math.hypot(dx, dy) || 1;
	const ux = (dx / length) * 6;
	const uy = (dy / length) * 6;
	const tip = { x: at.x + ux, y: at.y + uy };
	return [
		{
			kind: 'path',
			points: [
				[at, tip],
				[
					{ x: tip.x - ux * 0.35 - uy * 0.2, y: tip.y - uy * 0.35 + ux * 0.2 },
					tip,
					{ x: tip.x - ux * 0.35 + uy * 0.2, y: tip.y - uy * 0.35 - ux * 0.2 }
				]
			],
			stroke: stroke(WEIGHT.thin, BLACK)
		},
		{
			kind: 'text',
			at: { x: tip.x, y: tip.y - 1 },
			text: 'US',
			size: 2,
			anchor: 'middle',
			baseline: 'baseline'
		}
	];
}

/** A viewport's own title and scale, under its frame. */
function captionItems(
	block: ViewportBlock,
	rect: RectMm,
	denominator: number,
	ctx: SheetContext,
	dpi?: number
): DrawItem[] {
	void ctx;
	const items: DrawItem[] = [];
	const y = rect.y + rect.h + 1;
	if (block.title) {
		items.push({
			kind: 'text',
			at: { x: rect.x, y },
			text: block.title,
			size: 2.6,
			bold: true,
			baseline: 'top'
		});
	}
	const scale = scaleLabel(block.projection, denominator, block.style.type === 'Picture');
	const suffix = dpi ? `  ·  ${dpi} dpi` : '';
	items.push({
		kind: 'text',
		at: { x: rect.x + rect.w, y },
		text: scale + suffix,
		size: 2.6,
		anchor: 'end',
		baseline: 'top'
	});
	return items;
}

function note(rect: RectMm, text: string, metrics: Metrics, anchor: 'start' | 'middle' = 'middle') {
	void metrics;
	return {
		kind: 'text' as const,
		at:
			anchor === 'middle'
				? { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 }
				: { x: rect.x + 2, y: rect.y },
		text,
		size: 2.4,
		color: '#666666',
		anchor,
		baseline: 'middle' as const
	};
}
