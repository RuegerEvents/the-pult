<script lang="ts">
	/**
	 * The ground plan: where the rig is, and — in program mode — what it is doing.
	 *
	 * Was half of a Stage tab that switched between the plan and the 3D rig. They are
	 * two panels now, because the workspace can show both at once and programming
	 * wants exactly that: aim a head on the plan, watch the beam move in the room.
	 */

	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { calibrationScale, fixturePoint, originForPixel } from '$lib/stage.js';
	import { at } from '$lib/scene.js';
	import { collection } from '$lib/stores/show.js';
	import { editing } from '$lib/stores/editing.js';
	import { shownPlanId } from '$lib/stores/stage.js';
	import { selection } from '$lib/stores/selection.js';
	import StagePlanView from './StagePlanView.svelte';
	import { guessScale, uploadPlan } from './upload.js';

	const data = getDataContext();
	const client = getClientContext();

	const plans = collection('stage_plans');
	// Program and Move stay live: placing and aiming a light is what the panel is
	// for. The lock covers the plan itself — its scale, its origin, its angle, and
	// whether it is there at all.
	const unlocked = editing('plan');
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	let mode = $state<'move' | 'program' | 'scale' | 'origin'>('program');
	let uploading = $state(false);
	/// Held between the two clicks of a measurement, then asked about in metres.
	let measured = $state<{ pixels: number } | null>(null);
	let realLength = $state('');
	let view = $state<StagePlanView | null>(null);

	/**
	 * Which plan is on screen.
	 *
	 * A show has as many plans as it has rooms — a main stage and a foyer, a ground
	 * plan and a truss plot — and the panel showed the first one and offered no way
	 * to reach the rest. Which one *this browser* is looking at is not show data, so
	 * it lives here.
	 */
	const plan = $derived($plans.find((p) => p.id === $shownPlanId) ?? $plans[0] ?? null);
	const planUrl = $derived(plan ? client.httpUrl(`/assets/${plan.asset}`) : null);
	const placedCount = $derived($fixtures.filter((f) => fixturePoint(f) !== null).length);

	// Nothing prunes the selection any more: it is a query over the rig, so a
	// deleted fixture stops matching and leaves on the next evaluation.

	async function choose(event: Event, { asNew = false } = {}) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		if (!file) return;
		uploading = true;
		try {
			const uploaded = await uploadPlan(file, client.httpUrl('/assets'));
			if (plan && !asNew) {
				// Replacing the drawing keeps the calibration: a revised ground plan is
				// almost always the same room at the same scale.
				await data.stage_plans.byId(plan.id).asset.set(uploaded.sha256);
				await data.stage_plans.byId(plan.id).width_px.set(uploaded.width_px);
				await data.stage_plans.byId(plan.id).height_px.set(uploaded.height_px);
				await data.stage_plans.byId(plan.id).name.set(file.name);
			} else {
				await data.stage_plans.create({
					id: crypto.randomUUID(),
					name: file.name,
					asset: uploaded.sha256,
					width_px: uploaded.width_px,
					height_px: uploaded.height_px,
					// Centred on the origin, so the plan arrives around the show rather
					// than off in a corner of it.
					origin: {
						x: (-uploaded.width_px * guessScale(uploaded.width_px)) / 2,
						y: 0,
						z: (-uploaded.height_px * guessScale(uploaded.width_px)) / 2
					},
					metres_per_pixel: guessScale(uploaded.width_px),
					rotation_deg: 0,
					opacity: 0.55,
					visible: true
				});
			}
			view?.fit();
		} catch (e) {
			addToast(e instanceof Error ? e.message : 'that plan would not upload');
		} finally {
			uploading = false;
			input.value = '';
		}
	}

	/// Two points on the drawing, and how far apart they really are, is a scale.
	function measure(a: { px: number; py: number }, b: { px: number; py: number }) {
		measured = { pixels: Math.hypot(b.px - a.px, b.py - a.py) };
		realLength = '';
	}

	async function applyScale() {
		if (!plan || !measured) return;
		const scale = calibrationScale({ px: 0, py: 0 }, { px: measured.pixels, py: 0 }, Number(realLength));
		if (scale === null) {
			addToast('Two points that far apart cannot be that distance.', 'warning');
			return;
		}
		await data.stage_plans.byId(plan.id).metres_per_pixel.set(scale);
		measured = null;
		mode = 'move';
		view?.fit();
	}

	/// Move the plan so the clicked pixel lands on the show's origin.
	async function setOrigin(px: number, py: number) {
		if (!plan) return;
		const { x, z } = originForPixel(plan, px, py);
		await data.stage_plans.byId(plan.id).origin.set({ x, y: plan.origin.y, z });
		mode = 'move';
	}

	const place = (fixtureId: string, x: number, z: number) => {
		const existing = $fixtures.find((f) => f.id === fixtureId);
		// Keep the height it was hung at, and the way it is turned; the plan only ever
		// says where on the floor.
		const was = existing?.position ?? null;
		const y = was?.position.y ?? 0;
		data.fixtures.byId(fixtureId).position.set(
			was ? { ...was, position: { x, y, z } } : at({ x, y, z })
		);
	};

	async function unplaceSelected() {
		await Promise.all($selection.map((id) => data.fixtures.byId(id).position.set(null)));
	}
</script>

