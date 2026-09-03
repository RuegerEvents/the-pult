/**
 * Drawing the pieces the console knows by name.
 *
 * `pult-schema`'s catalogue says an `f34-2m` is two metres of 290 mm box truss.
 * This is what two metres of 290 mm box truss looks like: four chords and a zig-zag
 * of bracing, built out of cylinders rather than loaded from anywhere. So a console
 * that has never imported a mesh still draws a rig, instead of hanging its lights in
 * the air over nothing.
 *
 * # One geometry per piece, however many are in the rig
 *
 * A festival rig is a hundred identical truss sections. Each one is a `Mesh` sharing
 * a single merged `BufferGeometry` and a single material, so a hundred of them cost
 * a hundred draw calls and one upload — the same bargain `geometry.ts` strikes for
 * an imported symbol, and for the same reason.
 *
 * The merge matters as much as the cache. A truss built as a group of twenty
 * cylinders would be twenty draw calls *each*, and a hundred sections would be two
 * thousand — which is the point at which a rig view stops holding frame rate for
 * reasons that have nothing to do with the show.
 */

import * as THREE from 'three';
import { mergeGeometries } from 'three/examples/jsm/utils/BufferGeometryUtils.js';

import { CATALOGUE, piece, type StockPiece } from '$lib/generated/catalogue.js';
import type { StockShape } from '$lib/generated/index.js';

export { CATALOGUE, piece, type StockPiece };

/**
 * How the console's own pieces are finished.
 *
 * Aluminium, and not shiny: a truss under stage light is a matte grey thing, and a
 * mirror-finish one reads as a prop. Shared by every stock mesh in the rig, because
 * a material per object is a shader compile per object.
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

/** Built once per catalogue id, and cloned into place after that. */
const built = new Map<string, THREE.BufferGeometry>();

/**
 * A mesh for one catalogue id, or `null` for a name this build does not know.
 *
 * `null` rather than a placeholder box: an unknown id is a showfile from a later
 * version of this console naming a piece that did not exist yet, and drawing a
 * mystery cube in the middle of somebody's rig is worse than drawing nothing. A
 * *broken* mesh is a different case and `geometry.ts` still boxes that one, because
 * there the object is known to be there and known to have failed.
 */
export function stockMesh(id: string | null | undefined): THREE.Mesh | null {
	const entry = piece(id);
	if (!entry) return null;

	let geometry = built.get(entry.id);
	if (!geometry) {
		geometry = buildGeometry(entry);
		built.set(entry.id, geometry);
	}
	const mesh = new THREE.Mesh(geometry, materialFor(entry.shape));
	mesh.castShadow = false;
	mesh.receiveShadow = false;
	return mesh;
}

function materialFor(shape: StockShape): THREE.Material {
	return shape === 'BoxTruss' || shape === 'TrussCorner' ? ALUMINIUM : PAINTED;
}

function buildGeometry(entry: StockPiece): THREE.BufferGeometry {
	switch (entry.shape) {
		case 'BoxTruss':
			return boxTruss(entry.size.x, entry.size.y);
		case 'TrussCorner':
			return trussCorner(entry.size.x);
		case 'Deck':
			return deck(entry.size.x, entry.size.y, entry.size.z);
		case 'Panel':
			return panel(entry.size.x, entry.size.y, entry.size.z);
	}
}

/** A chord tube: 50 mm on F34, which is what the ladder is welded out of. */
const CHORD = 0.05;
/** And the bracing between them, 20 mm. */
const BRACE = 0.02;

/**
 * A straight length of box truss, centred on its own origin and running along X.
 *
 * Centred rather than starting at zero because that is what everything else here
 * does — a fixture hangs at an offset from the middle of the truss it is on, and a
 * piece whose origin was at one end would make every one of those offsets a
 * different number depending on how long the section happened to be.
 */
