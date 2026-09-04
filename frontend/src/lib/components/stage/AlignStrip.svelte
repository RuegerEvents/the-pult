<script lang="ts">
	/**
	 * Lining several things up.
	 *
	 * Shown only with two or more of one kind selected, because with one there is
	 * nothing to line it up with and a strip of disabled buttons is worse than no
	 * strip.
	 *
	 * **Co-mounted lights operate on `along`**, and everything else in world space.
	 * That is the whole of the rule and it is not a special case: six lanterns clamped
	 * to one bar are spread *along the bar*, whichever way the bar is turned, and
	 * spacing them in world X would take five of them off it.
	 */
	import type { Fixture, SceneObject } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { chordsFor, mountPoint } from '$lib/mount.js';
	import { IDENTITY } from '$lib/scene.js';
	import { piece } from '$lib/stock.js';
	import { asOneAct, distributed, moveObjects, spaced, worldOf } from '$lib/stores/editor.js';
	import { objectsById } from '$lib/stores/scene.js';

	let {
		objects = [],
		fixtures = []
	}: { objects?: SceneObject[]; fixtures?: Fixture[] } = $props();

	const data = getDataContext();

	let by = $state(1);

	/**
	 * The lights that are all clamped to one piece, if they are.
	 *
	 * All of them or none: a selection of four on this bar and two on that one has no
	 * single axis to be spread along, and quietly spreading the four would be a button
	 * doing half of what it said.
	 */
	const onOneBar = $derived.by(() => {
		if (fixtures.length < 2) return null;
		const parent = fixtures[0].parent;
		const clamped = fixtures.every((f) => f.parent === parent && f.mount);
		return clamped && parent ? parent : null;
	});

	const axes = ['x', 'y', 'z'] as const;
	type Axis = (typeof axes)[number];

	/** What the buttons act on: a slide along a bar, or a world axis. */
	let axis = $state<Axis>('x');

	function chordsOf(parent: string) {
		const object = $objectsById.get(parent);
		return chordsFor(piece(object?.catalogue), null);
	}

	async function alongTheBar(place: (values: number[]) => number[]) {
		const parent = onOneBar;
		if (!parent) return;
		const chords = chordsOf(parent);
		const wanted = place(fixtures.map((f) => f.mount!.along));
		await asOneAct(async () => {
			for (const [index, fixture] of fixtures.entries()) {
				const mount = { ...fixture.mount!, along: wanted[index] };
				const row = data.fixtures.byId(fixture.id);
				await row.mount.set(mount);
				await row.position.set({
					...IDENTITY,
					position: mountPoint(mount, chords),
					rotation: { x: mount.roll, y: 0, z: 0 }
				});
			}
		});
	}

	async function inTheWorld(place: (values: number[]) => number[]) {
		const worlds = objects.map((object) => worldOf(object));
		const wanted = place(worlds.map((world) => world.position[axis]));
		await asOneAct(async () => {
			for (const [index, object] of objects.entries()) {
				const world = { ...worlds[index] };
				world.position = { ...world.position, [axis]: wanted[index] };
				// Through the same path a drag takes, so a parented piece is written
				// relative to its parent rather than in world terms.
				await moveObjects([{ id: object.id, world }]);
			}
		});
	}

	const act = (place: (values: number[]) => number[]) =>
		onOneBar ? alongTheBar(place) : inTheWorld(place);

	/** Align to the first: everything takes the value the first one already has. */
	const toFirst = (values: number[]) => values.map(() => values[0]);
</script>

<div class="strip">
	<span class="what">
		{#if onOneBar}
			{fixtures.length} lights on one bar
		{:else}
			{objects.length} pieces
		{/if}
	</span>

	{#if !onOneBar}
		<div class="axes" role="radiogroup" aria-label="Which axis to line up">
			{#each axes as each (each)}
				<button
					class="axis"
					class:on={axis === each}
					role="radio"
					aria-checked={axis === each}
					onclick={() => (axis = each)}
				>
					{each.toUpperCase()}
				</button>
			{/each}
		</div>
	{/if}

	<button class="ghost" onclick={() => void act(distributed)} title="Spread evenly between the two ends">
		Distribute
	</button>
	<label class="by">
		<button class="ghost" onclick={() => void act((values) => spaced(values, by))}>Space by</button>
		<input type="number" step="0.1" bind:value={by} />
		<span class="unit">m</span>
	</label>
	<button
		class="ghost"
		onclick={() => void act(toFirst)}
		title="Everything takes the value the first one already has"
	>
		Align
	</button>
</div>

<style>
	.strip {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 12px;
		border-bottom: 1px solid var(--line);
		flex: none;
		background: #141414;
	}
	.what { color: #777; font-size: 12px; margin-right: 4px; }
	.by { display: flex; align-items: center; gap: 5px; color: #888; font-size: 12px; }
	.by input { width: 6ch; background: #101010; border: 1px solid var(--line-strong); border-radius: 3px; color: #ddd; padding: 3px 5px; font: inherit; font-size: 12px; }
	.unit { color: #666; }
	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover { border-color: var(--line-input); color: #fff; }
	.axes { display: flex; }
	.axis { background: none; border: 1px solid var(--line-strong); color: #999; padding: 4px 9px; font: inherit; font-size: 12px; cursor: pointer; margin-left: -1px; }
	.axis:first-child { border-radius: 3px 0 0 3px; margin-left: 0; }
	.axis:last-child { border-radius: 0 3px 3px 0; }
	.axis.on { background: #2a2f3a; border-color: var(--line-input); color: #fff; }
</style>
