<script lang="ts">
	/**
	 * What is selected, in the order it was selected.
	 *
	 * The order is not decoration. It is what an effect will eventually spread along,
	 * so "the third fixture" has to be something the operator decides rather than
	 * something the patch decided — which is why the spec asks for this list to be
	 * reorderable by hand.
	 *
	 * Selection is deliberately separate from the programmer: clearing one leaves the
	 * other alone, so a value can be parked and then reached again from somewhere else.
	 */

	import { clearSelection, remove, reorder, selection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { fixtureTint } from '$lib/stage.js';

	const fixtures = collection('fixtures');

	/// The selection in its own order, skipping anything that has left the rig.
	const chosen = $derived(
		$selection.map((id) => $fixtures.find((f) => f.id === id)).filter((f) => f !== undefined)
	);

	let dragging = $state<number | null>(null);
	let over = $state<number | null>(null);

	function drop() {
		if (dragging !== null && over !== null) reorder(dragging, over);
		dragging = null;
		over = null;
	}
</script>

<div class="panel">
	<header>
		<h2>Selection</h2>
		<span class="count">{$selection.length}</span>
		<span class="spacer"></span>
		<button class="ghost" disabled={$selection.length === 0} onclick={clearSelection}>Clear</button>
	</header>

	<div class="list">
		{#if chosen.length === 0}
			<p class="empty">Nothing selected. Click a fixture in the rig or on the plan.</p>
		{:else}
			{#each chosen as fixture, index (fixture.id)}
				<div
					class="row"
					class:over={over === index && dragging !== index}
					draggable="true"
					role="listitem"
					ondragstart={() => (dragging = index)}
					ondragover={(e) => {
						e.preventDefault();
						over = index;
					}}
					ondragend={drop}
					ondrop={(e) => {
						e.preventDefault();
						drop();
					}}
				>
					<span class="grip" aria-hidden="true">⠿</span>
					<span class="position mono">{index + 1}</span>
					<span class="dot" style:background={fixtureTint(fixture)}></span>
					<span class="name">{fixture.name}</span>
					<button
						class="icon"
						title="Take it out of the selection"
						aria-label="Remove {fixture.name}"
						onclick={() => remove(fixture.id)}
					>
						✕
					</button>
				</div>
			{/each}
		{/if}
	</div>
</div>

<style>
	.panel {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--line);
		flex: none;
	}
	h2 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	.count {
		color: var(--accent);
		font-family: monospace;
		font-size: var(--font-xs);
	}
	.spacer {
		flex: 1;
	}

	.list {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 6px;
	}

	.row {
		display: grid;
		grid-template-columns: auto 20px 10px 1fr auto;
		align-items: center;
		gap: 7px;
		padding: 3px 6px;
		border-radius: 3px;
		border-top: 2px solid transparent;
		color: var(--text);
		font-size: var(--font-sm);
		cursor: grab;
	}
	.row:hover {
		background: var(--bg-hover);
	}
	.row.over {
		border-top-color: var(--accent);
	}

	.grip {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.position {
		color: var(--text-faint);
		font-size: var(--font-xs);
		text-align: right;
	}
	.mono {
		font-family: monospace;
	}
	.dot {
		width: 9px;
		height: 9px;
		border-radius: 50%;
		border: 1px solid #8a8a8a;
	}
	.name {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		line-height: 1;
		cursor: pointer;
	}
	.icon:hover {
		color: var(--bad);
	}

	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 8px 6px;
	}

	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: #bbb;
		padding: 3px 10px;
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.ghost:hover:not(:disabled) {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.ghost:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}
</style>
