<script lang="ts">
	/**
	 * What an operator does *to* the drawing: a panel rather than a strip of buttons
	 * over the rig.
	 *
	 * It started on the rig's own toolbar and outgrew it. A toolbar that wraps to two
	 * rows takes the height away from the thing it is a toolbar for, and every sheet
	 * that opened out of it moved the picture underneath — which is the one thing a
	 * view somebody is aiming a pointer at must not do. So the verbs live in a tile
	 * like everything else, and the rig panel keeps only what is about *looking*: where
	 * the camera stands, and how the picture is drawn.
	 *
	 * It reads the same stores the viewer does, so it works whether or not a rig tile
	 * is open and however many there are.
	 */
	import { collection } from '$lib/stores/show.js';
	import { selected } from '$lib/stores/selection.js';
	import { view } from '$lib/stores/view.js';
	import {
		gizmoMode,
		isLocked,
		layers,
		objectsById,
		selectedObjects,
		selectObjects
	} from '$lib/stores/scene.js';
	import { askToDelete, duplicateObjects } from '$lib/stores/editor.js';
	import MvrButtons from './MvrButtons.svelte';
	import AlignStrip from './AlignStrip.svelte';

	const fixtures = collection('fixtures');

	const held = $derived(
		[...$selectedObjects]
			.map((id) => $objectsById.get(id))
			.filter((object): object is NonNullable<typeof object> => !!object)
	);
	/** Of those, the ones an operator may actually change. */
	const editable = $derived(held.filter((object) => !isLocked(object, $layers)));
	const heldFixtures = $derived($fixtures.filter((fixture) => $selected.has(fixture.id)));

	async function duplicate() {
		if (editable.length === 0) return;
		const copies = await duplicateObjects(
			new Set(editable.map((object) => object.id)),
			$view.grid || 0.5
		);
		// Left selected, because a duplicate you then have to go and find is a
		// duplicate you drag the original of by mistake.
		if (copies.length > 0) selectObjects(copies);
	}
</script>

<div class="tools">
	<section>
		<h3>Rig</h3>
		<div class="row">
			<MvrButtons />
		</div>
	</section>

	<section>
		<h3>Gizmo</h3>
		<div class="row" role="radiogroup" aria-label="What the gizmo does">
			<button
				class="seg"
				class:on={$gizmoMode === 'translate'}
				role="radio"
				aria-checked={$gizmoMode === 'translate'}
				disabled={editable.length === 0}
				title="Slide it along the axes"
				onclick={() => gizmoMode.set('translate')}>Move</button
			>
			<button
				class="seg"
				class:on={$gizmoMode === 'rotate'}
				role="radio"
				aria-checked={$gizmoMode === 'rotate'}
				disabled={editable.length === 0}
				title="Turn it about the pivot, which drags where you want it"
				onclick={() => gizmoMode.set('rotate')}>Turn</button
			>
			<button
				class="seg"
				class:on={$gizmoMode === 'scale'}
				role="radio"
				aria-checked={$gizmoMode === 'scale'}
				disabled={editable.length === 0}
				title="Objects only: a fixture is a real thing of a real size"
				onclick={() => gizmoMode.set('scale')}>Size</button
			>
		</div>
	</section>

	<section>
		<h3>Selection</h3>
		<div class="row">
			<button
				class="ghost"
				disabled={editable.length === 0}
				title="Copy this and everything on it, one grid step over — the copies patched after the rest"
				onclick={() => void duplicate()}>Duplicate</button
			>
			<button
				class="ghost"
				disabled={editable.length === 0}
				title="Throw it away. Anything hanging off it asks first."
				onclick={() => askToDelete(editable)}>Delete</button
			>
		</div>
		<p class="what">
			{#if held.length === 0}
				Nothing picked. Click a piece in the rig, or a row in Objects.
			{:else}
				{held.length}
				{held.length === 1 ? 'piece' : 'pieces'}{#if editable.length < held.length}, {held.length -
					editable.length} locked{/if}
			{/if}
		</p>
	</section>

	{#if editable.length >= 2 || heldFixtures.length >= 2}
		<section class="wide">
			<h3>Line up</h3>
			<AlignStrip
				objects={editable.length >= 2 ? editable : []}
				fixtures={editable.length >= 2 ? [] : heldFixtures}
			/>
		</section>
	{/if}
</div>

<style>
	.tools { display: flex; flex-wrap: wrap; align-items: flex-start; gap: 12px 24px; padding: 10px 12px; }
	section { display: flex; flex-direction: column; gap: 6px; }
	section.wide { flex-basis: 100%; }
	h3 { margin: 0; color: var(--text-dim); font-size: var(--font-xs); font-weight: 500; text-transform: uppercase; letter-spacing: 0.04em; }
	.row { display: flex; align-items: center; gap: 8px; }
	.what { margin: 2px 0 0; color: var(--text-dim); font-size: var(--font-xs); max-width: 34ch; line-height: 1.5; }

	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: var(--text); padding: 4px 10px; font: inherit; font-size: var(--font-xs); cursor: pointer; }
	.ghost:hover:not(:disabled) { border-color: var(--line-input); }
	.ghost:disabled { color: var(--text-dim); border-color: var(--line); cursor: default; }

	.seg { background: none; border: 1px solid var(--line-strong); color: var(--text-dim); padding: 4px 11px; font: inherit; font-size: var(--font-xs); cursor: pointer; margin-left: -1px; }
	.seg:first-of-type { border-radius: 3px 0 0 3px; margin-left: 0; }
	.seg:last-of-type { border-radius: 0 3px 3px 0; }
	.seg:hover:not(:disabled) { color: var(--text); }
	.seg.on { background: #2a2f3a; border-color: var(--line-input); color: #fff; }
	.seg:disabled { color: #555; cursor: default; }
</style>