function boxTruss(length: number, square: number): THREE.BufferGeometry {
	const half = square / 2 - CHORD / 2;
	const parts: THREE.BufferGeometry[] = [];

	// The four chords, along X.
	for (const y of [half, -half]) {
		for (const z of [half, -half]) {
			const chord = new THREE.CylinderGeometry(CHORD / 2, CHORD / 2, length, 8);
			// A cylinder stands up Y; these run along X.
			chord.rotateZ(Math.PI / 2);
			chord.translate(0, y, z);
			parts.push(chord);
		}
	}

	// Bracing, as a zig-zag down each of the four faces. One bay every ~250 mm,
	// which is close enough to the real spacing to read correctly and coarse enough
	// that a three-metre section is a few dozen tubes rather than a few hundred.
	const bays = Math.max(2, Math.round(length / 0.25));
	const step = length / bays;
	for (let bay = 0; bay < bays; bay++) {
		const x0 = -length / 2 + bay * step;
		const x1 = x0 + step;
		const up = bay % 2 === 0;
		// The two vertical faces, then the two horizontal ones.
		for (const z of [half, -half]) {
			parts.push(strut({ x: x0, y: up ? half : -half, z }, { x: x1, y: up ? -half : half, z }));
		}
		for (const y of [half, -half]) {
			parts.push(strut({ x: x0, y, z: up ? half : -half }, { x: x1, y, z: up ? -half : half }));
		}
	}

	return mergeGeometries(parts, false) ?? new THREE.BufferGeometry();
}

/** The block that turns one run of truss into another: a cube of chords. */
function trussCorner(square: number): THREE.BufferGeometry {
	const half = square / 2 - CHORD / 2;
	const parts: THREE.BufferGeometry[] = [];
	for (const axis of ['x', 'y', 'z'] as const) {
		for (const a of [half, -half]) {
			for (const b of [half, -half]) {
				const bar = new THREE.CylinderGeometry(CHORD / 2, CHORD / 2, square, 8);
				if (axis === 'x') {
					bar.rotateZ(Math.PI / 2);
					bar.translate(0, a, b);
				} else if (axis === 'z') {
					bar.rotateX(Math.PI / 2);
					bar.translate(a, b, 0);
				} else {
					bar.translate(a, 0, b);
				}
				parts.push(bar);
			}
		}
	}
	return mergeGeometries(parts, false) ?? new THREE.BufferGeometry();
}

/**
 * A rostrum: a top with four legs.
 *
 * Its origin is the *top surface*, not the middle, because a deck is a thing you put
 * something on and where the top is is the number anybody cares about.
 */
function deck(length: number, thickness: number, depth: number): THREE.BufferGeometry {
	const parts: THREE.BufferGeometry[] = [];
	const top = new THREE.BoxGeometry(length, thickness, depth);
	top.translate(0, -thickness / 2, 0);
	parts.push(top);

	const leg = 0.06;
	for (const x of [length / 2 - leg, -(length / 2 - leg)]) {
		for (const z of [depth / 2 - leg, -(depth / 2 - leg)]) {
			// Legs are drawn short and are mostly hidden by whatever the deck stands
			// on; the deck's *height* is where the object was placed, not its size.
			const post = new THREE.BoxGeometry(leg, thickness * 2, leg);
			post.translate(x, -thickness * 2, z);
			parts.push(post);
		}
	}
	return mergeGeometries(parts, false) ?? new THREE.BufferGeometry();
}

/**
 * A flat panel standing on its bottom edge.
 *
 * Origin at the bottom, because a wall is placed on the floor and a flat is placed
 * on the deck, and both of those are the bottom edge.
 */
function panel(width: number, height: number, depth: number): THREE.BufferGeometry {
	const box = new THREE.BoxGeometry(width, height, depth);
	box.translate(0, height / 2, 0);
	return box;
}

/** Two points, joined by a tube. What a brace is. */
function strut(
	from: { x: number; y: number; z: number },
	to: { x: number; y: number; z: number }
): THREE.BufferGeometry {
	const a = new THREE.Vector3(from.x, from.y, from.z);
	const b = new THREE.Vector3(to.x, to.y, to.z);
	const along = b.clone().sub(a);
	const tube = new THREE.CylinderGeometry(BRACE / 2, BRACE / 2, along.length(), 6);
	// A cylinder stands up Y, so turn Y onto the line and put it at the middle.
	const turn = new THREE.Quaternion().setFromUnitVectors(
		new THREE.Vector3(0, 1, 0),
		along.clone().normalize()
	);
	tube.applyQuaternion(turn);
	tube.translate((a.x + b.x) / 2, (a.y + b.y) / 2, (a.z + b.z) / 2);
	return tube;
}
