/**
 * Building a rig: the writes, and nothing that draws.
 *
 * Everything an operator does to the drawing — put a piece in, move it, copy it,
 * throw it away, clamp a light to it — is a sequence of ordinary path writes inside
 * one gesture. There is no engine command for any of it, deliberately: a duplicate is
 * a create per row and a delete-with-children is a delete per row, and both of those
 * are things the schema already knows how to do. What makes each of them *one* act is
 * `gesture.ts`, which is also what makes each of them one Ctrl-Z.
 *
 * The one exception to "nothing that draws" is the delete prompt's own state, which is
 * here because it is the *question a write asks* rather than a thing a panel owns: the
 * verb can be reached from the tools panel and from the rig's own Delete key, and a
 * modal that belonged to one of them would be missing from the other.
 *
 * The one thing that is not obvious: **a mount and a placement are written together**.
 * The station cannot resolve a mount on a truss that came out of a drawing — that
 * needs the mesh's bounds and only the browser measures a mesh — so the browser is the
 * writer for every parent, and a write that set one without the other would leave a
 * light whose two halves disagreed. See `Fixture::mount`.
 */

import { get, writable } from 'svelte/store';

import type { Fixture, FixtureType, Mount, SceneObject, Transform, Vec3 } from '$lib/generated/index.js';
import { dmxAddress, fixtureMode, footprint, nextFreeAddress } from '$lib/patch.js';
import { canonicalProperties, piece } from '$lib/stock.js';
import { IDENTITY, localOf, parentWorld, worldTransform } from '$lib/scene.js';
import { beginGesture, endGesture } from './gesture.js';
import { collection, showData } from './show.js';
import { layers, sceneObjects } from './scene.js';

/** What a new layer is called when a show that has none gets its first piece. */
export const DEFAULT_LAYER = 'Stage';

/**
 * Everything between here and the end is one act.
 *
 * A thin wrapper, but it is the one every verb below goes through — and the reason it
 * is a wrapper rather than a pair of calls at each site is that an exception between
 * them would leave the gesture open, and the next unrelated write would join it.
 */
export async function asOneAct<T>(work: () => Promise<T>): Promise<T> {
	beginGesture();
	try {
		return await work();
	} finally {
		endGesture();
	}
}

/**
 * The layer a new piece goes in.
 *
 * A show that has never seen a drawing has no layers at all, and a rig whose pieces
 * are all on no layer is a rig the Layers panel cannot show or hide. So the first
 * placement makes one — inside the same gesture as the piece itself, so that taking
 * the piece back takes the layer with it rather than leaving an empty one behind.
 */
export async function stageLayer(): Promise<string> {
	const existing = get(layers);
	if (existing.length > 0) return existing[0].id;

	const id = crypto.randomUUID();
	await showData().layers.create({
		id,
		name: DEFAULT_LAYER,
		locked: false,
		sort_order: 0
	});
	return id;
}

/** Put one catalogue piece in the room, and answer its id. */
export async function placePiece(
	catalogueId: string,
	transform: Transform,
	options: { parent?: string | null; name?: string; properties?: unknown } = {}
): Promise<string | null> {
	const entry = piece(catalogueId);
	if (!entry) return null;

	return asOneAct(async () => {
		const layer = await stageLayer();
		const id = crypto.randomUUID();
		await showData().scene_objects.create({
			id,
			name: options.name ?? nextName(entry.title),
			kind: entry.kind,
			transform,
			parent: options.parent ?? null,
			layer,
			class: null,
			geometry: [],
			symbol: null,
			catalogue: entry.id,
			properties: canonicalProperties(entry, options.properties),
			locked: false
		});
		return id;
	});
}

/**
 * A name nothing else in the show has yet.
 *
 * "F34 truss 2 m 3", which is what a person would write on a plan. Not unique by
 * construction — two browsers placing at once can pick the same number — and
 * deliberately not: a name is a label an operator reads and edits, and the id is what
 * anything actually matches on.
 */
function nextName(title: string): string {
	const taken = get(sceneObjects).filter((object) => object.name.startsWith(title));
	return taken.length === 0 ? title : `${title} ${taken.length + 1}`;
}

