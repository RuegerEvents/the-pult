/**
 * Meshes, loaded once and shared.
 *
 * An imported rig points at its geometry by sha: the archive's files are in the
 * asset store, and `named_assets` remembers what the file that carried them called
 * each one. This is where those become something three.js can draw.
 *
 * Three things it has to get right.
 *
 * **A mesh is loaded once.** A drawing with ninety-five truss sections instances
 * five symbols, and loading a `.glb` per object would fetch the same bytes ninety
 * times and keep ninety copies of it. Keyed by sha, so two objects sharing a mesh
 * share the loaded group as well.
 *
 * **A `.3ds` is Z-up.** The same axis difference as the file formats themselves, and
 * it is applied here and in no other place. glTF is Y-up already and needs nothing.
 *
 * **A file the loader refuses becomes a box.** A rig view that goes blank because one
 * mesh out of two hundred is malformed is worse than a rig view with a box in it, and
 * the box says where the object is, which is most of what the view is for.
 */
import { writable, type Readable } from 'svelte/store';
import * as THREE from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { TDSLoader } from 'three/examples/jsm/loaders/TDSLoader.js';

import type { NamedAsset } from './generated/index.js';

/** A loaded mesh, and how big it is. */
export type Loaded = {
	/** Ready to be cloned into the scene. Never mutate it: it is shared. */
	object: THREE.Object3D;
	/** Its extent in metres, for the plan view and for framing. */
	size: THREE.Vector3;
	/** True when the file could not be read and this is a stand-in. */
	placeholder: boolean;
};

/**
 * What size a placeholder is, in metres.
 *
 * A metre cube: big enough to see, small enough not to look like a wall, and
 * obviously not the truss it stands in for.
 */
export const PLACEHOLDER_SIZE = 1;

const loaded = new Map<string, Promise<Loaded>>();

/**
 * How big each loaded mesh is, by sha.
 *
 * The plan view draws an object as its footprint and has no three.js scene to measure
 * one in, so the size arrives here as meshes load and the drawing fills in. Until it
 * does, an object is a default square — which is honest: the console does not yet
 * know how big that truss is.
 */
const measured = writable<Map<string, THREE.Vector3>>(new Map());
export const meshSizes: Readable<Map<string, THREE.Vector3>> = measured;

/** Start loading these, for a view that wants their sizes rather than their meshes. */
export function measureAll(refs: { asset: string; file_name: string }[], names: NamedAsset[]) {
	for (const reference of refs) {
		void load(reference.asset, reference.file_name, names);
	}
}

/** Where the console serves an asset from. Same origin, like everything else. */
const assetUrl = (sha: string) => `/assets/${sha}`;

/**
 * A loading manager that resolves the names inside a mesh.
 *
 * A `.3ds` asks for its texture as `tx603.jpg` and nothing else; the store has no
 * names in it. `named_assets` is the bridge, and three.js's own URL modifier is where
 * it is crossed — so nothing below this line knows that assets are content-addressed.
 */
function managerFor(names: NamedAsset[]): THREE.LoadingManager {
	const manager = new THREE.LoadingManager();
	manager.setURLModifier((url) => resolveAssetUrl(url, names));
	return manager;
}

/**
 * What a mesh asking for a file by name should actually fetch.
 *
 * The rule the paragraph above is about, on its own so it can be tested without a
 * scene: a name the archive carried becomes the asset it was stored as, and anything
 * else is left alone.
 */
export function resolveAssetUrl(url: string, names: NamedAsset[]): string {
	// Already ours, or a data URI a glTF embedded: leave it alone.
	if (url.startsWith('/assets/') || url.startsWith('data:') || url.startsWith('blob:')) {
		return url;
	}
	const bySha = new Map(names.map((n) => [n.name, n.asset]));
	const name = url.split('/').pop() ?? url;
	const sha = bySha.get(name) ?? bySha.get(safelyDecoded(name));
	return sha ? assetUrl(sha) : url;
}

/** `decodeURIComponent` refuses a lone `%`, and a texture may well be called one. */
function safelyDecoded(name: string): string {
	try {
		return decodeURIComponent(name);
	} catch {
		return name;
	}
}