<div class="stage">
	<nav class="bar">
		<div class="switch">
			<button class:on={mode === 'program'} onclick={() => (mode = 'program')}>Program</button>
			<button class:on={mode === 'move'} onclick={() => (mode = 'move')}>Move</button>
		</div>

		{#if $plans.length > 1}
			<select
				class="ghost"
				value={plan?.id ?? ''}
				onchange={(e) => shownPlanId.set(e.currentTarget.value)}
			>
				{#each $plans as p (p.id)}
					<option value={p.id}>{p.name}</option>
				{/each}
			</select>
		{/if}

		{#if $unlocked}
			<label class="ghost file">
				{uploading ? 'Uploading…' : plan ? 'Replace plan' : 'Upload plan'}
				<input type="file" accept="image/png,image/jpeg,image/webp,application/pdf" onchange={choose} />
			</label>
			{#if plan}
				<!-- Adds rather than replaces. A second room is a second plan, and the
				     upload button above was the only way in, so uploading one meant
				     losing the one you had. -->
				<label class="ghost file">
					New plan
					<input
						type="file"
						accept="image/png,image/jpeg,image/webp,application/pdf"
						onchange={(e) => choose(e, { asNew: true })}
					/>
				</label>
			{/if}
		{/if}

		{#if plan && $unlocked}
			<button
				class="ghost"
				class:on={mode === 'scale'}
				onclick={() => { mode = mode === 'scale' ? 'move' : 'scale'; measured = null; }}
			>
				Set scale
			</button>
			<button
				class="ghost"
				class:on={mode === 'origin'}
				onclick={() => (mode = mode === 'origin' ? 'move' : 'origin')}
			>
				Set origin
			</button>
			<label class="opacity">
				Fade
				<input
					type="range"
					min="0.05"
					max="1"
					step="0.05"
					value={plan.opacity}
					oninput={(e) => data.stage_plans.byId(plan.id).opacity.set(Number(e.currentTarget.value))}
				/>
			</label>
			<label class="toggle">
				<input
					type="checkbox"
					checked={plan.visible}
					onchange={(e) => data.stage_plans.byId(plan.id).visible.set(e.currentTarget.checked)}
				/>
				Show
			</label>
			<!-- `stage.ts` has rotated plans since positions landed and nothing could
			     set the angle. A drawing squared up to the page is rarely squared up
			     to the room. -->
			<label class="opacity">
				Turn
				<input
					type="range"
					min="-180"
					max="180"
					step="1"
					value={plan.rotation_deg}
					oninput={(e) => data.stage_plans.byId(plan.id).rotation_deg.set(Number(e.currentTarget.value))}
				/>
				<span class="mono deg">{Math.round(plan.rotation_deg)}°</span>
			</label>
			<button class="ghost" onclick={() => data.stage_plans.byId(plan.id).delete()}>Remove plan</button>
		{/if}

		<span class="spacer"></span>
		{#if $selection.length > 0 && mode === 'move'}
			<button class="ghost" onclick={unplaceSelected}>Unplace</button>
		{/if}
		<button class="ghost" onclick={() => view?.fit()}>Fit</button>
		<span class="count">{placedCount} of {$fixtures.length} placed</span>
	</nav>

	{#if mode === 'scale'}
		<p class="hint">
			{#if measured}
				Those two points are
				<input
					class="text-input narrow"
					type="number"
					step="0.1"
					placeholder="metres"
					bind:value={realLength}
					onkeydown={(e) => e.key === 'Enter' && applyScale()}
				/>
				metres apart.
				<button class="primary" onclick={applyScale}>Set</button>
				<button class="ghost" onclick={() => (measured = null)}>Start again</button>
			{:else}
				Click two points on the plan whose real distance apart you know.
			{/if}
		</p>
	{:else if mode === 'origin'}
		<p class="hint">Click the point on the plan that is the show's origin.</p>
	{/if}

	{#if $fixtures.length === 0}
		<p class="empty">Patch a fixture and it will turn up here to be placed.</p>
	{:else}
		<StagePlanView
			bind:this={view}
			{plan}
			{planUrl}
			fixtures={$fixtures}
			types={$types}
			{mode}
			onplace={place}
			onmeasure={measure}
			onorigin={setOrigin}
		/>
	{/if}
</div>

<style>
	.stage { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; }
	.spacer { flex: 1; }
	.count { color: #777; font-size: 12px; }

	.switch { display: flex; border: 1px solid var(--line-strong); border-radius: 3px; overflow: hidden; }
	.switch button { background: none; border: none; color: #888; padding: 4px 11px; font: inherit; font-size: 12px; cursor: pointer; }
	.switch button.on { background: #1e3a5f44; color: var(--accent); }

	.file { position: relative; overflow: hidden; }
	.file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }

	.opacity, .toggle { display: flex; align-items: center; gap: 5px; color: #888; font-size: 12px; }
	.opacity input { width: 80px; }

	.hint { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; color: #fbbf24; font-size: 12px; padding: 8px 16px; border-bottom: 1px solid var(--line); flex: none; }
	.empty { color: #777; font-size: 13px; margin: auto; }

	.text-input { background: #171717; border: 1px solid var(--line-strong); border-radius: 3px; color: var(--text); padding: 3px 6px; font: inherit; font-size: 12px; }
	.text-input.narrow { width: 78px; }
	.primary { background: var(--accent-solid); border: none; border-radius: 3px; color: #fff; padding: 4px 11px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover { border-color: var(--line-input); color: #fff; }
	.ghost.on { border-color: var(--accent); color: var(--accent); }
	.deg {
		color: var(--text-dim);
		font-size: 11px;
		min-width: 2.6rem;
		text-align: right;
	}
</style>