/**
 * Move several objects at once, given where each has got to in world terms.
 *
 * The gizmo speaks in world placements and a `SceneObject.transform` is relative to
 * its parent, so every one of these goes through `localOf`. Sixty frames of a drag are
 * sixty of these calls and one row in the history, which
 * `crates/pult-backend/tests/counts.rs` asserts.
 */
export async function moveObjects(moves: { id: string; world: Transform }[]): Promise<void> {
	const objects = byIdNow();
	const data = showData();
	await Promise.all(
		moves.map(({ id, world }) => {
			const object = objects.get(id);
			if (!object) return Promise.resolve();
			const local = localOf(world, parentWorld(object.parent, objects));
			return data.scene_objects.byId(id).transform.set(local);
		})
	);
}

/** The same for fixtures, which move on the same gizmo and have no scale. */
export async function moveFixtures(moves: { id: string; world: Transform }[]): Promise<void> {
	const objects = byIdNow();
	const fixtures = get(collection('fixtures'));
	const data = showData();
	await Promise.all(
		moves.map(({ id, world }) => {
			const fixture = fixtures.find((each) => each.id === id);
			if (!fixture) return Promise.resolve();
			const local = localOf(world, parentWorld(fixture.parent, objects));
			return data.fixtures.byId(id).position.set(local);
		})
	);
}

/**
 * Clamp a light to a piece, or let it go.
 *
 * Both halves in one write pair: the mount says which chord and how far along, the
 * position is where that comes to. A caller passing `null` for the mount is dragging
 * the light *off* the truss, and then the placement it already has in the world is
 * what it keeps — a light that fell to the origin when it stopped being clamped would
 * be a light somebody had to find again.
 */
export async function clampFixture(
	id: string,
	parent: string | null,
	mount: Mount | null,
	local: Transform
): Promise<void> {
	const fixture = showData().fixtures.byId(id);
	await asOneAct(async () => {
		await fixture.parent.set(parent);
		await fixture.mount.set(mount);
		await fixture.position.set(local);
	});
}

// ── Copying and throwing away ─────────────────────────────────────────────────

/** Every object hanging off these, and off those, all the way down. */
export function subtreeOf(ids: Iterable<string>, objects: SceneObject[]): Set<string> {
	const found = new Set(ids);
	let growing = true;
	while (growing) {
		growing = false;
		for (const object of objects) {
			if (object.parent && found.has(object.parent) && !found.has(object.id)) {
				found.add(object.id);
				growing = true;
			}
		}
	}
	return found;
}

/** And the fixtures hanging off any of them. */
export function fixturesOn(ids: Set<string>, fixtures: Fixture[]): Fixture[] {
	return fixtures.filter((fixture) => fixture.parent && ids.has(fixture.parent));
}

/**
 * Copy a selection, its children and its lights, one grid step over.
 *
 * **The copies are patched after the rest, not on top of it.** Two fixtures at one
 * address is a rig where half the lights do the wrong thing and nothing says why, and
 * a duplicate is exactly the gesture somebody uses to rough out a second bar. So the
 * copy keeps the type, the place and the mount, takes the next free address in its own
 * universe, and loses the numbers on its label — which are the operator's to decide.
 *
 * Answers the new objects' ids, so the caller can leave them selected: a duplicate you
 * then have to go and find is a duplicate you drag the original of by mistake.
 */
