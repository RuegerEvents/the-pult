<script lang="ts">
	/**
	 * The layers of the drawing: what is shown, and what is locked.
	 *
	 * The two are deliberately different kinds of thing. **Shown** is this browser's:
	 * two people looking at one rig should be able to look at different parts of it,
	 * so it lives in a store and nothing about it reaches the show. **Locked** is the
	 * show's, because that is a decision about the rig rather than about a screen.
	 *
	 * Hiding a layer takes its objects and fixtures out of the plan and the rig and
	 * nowhere else: a hidden light still takes a cue and still answers a group.
	 */
	import { getDataContext } from '$lib/ws/context.js';
	import { editing } from '$lib/stores/editing.js';
	import { collection } from '$lib/stores/show.js';
	import {
		hiddenLayers,
		layers,
		sceneObjects,
		showAllLayers,
		toggleLayer
	} from '$lib/stores/scene.js';

	const data = getDataContext();
	const unlocked = editing('layers');
	const fixtures = collection('fixtures');

	const countsFor = (id: string) => ({
		objects: $sceneObjects.filter((object) => object.layer === id).length,
		fixtures: $fixtures.filter((fixture) => fixture.layer === id).length
	});
</script>

<div class="layers">
	{#if $layers.length === 0}
		<p class="empty">
			No layers. Import an MVR and its drawing's layers turn up here.
		</p>
	{:else}
		<nav class="bar">
			<span class="count">{$layers.length} layers</span>
			<span class="spacer"></span>
			{#if $hiddenLayers.size > 0}
				<button class="ghost" onclick={showAllLayers}>Show all</button>
			{/if}
		</nav>
		<ul>
			{#each $layers as layer (layer.id)}
				{@const counts = countsFor(layer.id)}
				<li class:hidden={$hiddenLayers.has(layer.id)}>
					<label class="shown">
						<input
							type="checkbox"
							checked={!$hiddenLayers.has(layer.id)}
							onchange={() => toggleLayer(layer.id)}
						/>
						<span class="name">{layer.name}</span>
					</label>
					<span class="of">
						{counts.fixtures} fixtures, {counts.objects} objects
					</span>
					<!-- Locked is show data, so it is behind the same edit toggle every
					     other write in the console is behind. -->
					{#if $unlocked}
						<label class="lock">
							<input
								type="checkbox"
								checked={layer.locked}
								onchange={(e) =>
									data.layers.byId(layer.id).locked.set(e.currentTarget.checked)}
							/>
							Locked
						</label>
					{:else if layer.locked}
						<span class="lock reading">Locked</span>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.layers { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.bar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; }
	.spacer { flex: 1; }
	.count { color: var(--text-dim); font-size: var(--font-xs); }
	.empty { color: var(--text-dim); font-size: var(--font-sm); margin: auto; padding: 16px; text-align: center; }

	ul { list-style: none; margin: 0; padding: 0; overflow: auto; }
	li { display: flex; align-items: center; gap: 10px; padding: 6px 12px; border-bottom: 1px solid var(--line); }
	li.hidden .name { color: var(--text-dim); }

	.shown { display: flex; align-items: center; gap: 8px; cursor: pointer; flex: 1; min-width: 0; }
	.name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.of { color: var(--text-dim); font-size: var(--font-xs); white-space: nowrap; }
	.lock { display: flex; align-items: center; gap: 5px; color: var(--text-dim); font-size: var(--font-xs); cursor: pointer; }
	.lock.reading { cursor: default; }

	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: var(--text); padding: 4px 10px; font: inherit; font-size: var(--font-xs); cursor: pointer; }
	.ghost:hover { border-color: var(--line-input); }
</style>
