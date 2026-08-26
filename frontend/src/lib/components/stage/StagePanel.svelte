<script lang="ts">
	import { onMount } from 'svelte';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import type { Fixture, FixtureType, StagePlan } from '$lib/generated/index.js';
	import { calibrationScale, fixturePoint, originForPixel } from '$lib/stage.js';
	import { pruneSelection, selection } from '$lib/stores/selection.js';
	import { Canvas } from '@threlte/core';
	import StagePlanView from './StagePlanView.svelte';
	import Rig3D from './Rig3D.svelte';
	import { guessScale, uploadPlan } from './upload.js';

	const data = getDataContext();
	const client = getClientContext();

	let plans = $state<StagePlan[]>([]);
	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);

	let mode = $state<'move' | 'scale' | 'origin'>('move');
	/// Plan or rig. The plan is where a show is laid out; the rig is where it is
	/// looked at, and the spec calls that view the primary one.
	let dimension = $state<'plan' | 'rig'>('plan');
	let uploading = $state(false);
	/// Held between the two clicks of a measurement, then asked about in metres.
	let measured = $state<{ pixels: number } | null>(null);
	let realLength = $state('');
	let view = $state<StagePlanView | null>(null);

	const plan = $derived(plans[0] ?? null);
	const planUrl = $derived(plan ? client.httpUrl(`/assets/${plan.asset}`) : null);
	const placedCount = $derived(fixtures.filter((f) => fixturePoint(f) !== null).length);

	async function choose(event: Event) {
		const file = (event.currentTarget as HTMLInputElement).files?.[0];
		if (!file) return;
		uploading = true;
		try {
			const uploaded = await uploadPlan(file, client.httpUrl('/assets'));
			if (plan) {
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
			(event.currentTarget as HTMLInputElement).value = '';
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
		const existing = fixtures.find((f) => f.id === fixtureId);
		// Keep the height it was hung at; the plan only ever says where on the floor.
		const y = existing ? (fixturePoint(existing)?.y ?? 0) : 0;
		data.fixtures.byId(fixtureId).position.set({ Point: { x, y, z } });
	};

	async function unplaceSelected() {
		await Promise.all($selection.map((id) => data.fixtures.byId(id).position.set(null)));
	}

	onMount(() => {
		const stops = [
			data.stage_plans.subscribeDeep((v) => { plans = v; }),
			data.fixtures.subscribeDeep((v) => {
				fixtures = v;
				pruneSelection(v.map((f) => f.id));
			}),
			data.fixture_types.subscribeDeep((v) => { types = v; })
		];
		return () => stops.forEach((stop) => stop());
	});
</script>

<div class="stage">
	<nav class="bar">
		<div class="switch">
			<button class:on={dimension === 'plan'} onclick={() => (dimension = 'plan')}>Plan</button>
			<button class:on={dimension === 'rig'} onclick={() => (dimension = 'rig')}>3D</button>
		</div>

		<label class="ghost file">
			{uploading ? 'Uploading…' : plan ? 'Replace plan' : 'Upload plan'}
			<input type="file" accept="image/png,image/jpeg,image/webp,application/pdf" onchange={choose} />
		</label>

		{#if plan && dimension === 'plan'}
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
			<button class="ghost" onclick={() => data.stage_plans.byId(plan.id).delete()}>Remove plan</button>
		{:else if plan}
			<label class="toggle">
				<input
					type="checkbox"
					checked={plan.visible}
					onchange={(e) => data.stage_plans.byId(plan.id).visible.set(e.currentTarget.checked)}
				/>
				Floor
			</label>
		{/if}

		<span class="spacer"></span>
		{#if $selection.length > 0}
			<button class="ghost" onclick={unplaceSelected}>Unplace</button>
		{/if}
		{#if dimension === 'plan'}
			<button class="ghost" onclick={() => view?.fit()}>Fit</button>
		{/if}
		<span class="count">{placedCount} of {fixtures.length} placed</span>
	</nav>

	{#if dimension === 'rig'}
		<!-- Nothing: the rig view has no modes, and a hint bar would only take height
		     away from the thing worth looking at. -->
	{:else if mode === 'scale'}
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

	{#if fixtures.length === 0}
		<p class="empty">Patch a fixture and it will turn up here to be placed.</p>
	{:else if dimension === 'rig'}
		<div class="rig">
			<Canvas>
				<Rig3D {fixtures} {types} plan={plan?.visible ? plan : null} planUrl={plan?.visible ? planUrl : null} />
			</Canvas>
		</div>
	{:else}
		<StagePlanView
			bind:this={view}
			{plan}
			{planUrl}
			{fixtures}
			{types}
			{mode}
			onplace={place}
			onmeasure={measure}
			onorigin={setOrigin}
		/>
	{/if}
</div>

<style>
	.stage { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 10px 16px; border-bottom: 1px solid #2a2a2a; flex: none; }
	.spacer { flex: 1; }
	.count { color: #777; font-size: 12px; }

	.rig { flex: 1; min-height: 0; background: #101010; }

	.switch { display: flex; border: 1px solid #3a3a3a; border-radius: 3px; overflow: hidden; }
	.switch button { background: none; border: none; color: #888; padding: 4px 11px; font: inherit; font-size: 12px; cursor: pointer; }
	.switch button.on { background: #1e3a5f44; color: #4a9eff; }

	.file { position: relative; overflow: hidden; }
	.file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }

	.opacity, .toggle { display: flex; align-items: center; gap: 5px; color: #888; font-size: 12px; }
	.opacity input { width: 80px; }

	.hint { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; color: #fbbf24; font-size: 12px; padding: 8px 16px; border-bottom: 1px solid #2a2a2a; flex: none; }
	.empty { color: #777; font-size: 13px; margin: auto; }

	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 6px; font: inherit; font-size: 12px; }
	.text-input.narrow { width: 78px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 4px 11px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover { border-color: #555; color: #fff; }
	.ghost.on { border-color: #4a9eff; color: #4a9eff; }
</style>