export async function duplicateObjects(ids: Set<string>, step: number): Promise<string[]> {
	const objects = get(sceneObjects);
	const fixtures = get(collection('fixtures'));
	const types = get(collection('fixture_types'));
	const wanted = subtreeOf(ids, objects);
	const copying = objects.filter((object) => wanted.has(object.id));
	if (copying.length === 0) return [];

	// Fresh ids first, so a child's `parent` can point at its copy rather than at the
	// original — which is what makes a copied run move as a run.
	const fresh = new Map(copying.map((object) => [object.id, crypto.randomUUID()]));
	const over = step > 0 ? step : 0.5;

	return asOneAct(async () => {
		const data = showData();
		for (const object of copying) {
			// Only the *roots* of what was copied move over: a child is placed relative
			// to its parent, and shifting both would move it twice.
			const shifted = ids.has(object.id) && !fresh.has(object.parent ?? '');
			await data.scene_objects.create({
				...object,
				id: fresh.get(object.id)!,
				parent: object.parent ? (fresh.get(object.parent) ?? object.parent) : null,
				transform: shifted ? nudged(object.transform, over) : object.transform
			});
		}
		// The copies are addressed after everything already in the show. Two fixtures
		// at one address is a rig where half the lights do the wrong thing and nothing
		// says why, and a duplicate is exactly the gesture somebody uses to rough out a
		// second bar. There is no "unpatched" for an address to be, so the next free
		// one in the same universe is the honest answer — and the numbers on the label
		// are cleared, because those are the operator's to decide.
		const patched = [...fixtures];
		for (const fixture of fixturesOn(wanted, fixtures)) {
			const copy: Fixture = {
				...fixture,
				id: crypto.randomUUID(),
				parent: fresh.get(fixture.parent!) ?? fixture.parent,
				address: afterTheRest(fixture, patched, types),
				fixture_number: null,
				unit_number: null
			};
			patched.push(copy);
			await data.fixtures.create(copy);
		}
		return [...ids].map((id) => fresh.get(id)!).filter(Boolean);
	});
}

/**
 * Where a copy of this fixture is patched.
 *
 * After everything else in its own universe, rolling into the next when one is full —
 * which is what a patch head does, and what makes copying a bar of twelve give twelve
 * addresses in a row. A fixture on an OpenHaunt node keeps its address, because that
 * address is a *serial*: there is no next one, and inventing a node that is not on the
 * network would be worse than a copy an operator has to repatch by hand.
 */
function afterTheRest(
	fixture: Fixture,
	taken: Fixture[],
	types: FixtureType[]
): Fixture['address'] {
	const dmx = dmxAddress(fixture.address);
	const mode = fixtureMode(fixture.address);
	if (!dmx || mode === null) return fixture.address;

	const type = types.find((each) => each.id === fixture.fixture_type_id);
	const span = (each: Fixture) => {
		const its = types.find((t) => t.id === each.fixture_type_id);
		return Math.max(...footprint(its, each.address), 1);
	};
	const channels = Math.max(...footprint(type, fixture.address), 1);

	let universe = dmx.universe;
	let address = nextFreeAddress(taken, universe, span);
	if (address + channels > 513) {
		universe += 1;
		address = nextFreeAddress(taken, universe, span);
	}
	return { Dmx: { mode, breaks: [{ universe, address }] } };
}

function nudged(transform: Transform, by: number): Transform {
	return { ...transform, position: { ...transform.position, x: transform.position.x + by } };
}

/**
 * The question the delete prompt is asking, or nothing.
 *
 * Mounted once at the root of the app rather than inside a tile, because a modal
 * belongs to the window: a prompt drawn inside a small panel would be a dialogue in a
 * corner, and one drawn inside the *rig* would be missing whenever somebody deleted
 * from the objects list.
 */
export const deleteAsk = writable<{
	name: string;
	ids: Set<string>;
	objects: SceneObject[];
	fixtures: Fixture[];
} | null>(null);

/**
 * Throw a selection away, asking first when there is something on it.
 *
 * A bare piece goes without being asked about; one with lights or other pieces on it
 * asks, because "delete them too" and "leave them where they are" are both things
 * people mean and no console can tell which.
 */
export function askToDelete(going: SceneObject[]): void {
	if (going.length === 0) return;
	const ids = new Set(going.map((object) => object.id));
	const what = whatWouldGo(ids, get(sceneObjects), get(collection('fixtures')));
	if (what.objects.length === 0 && what.fixtures.length === 0) {
		void deleteObjects(ids, false);
		return;
	}
	deleteAsk.set({ name: going[0].name || 'This piece', ids, ...what });
}

