/**
 * What a light looks like from above, on paper.
 *
 * Three sources, and the whole reason there are three is that a GDTF may carry a
 * usable version of either, both or neither — a file whose geometry is one
 * undifferentiated block draws a better head from its thumbnail, and one whose
 * thumbnail is the manufacturer's logo draws a better head from its geometry. Nobody
 * can tell which from outside the file, so `FixtureType::plan_symbol` is a field and
 * `Auto` is a chain.
 *
 * **Geometry first.** A projected outline is the fixture's own body at its own size,
 * so a plan drawn from it can be measured for clearance between two moving heads. A
 * thumbnail is whatever the manufacturer drew, at whatever size they drew it, and is
 * scaled to the type's stated dimensions here rather than trusted to be to scale.
 *
 * Nothing in here loads a mesh. A fixture type's `geometry` is a tree of boxes with
 * offsets and sizes — that is what GDTF's `<Models>` gives and what the importer keeps
 * — so a head's outline costs eight points per part and no network at all. Trusses are
 * the ones with meshes, and those are `stock.ts` and `geometry.ts`'s business.
 */

import type { FixtureType, PlanSymbol, Vec3 } from '../generated/index.js';
import type { Mm, PathItem, Pt } from './drawing.js';
import { BLACK, WEIGHT, WHITE, stroke } from './drawing.js';
import { hull, type Basis } from './project.js';

/** Which of the three a type actually draws with, once `Auto` has been walked. */
export type SymbolSource = 'geometry' | 'thumbnail' | 'generic';

/**
 * The chain, walked.
 *
 * `Auto` takes the first that exists; anything else is taken at its word and falls
 * back to generic, because an operator who asked for the thumbnail of a type that has
 * none should get a symbol rather than nothing at all.
 */
export function resolveSymbol(type: FixtureType | undefined, choice?: PlanSymbol): SymbolSource {
	if (!type) return 'generic';
	const wanted = choice ?? type.plan_symbol ?? 'Auto';
	const hasGeometry = type.geometry?.some((part) => part.size) ?? false;
	const hasThumbnail = Boolean(type.thumbnail);

	if (wanted === 'Geometry') return hasGeometry ? 'geometry' : 'generic';
	if (wanted === 'Thumbnail') return hasThumbnail ? 'thumbnail' : 'generic';
	if (wanted === 'Generic') return 'generic';
	if (hasGeometry) return 'geometry';
	if (hasThumbnail) return 'thumbnail';
	return 'generic';
}

/** The footprint a generic symbol is drawn at, in metres. */
function footprint(type: FixtureType | undefined): { w: number; d: number } {
	const dimensions = type?.physical?.dimensions_m as Vec3 | null | undefined;
	// A quarter of a metre square is a par can, which is the smallest thing anybody
	// hangs and the right guess for a type that says nothing about its own size.
	return {
		w: Math.max(dimensions?.x ?? 0.25, 0.05),
		d: Math.max(dimensions?.z ?? 0.25, 0.05)
	};
}

/** Whether a type has pan and tilt, which is what makes it read as a moving head. */
function moves(type: FixtureType | undefined): boolean {
	const kinds = type?.parameters?.map((p) => String(p.kind)) ?? [];
	return kinds.includes('Pan') && kinds.includes('Tilt');
}

/**
 * A generic head: a body, and a mark saying which way it faces.
 *
 * A mover gets a circle in a square — the yoke and the head, which is what every plan
 * in the world draws a mover as — and everything else gets the square alone. The nose
 * mark points along −Z in the fixture's own frame, which is where a fixture's axis
 * points and therefore which way the lamp is aimed.
 */
