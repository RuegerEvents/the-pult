/**
 * Drawing the pieces the console knows by name.
 *
 * This used to *be* the geometry: two metres of 290 mm box truss was four cylinders
 * and a zig-zag of bracing, built here out of `CylinderGeometry`. Which drew a rig
 * beautifully and exported nothing — MVR has no primitive, its `GeometryNode` is a
 * file or a symbol instance, so a stock piece written into an archive was an empty
 * group. A rig built from scratch is *all* stock pieces, so that was the whole rig.
 *
 * So the geometry moved to `pult-schema`'s `stock` module and this became a loader:
 * `GET /stock/{id}.glb` serves what the station generates from the table, and the
 * same bytes go into an export. What somebody opens in Vectorworks is what this drew.
 *
 * # One geometry per piece, however many are in the rig
 *
 * A festival rig is a hundred identical truss sections. `geometry.ts` caches by URL
 * and `instance` clones, so a hundred of them cost one download and one upload — the
 * same bargain an imported symbol strikes, now literally the same code.
 *
 * The materials are still this file's, and are shared: a material per object is a
 * shader compile per object, and the `.glb` carries one of its own so that anybody
 * else opening the file sees aluminium rather than white plastic.
 */

import * as THREE from 'three';

import { CATALOGUE, piece, type StockPiece } from '$lib/generated/catalogue.js';
import type { PropertyKind, StockShape } from '$lib/generated/index.js';
import { instance, loadFrom, type Loaded } from './geometry.js';

export { CATALOGUE, piece, type StockPiece };

/**
 * How the console's own pieces are finished.
 *
 * Aluminium, and not shiny: a truss under stage light is a matte grey thing, and a
 * mirror-finish one reads as a prop. Shared by every stock mesh in the rig.
 */
const ALUMINIUM = new THREE.MeshStandardMaterial({
	color: 0x9aa0a6,
	roughness: 0.65,
	metalness: 0.85
});

/** Decks and walls are painted, not extruded aluminium. */
const PAINTED = new THREE.MeshStandardMaterial({
	color: 0x2e3136,
	roughness: 0.9,
	metalness: 0.05
});

function materialFor(shape: StockShape): THREE.Material {
	return shape === 'Deck' || shape === 'Panel' ? PAINTED : ALUMINIUM;
}

/**
 * One spelling of what a piece was asked for.
 *
 * The browser's half of `catalogue::canonical_properties`, and it is here for one
 * reason: the URL is the cache key. An object saying nothing and an object spelling
 * out the defaults are the same deck, and two spellings would be two downloads of
 * identical bytes. The station canonicalises again on the way in, so being wrong here
 * costs a request rather than a wrong mesh.
 */
export function canonicalProperties(
	entry: StockPiece | undefined,
	given: unknown
): Record<string, number | string | boolean> {
	const said = (given && typeof given === 'object' ? given : {}) as Record<string, unknown>;
	const out: Record<string, number | string | boolean> = {};
	for (const property of entry?.properties ?? []) {
		out[property.key] = canonicalOne(property.kind, property.default, said[property.key]);
	}
	return out;
}

function canonicalOne(
	kind: PropertyKind,
	fallback: number,
	given: unknown
): number | string | boolean {
	if (kind === 'Bool') return typeof given === 'boolean' ? given : fallback !== 0;
	if ('Choice' in kind) {
		const options = kind.Choice.options;
		return typeof given === 'string' && options.includes(given) ? given : (options[0] ?? '');
	}
	const { min, max, step } = kind.Number;
	const raw = typeof given === 'number' && Number.isFinite(given) ? given : fallback;
	const clamped = Math.min(max, Math.max(min, raw));
	const stepped = step > 0 ? Math.round(clamped / step) * step : clamped;
	// Through six decimals, because the number goes into a cache key as text and a
	// float's own last bits are not a fact about the deck.
	return Math.round(stepped * 1e6) / 1e6;
}