/** And the answer. `null` is Cancel. */
export function answerDelete(keepChildren: boolean | null): void {
	const asking = get(deleteAsk);
	deleteAsk.set(null);
	if (!asking || keepChildren === null) return;
	void deleteObjects(asking.ids, keepChildren);
}

/** What deleting a selection would take with it, for the prompt to name. */
export function whatWouldGo(
	ids: Set<string>,
	objects: SceneObject[],
	fixtures: Fixture[]
): { objects: SceneObject[]; fixtures: Fixture[] } {
	const wanted = subtreeOf(ids, objects);
	return {
		objects: objects.filter((object) => wanted.has(object.id) && !ids.has(object.id)),
		fixtures: fixturesOn(wanted, fixtures)
	};
}

/**
 * Throw a selection away.
 *
 * `keepChildren` is the answer to the question the prompt asks, and it is a real
 * choice rather than a confirmation: deleting a bar because it was the wrong length
 * should not take six lanterns with it, and deleting a truss because the whole thing
 * is gone should. Keeping them means giving each one the placement it had in the
 * world, so they stay exactly where they were and simply stop hanging off anything —
 * and losing its mount with it, because it is no longer clamped to anything.
 */
export async function deleteObjects(ids: Set<string>, keepChildren: boolean): Promise<void> {
	const objects = get(sceneObjects);
	const fixtures = get(collection('fixtures'));
	const byId = byIdNow();
	const going = keepChildren ? new Set(ids) : subtreeOf(ids, objects);

	await asOneAct(async () => {
		const data = showData();
		if (keepChildren) {
			for (const child of objects.filter((o) => o.parent && ids.has(o.parent))) {
				const entity = data.scene_objects.byId(child.id);
				await entity.transform.set(worldTransform(child.transform, child.parent, byId));
				await entity.parent.set(null);
			}
			for (const fixture of fixturesOn(ids, fixtures)) {
				const entity = data.fixtures.byId(fixture.id);
				const world = worldTransform(fixture.position ?? IDENTITY, fixture.parent, byId);
				await entity.position.set(world);
				await entity.parent.set(null);
				await entity.mount.set(null);
			}
		} else {
			for (const fixture of fixturesOn(going, fixtures)) {
				await data.fixtures.byId(fixture.id).delete();
			}
		}
		for (const id of going) {
			await data.scene_objects.byId(id).delete();
		}
	});
}

// ── Lining things up ──────────────────────────────────────────────────────────

export type AlignAxis = 'x' | 'y' | 'z';

/**
 * Where several things end up when they are spread out evenly.
 *
 * Pure, and separated from the writing so it can be tested: the ends stay where they
 * are and everything between them is spaced equally along the axis, which is what
 * "distribute" means everywhere it appears. Fewer than three is already distributed.
 */
export function distributed(points: number[]): number[] {
	if (points.length < 3) return points;
	const order = points.map((value, index) => ({ value, index })).sort((a, b) => a.value - b.value);
	const first = order[0].value;
	const step = (order[order.length - 1].value - first) / (order.length - 1);
	const out = [...points];
	order.forEach((each, n) => {
		out[each.index] = first + step * n;
	});
	return out;
}

/** And when they are spaced a stated distance apart, in the order they are in now. */
export function spaced(points: number[], by: number): number[] {
	if (points.length === 0) return points;
	const order = points.map((value, index) => ({ value, index })).sort((a, b) => a.value - b.value);
	const first = order[0].value;
	const out = [...points];
	order.forEach((each, n) => {
		out[each.index] = first + by * n;
	});
	return out;
}

/** The objects of the show as a map, now — what a chain is composed through. */
function byIdNow(): Map<string, SceneObject> {
	return new Map(get(sceneObjects).map((object) => [object.id, object]));
}

/** Where a placement is in the world, for a caller that has only the row. */
export function worldOf(object: { transform: Transform; parent: string | null }): Transform {
	return worldTransform(object.transform, object.parent, byIdNow());
}

/** A point moved to a grid-snapped place, which is what a drop does. */
export function placedAt(point: Vec3): Transform {
	return { ...IDENTITY, position: point };
}
