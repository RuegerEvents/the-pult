<script lang="ts">
	/**
	 * Everything in the drawing, as a list.
	 *
	 * `scene_objects` was the one persisted collection with no panel: the Layers panel
	 * counts what is in each layer and never names it, and the rig view can only be
	 * asked by clicking. Which is fine until the piece is behind another one, or on a
	 * layer somebody has hidden, or is a `Group` — which has no geometry at all and so
	 * cannot be clicked on principle.
	 *
	 * **A tree, by parent.** A truss run is a handle with its sections under it, and a
	 * flat list of "Bar 1, Bar 2, Bar 3" beside the group that owns them says nothing
	 * about which run they are. Depth is a walk from the roots rather than a sort,
	 * because an object whose parent is missing has to turn up somewhere and the honest
	 * place is the top.
	 *
	 * Selecting here is the same selection the rig's gizmo drives — one store, so a row
	 * clicked here can be dragged there, and shift-click builds the multiple selection
	 * the align strip already reads.
	 */
	import type { SceneObject } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { editing } from '$lib/stores/editing.js';
	import { piece } from '$lib/stock.js';
	import { clearSelection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import {
		hiddenLayers,
		isLocked,
		layers,
		sceneObjects,
		selectedObjects,
		selectObject,
		toggleObject
	} from '$lib/stores/scene.js';

	const data = getDataContext();
	const unlocked = editing('objects');
	const fixtures = collection('fixtures');

	/** The drawing as a tree, flattened depth-first with the depth kept. */
	const rows = $derived.by(() => {
		const byParent = new Map<string | null, SceneObject[]>();
		const known = new Set($sceneObjects.map((object) => object.id));
		for (const object of $sceneObjects) {
			// An object whose parent has gone is a root: it is still in the show and
			// still somewhere in the room, and hiding it because its truss was deleted
			// would be the one case where the list lies.
			const under = object.parent && known.has(object.parent) ? object.parent : null;
			const siblings = byParent.get(under) ?? [];
			siblings.push(object);
			byParent.set(under, siblings);
		}
		for (const siblings of byParent.values()) {
			siblings.sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
		}

		const out: { object: SceneObject; depth: number }[] = [];
		const walk = (parent: string | null, depth: number) => {
			// A parent chain that loops would otherwise walk for ever, and the drawing
			// is written by peers and plugins as well as by this browser.
			if (depth > 24) return;
			for (const object of byParent.get(parent) ?? []) {
				out.push({ object, depth });
				walk(object.id, depth + 1);
			}
		};
		walk(null, 0);
		return out;
	});

	const layerName = (id: string | null) =>
		id === null ? '—' : ($layers.find((layer) => layer.id === id)?.name ?? 'a layer that has gone');

	/** What it is: the catalogue piece it names, a mesh it carries, or its kind. */
	function what(object: SceneObject): string {
		const entry = piece(object.catalogue);
		if (entry) return entry.title;
		if (object.geometry.length > 0 || object.symbol) return 'Imported mesh';
		return object.kind === 'Group' ? 'Group' : object.kind;
	}

	const fixturesOn = (id: string) => $fixtures.filter((fixture) => fixture.parent === id).length;

	function pick(object: SceneObject, event: MouseEvent) {
		if (event.shiftKey) {
			toggleObject(object.id);
			return;
		}
		selectObject(object.id);
		// The two selections are separate on purpose — `at 50` must never have a truss
		// in scope — so taking hold of a piece lets go of the lights.
		clearSelection();
	}
</script>

<div class="objects">
	{#if rows.length === 0}
		<p class="empty">
			Nothing drawn yet. Open the rig panel's <strong>Pieces</strong> sheet and drag a
			truss in, or import an MVR.
		</p>
	{:else}
		<nav class="bar">
			<span class="count">
				{rows.length}
				{rows.length === 1 ? 'object' : 'objects'}{#if $selectedObjects.size > 0}, {$selectedObjects.size}
					selected{/if}
			</span>
		</nav>
		<ul>
			{#each rows as { object, depth } (object.id)}
				{@const locked = isLocked(object, $layers)}
				{@const lights = fixturesOn(object.id)}
				<li
					class:on={$selectedObjects.has(object.id)}
					class:dimmed={$hiddenLayers.has(object.layer ?? '')}
				>
					<button class="row" style:padding-left="{12 + depth * 14}px" onclick={(e) => pick(object, e)}>
						<span class="name">{object.name || 'Unnamed'}</span>
						<span class="what">{what(object)}</span>
					</button>
					<span class="layer" title="Which layer of the drawing it is on">
						{layerName(object.layer)}
					</span>
					{#if lights > 0}
						<span class="of">{lights} {lights === 1 ? 'light' : 'lights'}</span>
					{/if}
					<!-- Lock is show data, so it is behind the same edit toggle every other
					     write in the console is behind. A locked piece keeps its gizmo away
					     and stays selectable: a piece you cannot pick is a piece whose name
					     and layer you cannot read. -->
					{#if $unlocked}
						<label class="lock">
							<input
								type="checkbox"
								checked={object.locked}
								onchange={(e) => data.scene_objects.byId(object.id).locked.set(e.currentTarget.checked)}
							/>
							Locked
						</label>
					{:else if locked}
						<span class="lock reading">Locked</span>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.objects { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.bar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; }
	.count { color: var(--text-dim); font-size: var(--font-xs); }
	.empty { color: var(--text-dim); font-size: var(--font-sm); margin: auto; padding: 16px; text-align: center; max-width: 40ch; line-height: 1.6; }

	ul { list-style: none; margin: 0; padding: 0; overflow: auto; }
	li { display: flex; align-items: center; gap: 10px; padding-right: 12px; border-bottom: 1px solid var(--line); }
	li.on { background: #232833; }
	/* On a layer this browser has hidden. Still listed — hiding a layer takes its
	   objects out of the *views*, and a list that hid them too would be a list that
	   could not be used to find one again. */
	li.dimmed .name, li.dimmed .what { color: var(--text-dim); }

	.row { display: flex; align-items: baseline; gap: 8px; flex: 1; min-width: 0; background: none; border: 0; color: inherit; font: inherit; text-align: left; padding: 6px 0; cursor: pointer; }
	.row:hover .name { color: #fff; }
	.name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.what { color: var(--text-dim); font-size: var(--font-xs); white-space: nowrap; }
	.layer, .of { color: var(--text-dim); font-size: var(--font-xs); white-space: nowrap; }
	.lock { display: flex; align-items: center; gap: 5px; color: var(--text-dim); font-size: var(--font-xs); cursor: pointer; white-space: nowrap; }
	.lock.reading { cursor: default; }
</style>
