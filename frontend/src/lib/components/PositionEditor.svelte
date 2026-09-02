<script lang="ts">
	/**
	 * Where a fixture is, and which way it is turned.
	 *
	 * Positions are set by dragging in the plan and the rig, which is right for a
	 * whole rig at once and useless for the one light that needs to be at exactly
	 * 4.2 metres because that is what the drawing says. This is the other way in.
	 *
	 * Y is called "trim" here rather than "height" because that is the word on the
	 * drawing and in the room: trim height is how high a bar is flown, and an
	 * operator typing one is copying it off a plot.
	 *
	 * A rotation is three angles, and nobody aims a light by typing three angles. So
	 * there is also *aim at*: give it a point in the room and it works the rotation
	 * out. Scale is not here — a fixture is the size it is, and the mirrored case
	 * that makes it signed belongs to a truss rather than to a light.
	 */

	import type { Transform, Vec3 } from '$lib/generated/index.js';
	import { at, facingTransform } from '$lib/scene.js';

	let {
		position,
		onchange
	}: {
		position: Transform | null;
		onchange: (next: Transform | null) => void;
	} = $props();

	/** Where *aim at* is pointing it, until it is applied. */
	let target = $state<Vec3>({ x: 0, y: 0, z: 0 });
	let aiming = $state(false);

	const placed = $derived(position ?? at({ x: 0, y: 0, z: 0 }));

	function move(patch: Partial<Vec3>) {
		onchange({ ...placed, position: { ...placed.position, ...patch } });
	}

	function turn(patch: Partial<Vec3>) {
		onchange({ ...placed, rotation: { ...placed.rotation, ...patch } });
	}

	function aimTarget(patch: Partial<Vec3>) {
		target = { ...target, ...patch };
	}

	/// Turn it to face the point in the room somebody typed, keeping its scale.
	function aimAtTarget() {
		const from = placed.position;
		const direction = { x: target.x - from.x, y: target.y - from.y, z: target.z - from.z };
		onchange({ ...facingTransform(from, direction), scale: placed.scale });
	}

	const step = 0.1;
</script>

<div class="position">
	{#if position === null}
		<button class="btn btn-ghost" onclick={() => onchange(at({ x: 0, y: 0, z: 0 }))}>
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
					value={placed.position.x}
					onchange={(e) => move({ x: Number(e.currentTarget.value) })}
				/>
			</label>
			<label>
				<!-- Trim, not height: it is the word on the plot an operator is copying. -->
				<span>trim</span>
				<input
					class="input"
					type="number"
					{step}
					value={placed.position.y}
					onchange={(e) => move({ y: Number(e.currentTarget.value) })}
				/>
			</label>
			<label>
				<span>z</span>
				<input
					class="input"
					type="number"
					{step}
					value={placed.position.z}
					onchange={(e) => move({ z: Number(e.currentTarget.value) })}
				/>
			</label>
			<button
				class="btn btn-ghost"
				title="Forget where this fixture is"
				onclick={() => onchange(null)}
			>Unplace</button>
		</div>

		<!-- XYZ Euler degrees, which is what the schema holds and what three.js reads.
		     Typed by anybody copying a drawing; everybody else uses aim at. -->
		<div class="axes">
			<label>
				<span>rot x</span>
				<input
					class="input"
					type="number"
					step="1"
					value={placed.rotation.x}
					onchange={(e) => turn({ x: Number(e.currentTarget.value) })}
				/>
			</label>
			<label>
				<span>rot y</span>
				<input
					class="input"
					type="number"
					step="1"
					value={placed.rotation.y}
					onchange={(e) => turn({ y: Number(e.currentTarget.value) })}
				/>
			</label>
			<label>
				<span>rot z</span>
				<input
					class="input"
					type="number"
					step="1"
					value={placed.rotation.z}
					onchange={(e) => turn({ z: Number(e.currentTarget.value) })}
				/>
			</label>
			<button class="btn btn-ghost" onclick={() => (aiming = !aiming)}>
				{aiming ? 'Done' : 'Aim at…'}
			</button>
		</div>

		{#if aiming}
			<!-- A point in the room rather than three angles. Applying turns the fixture
			     to face it and leaves everything else alone. -->
			<div class="axes">
				<label>
					<span>at x</span>
					<input
						class="input"
						type="number"
						{step}
						value={target.x}
						onchange={(e) => aimTarget({ x: Number(e.currentTarget.value) })}
					/>
				</label>
				<label>
					<span>at y</span>
					<input
						class="input"
						type="number"
						{step}
						value={target.y}
						onchange={(e) => aimTarget({ y: Number(e.currentTarget.value) })}
					/>
				</label>
				<label>
					<span>at z</span>
					<input
						class="input"
						type="number"
						{step}
						value={target.z}
						onchange={(e) => aimTarget({ z: Number(e.currentTarget.value) })}
					/>
				</label>
				<button class="btn btn-ghost" onclick={aimAtTarget}>Aim</button>
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

</style>