/** A metre cube in a colour that says "this is not the mesh you asked for". */
function placeholder(): Loaded {
	const mesh = new THREE.Mesh(
		new THREE.BoxGeometry(PLACEHOLDER_SIZE, PLACEHOLDER_SIZE, PLACEHOLDER_SIZE),
		new THREE.MeshStandardMaterial({ color: '#5a4a2a', roughness: 0.9, side: THREE.DoubleSide })
	);
	return {
		object: mesh,
		size: new THREE.Vector3(PLACEHOLDER_SIZE, PLACEHOLDER_SIZE, PLACEHOLDER_SIZE),
		placeholder: true
	};
}

/** Its extent, once it is loaded and in the space the console draws in. */
function measure(object: THREE.Object3D): THREE.Vector3 {
	const box = new THREE.Box3().setFromObject(object);
	const size = new THREE.Vector3();
	box.getSize(size);
	// A mesh with no vertices measures as infinite; a metre is a better lie.
	if (!Number.isFinite(size.x) || !Number.isFinite(size.y) || !Number.isFinite(size.z)) {
		return new THREE.Vector3(PLACEHOLDER_SIZE, PLACEHOLDER_SIZE, PLACEHOLDER_SIZE);
	}
	return size;
}

/**
 * Load one mesh, or hand back the one already loaded.
 *
 * `fileName` decides which loader, because that is all an archive entry says about
 * what it holds.
 */
export function load(sha: string, fileName: string, names: NamedAsset[]): Promise<Loaded> {
	return loadFrom(sha, assetUrl(sha), fileName, names);
}

/**
 * The same, from anywhere.
 *
 * The catalogue's pieces are generated by the station and served from `/stock/…`
 * rather than out of the content-addressed store, and they want exactly this cache
 * and exactly this cloning — a hundred truss sections are one download and one
 * upload. So the key is separated from the URL rather than a second loader being
 * written: `key` is what two callers share when they want the same mesh, and for an
 * asset that is its sha and for a stock piece it is the URL, which carries the piece
 * and everything it was asked for.
 */
export function loadFrom(
	key: string,
	url: string,
	fileName: string,
	names: NamedAsset[]
): Promise<Loaded> {
	const already = loaded.get(key);
	if (already) return already;

	const promise = read(url, fileName, names).catch(() => placeholder());
	loaded.set(key, promise);
	void promise.then((mesh) => {
		measured.update((sizes) => new Map(sizes).set(key, mesh.size));
	});
	return promise;
}

async function read(url: string, fileName: string, names: NamedAsset[]): Promise<Loaded> {
	const manager = managerFor(names);
	const extension = fileName.toLowerCase().split('.').pop() ?? '';

	if (extension === '3ds') {
		const object = await new TDSLoader(manager).loadAsync(url);
		// The one place the axis difference is applied. 3DS is Z-up, like the formats
		// it comes out of; the console is Y-up.
		object.rotateX(-Math.PI / 2);
		object.updateMatrixWorld(true);
		return { object, size: measure(object), placeholder: false };
	}
	if (extension === 'glb' || extension === 'gltf') {
		const gltf = await new GLTFLoader(manager).loadAsync(url);
		return { object: gltf.scene, size: measure(gltf.scene), placeholder: false };
	}
	throw new Error(`nothing here draws a ${extension}`);
}

/** Forget everything loaded. For tests, and for a show being closed. */
export function forgetLoadedMeshes() {
	loaded.clear();
	measured.set(new Map());
}

/**
 * A copy of a loaded mesh, ready to be put in the scene.
 *
 * `clone` shares geometry and materials, which is what makes ninety-five truss
 * sections cost one mesh — so a mirrored instance gets its own material rather than
 * turning back-face culling off for every copy of the same truss. Negative scale
 * reverses winding, and a mirrored object drawn with culling on is inside out.
 */
export function instance(mesh: Loaded, mirrored: boolean): THREE.Object3D {
	const copy = mesh.object.clone(true);
	if (!mirrored) return copy;
	copy.traverse((child) => {
		const drawn = child as THREE.Mesh;
		if (!drawn.isMesh) return;
		const material = drawn.material;
		drawn.material = Array.isArray(material)
			? material.map((each) => twoSided(each))
			: twoSided(material);
	});
	return copy;
}

function twoSided(material: THREE.Material): THREE.Material {
	const copy = material.clone();
	copy.side = THREE.DoubleSide;
	return copy;
}
