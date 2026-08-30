<script lang="ts">
	/**
	 * The rig in three dimensions, which the spec calls the primary view.
	 *
	 * Its own panel now rather than a tab beside the plan, so that programming can
	 * have both: aim a head here and watch the beam move on the ground plan, or the
	 * other way about.
	 */

	import { Canvas } from '@threlte/core';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { collection } from '$lib/stores/show.js';
	import { selection } from '$lib/stores/selection.js';
	import { shownPlanId } from '$lib/stores/stage.js';
	import Rig3D from './Rig3D.svelte';

	const client = getClientContext();
	const data = getDataContext();

	const plans = collection('stage_plans');
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	let follow = $state(true);
	let rig = $state<Rig3D | null>(null);

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
		<span class="count">{$selection.length} selected</span>
		<button class="ghost" onclick={() => rig?.goHome()}>Home</button>
	</nav>

	<div class="canvas">
		{#if $fixtures.length === 0}
			<p class="empty">Patch a fixture and place it, and it will turn up in here.</p>
		{:else}
			<Canvas>
				<Rig3D
					bind:this={rig}
					fixtures={$fixtures}
					types={$types}
					plan={plan?.visible ? plan : null}
					planUrl={plan?.visible ? planUrl : null}
					{follow}
				/>
			</Canvas>
		{/if}
	</div>
</div>

<style>
	.rig { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.bar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; }
	.spacer { flex: 1; }
	.count { color: #777; font-size: 12px; }
	.toggle { display: flex; align-items: center; gap: 5px; color: #888; font-size: 12px; cursor: pointer; }

	.canvas { flex: 1; min-height: 0; background: #101010; display: grid; }
	.empty { color: #777; font-size: 13px; margin: auto; }

	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover { border-color: var(--line-input); color: #fff; }
</style>