/**
 * Where the station serves one of its own pieces from.
 *
 * `?p=` is left off entirely when the piece asks nothing, which is all but the decks:
 * a shorter URL, and one fewer way for two browsers to disagree about a cache key.
 */
export function stockUrl(id: string, properties: unknown): string {
	const canonical = canonicalProperties(piece(id), properties);
	const query = Object.keys(canonical).length === 0 ? '' : `?p=${encodeURIComponent(JSON.stringify(canonical))}`;
	return `/stock/${id}.glb${query}`;
}

/**
 * A mesh for one catalogue piece, or `null` for a name this build does not know.
 *
 * `null` rather than a placeholder box: an unknown id is a showfile from a later
 * version of this console naming a piece that did not exist yet, and drawing a
 * mystery cube in the middle of somebody's rig is worse than drawing nothing. A
 * *broken* download is a different case and `geometry.ts` still boxes that one,
 * because there the piece is known and known to have failed.
 */
export async function stockMesh(
	id: string | null | undefined,
	properties: unknown
): Promise<THREE.Object3D | null> {
	const entry = piece(id);
	if (!entry) return null;
	const url = stockUrl(entry.id, properties);
	const loaded = await loadFrom(url, url, `${entry.id}.glb`, []);
	return dressed(instance(loaded, false), entry.shape);
}

/**
 * How big a piece is, without waiting for it to load.
 *
 * The table says, and the mesh is generated from the table — so a caller that wants
 * to know how long an `f34-3m` is has no reason to touch the network. A deck's
 * *drawn* height is its legs where those are longer than the slab, which is the one
 * place the two differ.
 */
export function stockSize(
	entry: StockPiece,
	properties: unknown
): { x: number; y: number; z: number } {
	if (entry.shape !== 'Deck') return entry.size;
	const legs = canonicalProperties(entry, properties).leg_height;
	return { ...entry.size, y: Math.max(entry.size.y, typeof legs === 'number' ? legs : 0) };
}

/** Everything in the loaded copy wearing this panel's shared material. */
function dressed(object: THREE.Object3D, shape: StockShape): THREE.Object3D {
	const material = materialFor(shape);
	object.traverse((node) => {
		const mesh = node as THREE.Mesh;
		if (!mesh.isMesh) return;
		mesh.material = material;
		mesh.castShadow = false;
		mesh.receiveShadow = false;
	});
	return object;
}

/** What the Pieces sheet lists, grouped the way somebody would look for one. */
export const PIECE_GROUPS: { shape: StockShape; label: string }[] = [
	{ shape: 'BoxTruss', label: 'Truss' },
	{ shape: 'TrussCorner', label: 'Corners' },
	{ shape: 'BasePlate', label: 'Plates' },
	{ shape: 'Pipe', label: 'Pipe' },
	{ shape: 'Deck', label: 'Decks' },
	{ shape: 'Panel', label: 'Scenery' }
];

/**
 * The catalogue in the order the sheet shows it.
 *
 * Grouped by what somebody is looking for rather than by shape exactly: a base plate
 * and a top plate are one heading, because nobody goes looking for "top plates".
 */
export function groupedCatalogue(): { label: string; pieces: StockPiece[] }[] {
	const claimed = new Set<string>();
	const groups = PIECE_GROUPS.map(({ shape, label }) => {
		const pieces = CATALOGUE.filter(
			(entry) =>
				entry.shape === shape ||
				(shape === 'BasePlate' && entry.shape === 'TopPlate')
		);
		pieces.forEach((entry) => claimed.add(entry.id));
		return { label, pieces };
	}).filter((group) => group.pieces.length > 0);

	// Anything a later version of this console adds that these headings do not cover
	// still turns up, rather than being quietly missing from the picker.
	const rest = CATALOGUE.filter((entry) => !claimed.has(entry.id));
	return rest.length > 0 ? [...groups, { label: 'Other', pieces: rest }] : groups;
}

/** For tests, and for a panel that wants to know what it will be handed. */
export type { Loaded };
