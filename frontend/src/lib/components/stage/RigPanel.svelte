<script lang="ts">
	/**
	 * The rig in three dimensions, which the spec calls the primary view.
	 *
	 * Its own panel now rather than a tab beside the plan, so that programming can
	 * have both: aim a head here and watch the beam move on the ground plan, or the
	 * other way about.
	 */

	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { collection, show as showStore } from '$lib/stores/show.js';
	import { selection } from '$lib/stores/selection.js';
	import { shownPlanId } from '$lib/stores/stage.js';
	import { RESOLUTIONS, setView, view } from '$lib/stores/view.js';
	import Rig3D from './Rig3D.svelte';
	import MvrButtons from './MvrButtons.svelte';

	const client = getClientContext();
	const data = getDataContext();

	const plans = collection('stage_plans');
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	let follow = $state(true);
	let rig = $state<Rig3D | null>(null);

	/// Polled rather than pushed: a frame cost that re-rendered the toolbar on every
	/// frame would be a readout that costs what it measures.
	let cost = $state<{ cpuMs: number; gpuMs: number | null; drawing: boolean } | null>(null);
	$effect(() => {
		const timer = setInterval(() => (cost = rig?.cost() ?? null), 1000);
		return () => clearInterval(timer);
	});

	/// The view's own settings, in a small sheet off the toolbar. This screen's and
	/// nobody else's: the haze is the show's and lives in Settings; the work light
	/// and the resolution are about the machine in front of somebody.
	let viewing = $state(false);

	// The same plan the plan panel is showing. A show with two rooms in it had the
	// rig drawing the first one's floor under the second one's lights.
	const plan = $derived($plans.find((p) => p.id === $shownPlanId) ?? $plans[0] ?? null);
	const planUrl = $derived(plan ? client.httpUrl(`/assets/${plan.asset}`) : null);
</script>

<div class="rig">
	<nav class="bar">
		<label class="toggle">
			<input type="checkbox" bind:checked={follow} />
			Follow selection
		</label>
		{#if plan}
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
		<MvrButtons />
		<!-- What a frame of this view costs, on the CPU and on the GPU. Read by hand
		     rather than measured by a script: `scripts/demo.sh --measure` deliberately
		     starts no browser, since one would be taking the CPU it is measuring. A
		     view that drew nothing in the last second says so: it draws only when
		     something changed, and a settled rig has no frame to cost. -->
		{#if cost}
			{#if cost.drawing}
				<span
					class="count"
					title="What one frame of this view costs: the work in this page, and how long the GPU took over it"
				>
					{cost.cpuMs.toFixed(1)} ms{#if cost.gpuMs !== null}
						· GPU {cost.gpuMs.toFixed(1)} ms{/if}
				</span>
			{:else}
				<span class="count" title="Nothing changed in the last second, so nothing was drawn">
					idle
				</span>
			{/if}
		{/if}
		<span class="count">{$selection.length} selected</span>
		<button class="ghost" class:open={viewing} onclick={() => (viewing = !viewing)}>View</button>
		<button class="ghost" onclick={() => rig?.goHome()}>Home</button>
	</nav>

	{#if viewing}
		<div class="sheet">
			<label class="field">
				<span>Work light</span>
				<input
					type="range"
					min="0"
					max="1"
					step="0.02"
					value={$view.workLight}
					oninput={(e) => setView({ workLight: e.currentTarget.valueAsNumber })}
				/>
				<span class="reading">{Math.round($view.workLight * 100)}%</span>
			</label>
			<label class="field">
				<span>Resolution</span>
				<select
					value={$view.resolution}
					onchange={(e) => setView({ resolution: Number(e.currentTarget.value) })}
				>
					{#each RESOLUTIONS as choice (choice.value)}
						<option value={choice.value}>{choice.label}</option>
					{/each}
				</select>
			</label>
			<p class="note">
				This screen's only. How bright the room is drawn with nothing on — 0% is a
				blackout, 100% is the house lights up — and how many pixels the view renders.
				Neither reaches a lamp or the show. The haze is the show's, in Settings.
			</p>
		</div>
	{/if}

	<div class="canvas">
		{#if $fixtures.length === 0}
			<p class="empty">Patch a fixture and place it, and it will turn up in here.</p>
		{:else}
			<!-- No `Canvas` wrapper any more: the viewer owns its own renderer, so
			     that two rig panels open at once are two renderers rather than a
			     fight over one. -->
			<Rig3D
				bind:this={rig}
				fixtures={$fixtures}
				types={$types}
				plan={plan?.visible ? plan : null}
				planUrl={plan?.visible ? planUrl : null}
				show={$showStore}
				{follow}
			/>
		{/if}
	</div>
</div>

<style>
	.rig { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.bar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; }
	.spacer { flex: 1; }
	.count { color: #777; font-size: 12px; }
	.toggle { display: flex; align-items: center; gap: 5px; color: #888; font-size: 12px; cursor: pointer; }

	.canvas { flex: 1; min-height: 0; background: #101010; position: relative; }
	.empty { color: #777; font-size: 13px; margin: auto; }

	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover, .ghost.open { border-color: var(--line-input); color: #fff; }

	.sheet { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 18px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; background: #151515; }
	.field { display: flex; align-items: center; gap: 8px; color: #888; font-size: 12px; }
	.field input[type='range'] { width: 140px; }
	.reading { color: #bbb; font-variant-numeric: tabular-nums; min-width: 4ch; }
	.note { flex-basis: 100%; color: #666; font-size: 11px; line-height: 1.5; max-width: 70ch; }
</style>
