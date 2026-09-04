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
	import { GRIDS, RENDER_MODES, RESOLUTIONS, setView, view } from '$lib/stores/view.js';
	import { VIEW_PRESETS } from '$lib/camera.js';
	import { gizmoMode, isLocked, layers, objectsById, selectedObjects } from '$lib/stores/scene.js';
	import { askToDelete } from '$lib/stores/editor.js';
	import Rig3D from './Rig3D.svelte';

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

	/// The view's own settings, in a menu that hangs *over* the picture rather than a
	/// sheet that pushes it down. Which is the whole of why it changed: a panel whose
	/// toolbar reflows the canvas moves the thing somebody is aiming a pointer at, and
	/// the next click lands somewhere else. This screen's and nobody else's — the haze
	/// is the show's and lives in Settings.
	let viewing = $state(false);

	const held = $derived(
		[...$selectedObjects]
			.map((id) => $objectsById.get(id))
			.filter((object): object is NonNullable<typeof object> => !!object)
	);
	/** Of those, the ones an operator may actually change. */
	const editable = $derived(held.filter((object) => !isLocked(object, $layers)));

	/**
	 * The keys, on the canvas rather than on the window.
	 *
	 * A workspace has other panels in it and Delete means something different in each;
	 * bound here, the rig's meaning of it applies while the rig has the focus. And
	 * never while somebody is typing in a field, which is the one that catches people.
	 *
	 * The verbs themselves live in **Rig tools**; these call the same functions, so a
	 * console with that panel shut still has the keys.
	 */
	function keys(event: KeyboardEvent) {
		const typing =
			event.target instanceof HTMLInputElement ||
			event.target instanceof HTMLSelectElement ||
			event.target instanceof HTMLTextAreaElement;
		if (typing || editable.length === 0) return;
		if (event.key === 'Delete' || event.key === 'Backspace') {
			event.preventDefault();
			askToDelete(editable);
		}
	}

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
		<span class="count">
			{$selection.length} selected{#if $selectedObjects.size > 0}, {$selectedObjects.size}
				{$selectedObjects.size === 1 ? 'piece' : 'pieces'}{/if}
		</span>
		<button class="ghost" class:open={viewing} onclick={() => (viewing = !viewing)}>View</button>
		<!-- Where to look from. Four computed places and one that follows what is
		     selected, none of which is stored anywhere: a camera position worked out
		     from the rig's own bounding box needs no schema and frames a five-fixture
		     demo and a festival alike. -->
		<div class="shots" role="group" aria-label="Where the view looks from">
			{#each VIEW_PRESETS as preset (preset.value)}
				<button class="shot" title={preset.blurb} onclick={() => rig?.frame(preset.value)}>
					{preset.label}
				</button>
			{/each}
			<button
				class="shot"
				title="Frame what is selected, from where you are looking now"
				disabled={$selection.length === 0}
				onclick={() => rig?.frameSelection()}
			>
				Focus
			</button>
		</div>
	</nav>

	<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
	<div class="canvas" role="presentation" tabindex="0" onkeydown={keys}>
		{#if viewing}
		<div
			class="sheet"
			role="dialog"
			tabindex="-1"
			aria-label="How this screen draws the rig"
			onpointerdown={(e) => e.stopPropagation()}
			onwheel={(e) => e.stopPropagation()}
		>
			<div class="field modes" role="radiogroup" aria-label="How the rig is drawn">
				{#each RENDER_MODES as mode (mode.value)}
					<button
						class="mode"
						class:on={$view.mode === mode.value}
						role="radio"
						aria-checked={$view.mode === mode.value}
						title={mode.blurb}
						onclick={() => setView({ mode: mode.value })}
					>
						{mode.label}
					</button>
				{/each}
			</div>
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
			<div class="field" role="group" aria-label="Projection">
				<span>Projection</span>
				{#each [['perspective', 'Perspective'], ['ortho', 'Flat']] as const as [value, label] (value)}
					<button
						class="mode"
						class:on={$view.projection === value}
						title={value === 'ortho'
							? 'Straight on: parallel lines stay parallel and a metre is a metre wherever it is. What a plan and a section are.'
							: 'A picture of the room.'}
						onclick={() => setView({ projection: value })}
					>
						{label}
					</button>
				{/each}
			</div>
			<label class="field">
				<span>Grid</span>
				<select value={$view.grid} onchange={(e) => setView({ grid: Number(e.currentTarget.value) })}>
					{#each GRIDS as choice (choice.value)}
						<option value={choice.value}>{choice.label}</option>
					{/each}
				</select>
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
				This screen's only. {RENDER_MODES.find((m) => m.value === $view.mode)?.blurb} The
				work light is how bright the room is drawn with nothing on — 0% is a blackout,
				100% is the house lights up. Flat is the same rig seen straight on, which is
				what a plan and a section are; pressing one of those presets turns it on and
				pressing the front or ¾ turns it off, and this switch overrides either. The grid
				is what a dragged piece snaps to — hold Alt to ignore it. None of it reaches a
				lamp or the show; the haze is the show's, in Settings.
			</p>
		</div>
		{/if}

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
				gizmoMode={$gizmoMode}
			/>
		{/if}
	</div>
</div>

<style>
	.rig { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	/* Wrapping, because the bar now carries the editor's verbs as well as the view's:
	   a narrow tile clipped the presets off the end rather than putting them on a
	   second line. */
	.bar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 10px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex: none; }
	.spacer { flex: 1; }
	.count { color: #777; font-size: 12px; }
	.toggle { display: flex; align-items: center; gap: 5px; color: #888; font-size: 12px; cursor: pointer; }

	.canvas { flex: 1; min-height: 0; background: #101010; position: relative; }
	.empty { color: #777; font-size: 13px; margin: auto; }

	.shots { display: flex; }
	.shot { background: none; border: 1px solid var(--line-strong); color: #bbb; padding: 4px 9px; font: inherit; font-size: 12px; cursor: pointer; margin-left: -1px; }
	.shot:first-child { border-radius: 3px 0 0 3px; margin-left: 0; }
	.shot:last-child { border-radius: 0 3px 3px 0; }
	.shot:hover:not(:disabled) { border-color: var(--line-input); color: #fff; }
	.shot:disabled { color: #555; cursor: default; }
	.ghost:disabled { color: #555; cursor: default; border-color: var(--line); }

	.ghost { background: none; border: 1px solid var(--line-strong); border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover, .ghost.open { border-color: var(--line-input); color: #fff; }

	/* Over the picture, not above it. A sheet that took its own row pushed the canvas
	   down every time it opened, which moves the thing somebody is aiming at. */
	.sheet { position: absolute; z-index: 25; top: 8px; right: 8px; width: min(24rem, calc(100% - 16px)); display: flex; flex-wrap: wrap; align-items: center; gap: 10px 18px; padding: 10px 12px; border: 1px solid var(--line-strong); border-radius: 4px; background: #16181c; box-shadow: 0 10px 30px rgb(0 0 0 / 55%); }
	.field { display: flex; align-items: center; gap: 8px; color: #888; font-size: 12px; }
	.field input[type='range'] { width: 140px; }
	.modes { gap: 0; }
	.mode { background: none; border: 1px solid var(--line-strong); color: #999; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; margin-left: -1px; }
	.mode:first-child { border-radius: 3px 0 0 3px; margin-left: 0; }
	.mode:last-child { border-radius: 0 3px 3px 0; }
	.mode:hover { color: #fff; }
	.mode.on { background: #2a2f3a; border-color: var(--line-input); color: #fff; position: relative; }
	.reading { color: #bbb; font-variant-numeric: tabular-nums; min-width: 4ch; }
	.note { flex-basis: 100%; color: #666; font-size: 11px; line-height: 1.5; max-width: 70ch; }
</style>
