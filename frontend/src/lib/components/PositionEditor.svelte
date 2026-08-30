<script lang="ts">
	/**
	 * Where a fixture is, and which way it faces at rest.
	 *
	 * Positions are set by dragging in the plan and the rig, which is right for a
	 * whole rig at once and useless for the one light that needs to be at exactly
	 * 4.2 metres because that is what the drawing says. This is the other way in.
	 *
	 * Y is called "trim" here rather than "height" because that is the word on the
	 * drawing and in the room: trim height is how high a bar is flown, and an
	 * operator typing one is copying it off a plot.
	 */

	import type { FixturePosition, Vec3 } from '$lib/generated/index.js';
	import { HANGING, joinPosition, splitPosition } from '$lib/stage.js';

	let {
		position,
		onchange
	}: {
		position: FixturePosition | null;
		onchange: (next: FixturePosition | null) => void;
	} = $props();

	const parts = $derived(splitPosition(position));
	const axial = $derived(parts.direction !== null);

	function movePoint(patch: Partial<Vec3>) {
		onchange(joinPosition({ ...parts.point, ...patch }, parts.direction));
	}

	function aim(patch: Partial<Vec3>) {
		if (!parts.direction) return;
		onchange(joinPosition(parts.point, { ...parts.direction, ...patch }));
	}

	/**
	 * A plain point becomes an axial one facing straight down.
	 *
	 * Not at the origin: a light with no aim yet is hanging, and pointing it at the
	 * floor beneath itself is both the truth and the least surprising default.
	 */
	function setAxial(on: boolean) {
		onchange(joinPosition(parts.point, on ? { ...HANGING } : null));
	}

	const step = 0.1;
</script>

<div class="position">
	{#if position === null}
		<button class="btn btn-ghost" onclick={() => onchange(joinPosition({ x: 0, y: 0, z: 0 }, null))}>
			Place
		</button>
	{:else}
		<div class="axes">
			<label>
				<span>x</span>
				<input
					class="input"
					type="number"
					{step}
					value={parts.point.x}
					onchange={(e) => movePoint({ x: Number(e.currentTarget.value) })}
				/>
			</label>
			<label>
				<!-- Trim, not height: it is the word on the plot an operator is copying. -->
				<span>trim</span>
				<input
					class="input"
					type="number"
					{step}
					value={parts.point.y}
					onchange={(e) => movePoint({ y: Number(e.currentTarget.value) })}
				/>
			</label>
			<label>
				<span>z</span>
				<input
					class="input"
					type="number"
					{step}
					value={parts.point.z}
					onchange={(e) => movePoint({ z: Number(e.currentTarget.value) })}
				/>
			</label>
			<button
				class="btn btn-ghost"
				title="Forget where this fixture is"
				onclick={() => onchange(null)}
			>Unplace</button>
		</div>

		<label class="aimed">
			<input type="checkbox" checked={axial} onchange={(e) => setAxial(e.currentTarget.checked)} />
			<span>Faces a direction</span>
		</label>

		{#if parts.direction}
			{@const d = parts.direction}
			<!-- Pan and tilt are angles away from something, and this is the something.
			     A moving head without one has nothing to be aimed relative to. -->
			<div class="axes">
				<label>
					<span>→ x</span>
					<input
						class="input"
						type="number"
						{step}
						value={d.x}
						onchange={(e) => aim({ x: Number(e.currentTarget.value) })}
					/>
				</label>
				<label>
					<span>→ y</span>
					<input
						class="input"
						type="number"
						{step}
						value={d.y}
						onchange={(e) => aim({ y: Number(e.currentTarget.value) })}
					/>
				</label>
				<label>
					<span>→ z</span>
					<input
						class="input"
						type="number"
						{step}
						value={d.z}
						onchange={(e) => aim({ z: Number(e.currentTarget.value) })}
					/>
				</label>
			</div>
		{/if}
	{/if}
</div>

<style>
	.position {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.axes {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}

	.axes label {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.axes span {
		color: var(--text-dim);
		font-size: var(--font-xs);
		min-width: 1.8rem;
	}

	.axes .input {
		/* Wide enough for a direction component: those come off an f32 and arrive as
		   -0.800000011920929, which a narrower box would show as "-0,80000". */
		width: 6.5rem;
		font-variant-numeric: tabular-nums;
	}

	.aimed {
		display: flex;
		align-items: center;
		gap: 6px;
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
</style>
