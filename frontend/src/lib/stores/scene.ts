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
import { derived, writable, type Readable } from 'svelte/store';

import type { Fixture, Layer, SceneObject } from '../generated/index.js';
import { byId } from '../scene.js';
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
