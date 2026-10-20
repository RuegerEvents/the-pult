<script lang="ts">
	/**
	 * A step list drawn rather than typed: columns are steps across one cycle, rows are
	 * how bright. Click a cell, or drag across the grid to paint a row of them, and the
	 * column takes that level. A heartbeat is four clicks.
	 *
	 * Levels between the ends of the layer: 0 is its low and 1 its high, which for an
	 * intensity is dark and full.
	 */

	import { asOneGesture } from '$lib/stores/gesture.js';

	let {
		levels,
		soft,
		onchange
	}: {
		levels: number[];
		soft: boolean;
		onchange: (levels: number[], soft: boolean) => void;
	} = $props();

	const ROWS = [1, 0.75, 0.5, 0.25, 0];
	let painting = $state(false);

	function set(column: number, level: number) {
		if (levels[column] === level) return;
		onchange(
			levels.map((l, i) => (i === column ? level : l)),
			soft
		);
	}

	function resize(n: number) {
		const count = Math.min(32, Math.max(2, n));
		const next = Array.from({ length: count }, (_, i) => levels[i] ?? 0);
		onchange(next, soft);
	}
</script>

<div class="steps">
	<div class="bar">
		<span class="label">Steps</span>
		<button class="btn btn-icon" aria-label="Fewer steps" onclick={() => resize(levels.length - 1)}>−</button>
		<span class="count">{levels.length}</span>
		<button class="btn btn-icon" aria-label="More steps" onclick={() => resize(levels.length + 1)}>+</button>
		<div class="group" role="group" aria-label="Between steps">
			<button class="btn" class:on={!soft} onclick={() => onchange(levels, false)}>Jump</button>
			<button class="btn" class:on={soft} onclick={() => onchange(levels, true)}>Crossfade</button>
		</div>
	</div>

	<div
		class="grid"
		style:grid-template-columns="repeat({levels.length}, 1fr)"
		role="grid"
		tabindex="-1"
		aria-label="Step levels"
		use:asOneGesture
		onpointerdown={() => (painting = true)}
		onpointerup={() => (painting = false)}
		onpointerleave={() => (painting = false)}
	>
		{#each ROWS as row (row)}
			{#each levels as level, column (column)}
				<button
					class="cell"
					class:lit={level >= row - 1e-6 && row > 0}
					class:at={Math.abs(level - row) < 1e-6}
					aria-label="Step {column + 1} at {Math.round(row * 100)}%"
					onpointerdown={() => set(column, row)}
					onpointerenter={() => painting && set(column, row)}
				></button>
			{/each}
		{/each}
	</div>
</div>

<style>
	.steps {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.label {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.count {
		min-width: 2ch;
		text-align: center;
		font-variant-numeric: tabular-nums;
	}
	.group {
		display: flex;
		gap: 2px;
		margin-left: auto;
	}
	.group .btn.on {
		border-color: var(--accent);
		color: var(--accent);
	}
	.grid {
		display: grid;
		gap: 2px;
		touch-action: none;
		user-select: none;
	}
	.cell {
		height: 16px;
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 2px;
		background: var(--bg-sunken);
		cursor: pointer;
	}
	.cell.lit {
		background: color-mix(in srgb, var(--accent) 45%, transparent);
	}
	.cell.at {
		background: var(--accent);
	}
</style>
