/**
 * The drawing: what is in it, and which parts of it this browser is looking at.
 *
 * Layer *visibility* is per browser and lives here rather than in the show, because
 * two people looking at one rig should be able to look at different parts of it — one
 * working on the overhead truss while the other is on the floor. Whether a layer is
 * **locked** is the show's, since that is a decision about the rig.
 *
 * Hiding a layer takes its objects and fixtures out of the plan and the rig views and
 * nowhere else. A hidden fixture still takes a cue, still answers a group, and is
 * still in the patch: a light that is on and invisible in every panel is a support
 * call, not a feature.
 */
import { derived, get, writable, type Readable } from 'svelte/store';

import type { Fixture, Layer, SceneObject, Vec3 } from '../generated/index.js';
import { byId, worldTransform } from '../scene.js';
import { collection } from './show.js';

export const layers: Readable<Layer[]> = derived(collection('layers'), ($layers) =>
	[...$layers].sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name))
);

export const sceneObjects: Readable<SceneObject[]> = collection('scene_objects');
export const symbols = collection('symbols');
export const namedAssets = collection('named_assets');

/**
 * The layers this browser has hidden, by id.
 *
 * Hidden rather than shown, so a layer somebody else adds arrives visible: a rig
 * with a truss in it nobody can see is the more surprising of the two mistakes.
 */
export const hiddenLayers = writable<Set<string>>(new Set());

export function toggleLayer(id: string) {
	hiddenLayers.update((hidden) => {
		const next = new Set(hidden);
		if (!next.delete(id)) next.add(id);
		return next;
	});
}

export function showAllLayers() {
	hiddenLayers.set(new Set());
}

/** Whether the views should draw something in this layer. */
export function isVisible(layer: string | null, hidden: Set<string>): boolean {
	return layer === null || !hidden.has(layer);
}

/** The objects to draw, with hidden layers taken out. */
export const visibleObjects: Readable<SceneObject[]> = derived(
	[sceneObjects, hiddenLayers],
	([$objects, $hidden]) => $objects.filter((object) => isVisible(object.layer, $hidden))
);

/**
 * Every object, whether visible or not, keyed by id.
 *
 * What a parent chain is walked through — and it is deliberately *not* filtered by
 * visibility: a light hangs where its truss is whether or not the truss is being
 * drawn, and composing through a hidden parent as though it were the origin would
 * move the rig when somebody ticked a box.
 */
export const objectsById = derived(sceneObjects, ($objects) => byId($objects));

/** Whether the views should draw this fixture. */
export function fixtureIsVisible(fixture: Fixture, hidden: Set<string>): boolean {
	return isVisible(fixture.layer, hidden);
}

// ── What is selected in the drawing ───────────────────────────────────────────
//
// Its own store beside the fixture selection, deliberately, and it is the decision
// the whole editor rests on: a `SelectionQuery` is a question about the *rig* — "every
// mover on the downstage truss" — and `at 50` means the fixtures it answers. A truss
// in that scope would be a truss an operator could accidentally put at fifty percent.
// Two selections, one gesture: clicking a truss clears the fixtures and clicking a
// light clears the objects, because a gizmo has to know which of them it is on.

/** The objects an operator has hold of, by id. */
export const selectedObjects = writable<Set<string>>(new Set());

/**
 * What the drawing's gizmo does: move, turn or resize.
 *
 * A store rather than a panel's own state, because the buttons that set it and the
 * viewer that obeys it are two different panels now — and because two rig tiles open
 * at once should not be in two different modes, any more than they are at two
 * different work heights.
 *
 * **Scale is objects only.** A fixture is a real thing of a real size, and a rig with
 * a lantern in it at 1.4× would be a rig whose paperwork lies about what is on the bar.
 */
export const gizmoMode = writable<'translate' | 'rotate' | 'scale'>('translate');

/**
 * Where a multiple selection turns and scales about.
 *
 * **Operator-placed**, which is the whole point: rotating four trusses about their
 * own average centre is almost never the move, and rotating them about the corner
 * where they meet almost always is. It starts at the selection's centre so there is
 * always something to grab, snaps to the grid, and is forgotten the moment the
 * selection changes — a pivot that outlived the thing it was set for would be a
 * handle in the middle of nowhere.
 *
 * Shown for one object too, because "turn this truss about its far end" is the same
 * gesture and it would be strange for it to appear only at two.
 */
export const pivot = writable<Vec3 | null>(null);

/** Take hold of one object and let go of everything else. */
export function selectObject(id: string) {
	selectedObjects.set(new Set([id]));
	pivot.set(null);
}

/** Add one, or drop it if it was already held. Shift-click. */
export function toggleObject(id: string) {
	selectedObjects.update((held) => {
		const next = new Set(held);
		if (!next.delete(id)) next.add(id);
		return next;
	});
	pivot.set(null);
}

/** Take hold of several at once — what the align strip and a duplicate leave. */
export function selectObjects(ids: string[]) {
	selectedObjects.set(new Set(ids));
	pivot.set(null);
}

export function clearObjects() {
	if (get(selectedObjects).size === 0) return;
	selectedObjects.set(new Set());
	pivot.set(null);
}

/**
 * Where the pivot actually is: what an operator put it at, or the middle of what is
 * selected.
 *
 * Composed through `worldTransform`, so the pivot of a selection that includes a
 * light on a truss is where that light *is* rather than where its row says it is.
 */
export function pivotPoint(
	placed: Vec3 | null,
	ids: Set<string>,
	objects: Map<string, SceneObject>
): Vec3 | null {
	if (placed) return placed;
	const points = [...ids]
		.map((id) => objects.get(id))
		.filter((object): object is SceneObject => !!object)
		.map((object) => worldTransform(object.transform, object.parent, objects).position);
	if (points.length === 0) return null;
	const sum = points.reduce((a, p) => ({ x: a.x + p.x, y: a.y + p.y, z: a.z + p.z }), {
		x: 0,
		y: 0,
		z: 0
	});
	return { x: sum.x / points.length, y: sum.y / points.length, z: sum.z / points.length };
}

/** Whether an operator can take hold of this one. A locked piece still picks. */
export function isLocked(object: SceneObject | undefined, layers: Layer[]): boolean {
	if (!object) return false;
	return object.locked || layers.some((layer) => layer.id === object.layer && layer.locked);
}