function genericOutline(type: FixtureType | undefined, scale: number): PathItem[] {
	const { w, d } = footprint(type);
	const halfW = (w * scale) / 2;
	const halfD = (d * scale) / 2;
	const body: PathItem = {
		kind: 'path',
		points: [
			[
				{ x: -halfW, y: -halfD },
				{ x: halfW, y: -halfD },
				{ x: halfW, y: halfD },
				{ x: -halfW, y: halfD },
				{ x: -halfW, y: -halfD }
			]
		],
		fill: WHITE,
		stroke: stroke(WEIGHT.thin, BLACK)
	};
	if (!moves(type)) return [body];

	const radius = Math.min(halfW, halfD) * 0.78;
	const circle: Pt[] = [];
	// Twenty-four segments: at the size a head is drawn on an A3 plan that is smooth,
	// and it keeps a five-hundred-fixture sheet to a number of points a PDF can hold.
	for (let i = 0; i <= 24; i++) {
		const a = (i / 24) * Math.PI * 2;
		circle.push({ x: Math.cos(a) * radius, y: Math.sin(a) * radius });
	}
	return [
		body,
		{ kind: 'path', points: [circle], stroke: stroke(WEIGHT.fine, BLACK) },
		{
			kind: 'path',
			points: [
				[
					{ x: 0, y: 0 },
					{ x: 0, y: -halfD }
				]
			],
			stroke: stroke(WEIGHT.thin, BLACK)
		}
	];
}

/**
 * The outline of the type's own parts, projected.
 *
 * Each part is a box at an offset; the eight corners of each go through the basis and
 * the whole lot is hulled, which for a lantern body is its silhouette. Parts on a
 * moving axis are included where they are in the file, which is the head at rest —
 * drawing a plan of where every head *happens to be pointing* would make the same rig
 * a different drawing every time somebody moved a fader.
 */
function geometryOutline(type: FixtureType, basis: Basis, scale: number): PathItem[] {
	const points: Pt[] = [];
	for (const part of type.geometry ?? []) {
		const size = part.size as Vec3 | null | undefined;
		if (!size) continue;
		const o = part.offset as Vec3;
		for (const sx of [-0.5, 0.5]) {
			for (const sy of [-0.5, 0.5]) {
				for (const sz of [-0.5, 0.5]) {
					const p = {
						x: o.x + size.x * sx,
						y: o.y + size.y * sy,
						z: o.z + size.z * sz
					};
					points.push({
						x: (p.x * basis.right.x + p.y * basis.right.y + p.z * basis.right.z) * scale,
						y: -(p.x * basis.up.x + p.y * basis.up.y + p.z * basis.up.z) * scale
					});
				}
			}
		}
	}
	if (points.length < 3) return genericOutline(type, scale);
	return [
		{
			kind: 'path',
			points: [hull(points)],
			fill: WHITE,
			stroke: stroke(WEIGHT.thin, BLACK)
		}
	];
}

/**
 * A head's symbol, in millimetres about its own centre, ready to be translated to
 * where the fixture is.
 *
 * The thumbnail source is not here: it is a raster placement rather than a path, so it
 * is handled by the sheet composer, which is the half of this feature that can hold
 * bytes. What comes back for a thumbnail-sourced type is its generic outline, and the
 * composer draws the picture over it — so a thumbnail that fails to load leaves a head
 * on the plan rather than a gap.
 */
export function symbolPaths(
	type: FixtureType | undefined,
	source: SymbolSource,
	basis: Basis,
	scale: number
): PathItem[] {
	if (source === 'geometry' && type) return geometryOutline(type, basis, scale);
	return genericOutline(type, scale);
}

/** Move a symbol's paths to where the head is, turning them by its rotation. */
export function placeSymbol(paths: PathItem[], at: Pt, rotateDeg: number): PathItem[] {
	const a = (rotateDeg * Math.PI) / 180;
	const cos = Math.cos(a);
	const sin = Math.sin(a);
	return paths.map((path) => ({
		...path,
		points: path.points.map((line) =>
			line.map((p) => ({
				x: at.x + p.x * cos - p.y * sin,
				y: at.y + p.x * sin + p.y * cos
			}))
		)
	}));
}

/** How big a thumbnail is drawn, in millimetres: the type's own footprint. */
export function thumbnailSize(type: FixtureType | undefined, scale: number): { w: Mm; h: Mm } {
	const { w, d } = footprint(type);
	return { w: w * scale, h: d * scale };
}
