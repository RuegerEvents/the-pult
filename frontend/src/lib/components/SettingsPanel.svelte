<script lang="ts">
	/**
	 * The two kinds of setting a console has, side by side because the difference
	 * between them is the thing people get wrong.
	 *
	 * A **show** setting travels with the show: change it here and the console in the
	 * other room is changed too, and it is still true when the file is opened next
	 * year on different hardware.
	 *
	 * A **console** setting belongs to the machine somebody is sitting at. It does
	 * not replicate and it is not in the showfile — it decides what a *new* show
	 * starts with and then stops mattering, which is what keeps two stations from
	 * disagreeing about a show they are both holding.
	 *
	 * So the panel is a few topics, and each topic is a small table whose two columns
	 * are those two kinds: *This show* and *New shows*. Which column a number is in says
	 * what it is, where the panel used to say it in a paragraph under every row. The
	 * reasoning is still one `?` away.
	 */

	import { onMount } from 'svelte';

	import type { Easing, FadeCurves, Show } from '$lib/generated/index.js';
	import { CURVE_LABELS, CURVES } from '$lib/fade.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { readPreferences, writePreferences, type Preferences } from '$lib/preferences.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { openSetup } from '$lib/stores/setup.js';
	import Help from './Help.svelte';
	import Undo2 from '@lucide/svelte/icons/undo-2';
	import Spline from '@lucide/svelte/icons/spline';
	import CloudFog from '@lucide/svelte/icons/cloud-fog';
	import Volume2 from '@lucide/svelte/icons/volume-2';

	const data = getDataContext();

	let show = $state<Show | null>(null);
	let prefs = $state<Preferences | null>(null);
	/// Said out loud only when something went wrong, because a setting that took is
	/// its own confirmation — the number is on screen.
	let trouble = $state<string | null>(null);

	async function setShowDepth(value: number) {
		if (!show) return;
		trouble = null;
		await data.show.history_depth.set(clamped(value));
	}

	async function setConsoleDepth(value: number) {
		trouble = null;
		const stored = await writePreferences({ historyDepth: clamped(value) });
		if (!stored) {
			trouble = 'This console could not write its settings down.';
			return;
		}
		prefs = stored;
	}

	async function setShowHomeFade(seconds: number) {
		if (!show) return;
		trouble = null;
		await data.show.home_fade_ms.set(clampedFade(seconds * 1000));
	}

	async function setConsoleHomeFade(seconds: number) {
		trouble = null;
		const stored = await writePreferences({ homeFadeMs: clampedFade(seconds * 1000) });
		if (!stored) {
			trouble = 'This console could not write its settings down.';
			return;
		}
		prefs = stored;
	}

	async function setShowHaze(field: 'haze_density' | 'haze_turbulence', value: number) {
		if (!show) return;
		trouble = null;
		await data.show[field].set(clampedHaze(value));
	}

	/**
	 * One group's default curve, in the show or on this console.
	 *
	 * The whole map is written rather than the one field, because `fade_curves` is a
	 * single JSON column: a path write per group would be five rows in the history
	 * for one decision.
	 */
	async function setShowCurve(group: keyof FadeCurves, curve: Easing) {
		if (!show) return;
		trouble = null;
		await data.show.fade_curves.set({ ...show.fade_curves, [group]: curve });
	}

	async function setConsoleCurve(group: keyof FadeCurves, curve: Easing) {
		if (!prefs) return;
		trouble = null;
		const stored = await writePreferences({
			fadeCurves: { ...prefs.fadeCurves, [group]: curve }
		});
		if (!stored) {
			trouble = 'This console could not write its settings down.';
			return;
		}
		prefs = stored;
	}

	/// In the order the panel lists them, which is the order an operator thinks about
	/// them in: what a fade *is* first, then where it points, then how it looks.
	const GROUPS: { key: keyof FadeCurves; label: string }[] = [
		{ key: 'intensity', label: 'Intensity' },
		{ key: 'position', label: 'Position' },
		{ key: 'color', label: 'Color' },
		{ key: 'beam', label: 'Beam' },
		{ key: 'other', label: 'Everything else' }
	];

	async function setConsoleHaze(
		field: 'hazeDensity' | 'hazeTurbulence',
		value: number
	) {
		trouble = null;
		const stored = await writePreferences({ [field]: clampedHaze(value) });
		if (!stored) {
			trouble = 'This console could not write its settings down.';
			return;
		}
		prefs = stored;
	}

	/// Brought inside what the shader will do, here as well as on the station, so a
	/// slider cannot send something the rig view has to defend itself against.
	const clampedHaze = (value: number) =>
		Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;

	const clamped = (value: number) =>
		Math.round(Math.min(prefs?.historyDepthMax ?? 10_000, Math.max(prefs?.historyDepthMin ?? 10, value)));

	const clampedFade = (ms: number) =>
		Math.round(Math.min(prefs?.homeFadeMsMax ?? 30_000, Math.max(0, ms || 0)));

	/// Roughly how many times Ctrl-Z can be pressed in a row, which is not the same
	/// number and is the one an operator actually cares about. An undo is a change
	/// too and shares the window with the ones it reverses, so a run of them meets
	/// itself around half way.
	const presses = (depth: number) => Math.floor(depth / 2);

	onMount(() => {
		readPreferences().then((p) => (prefs = p));
		return data.show.subscribe((v) => (show = v as Show | null));
	});
</script>

{#snippet number(id: string, value: number, min: number, max: number, step: number, unit: string, change: (v: number) => void)}
	{#if $canEdit}
		<span class="cell">
			<input
				{id}
				class="input narrow"
				type="number"
				{min}
				{max}
				{step}
				{value}
				onchange={(e) => change(e.currentTarget.valueAsNumber)}
			/>
			<span class="unit">{unit}</span>
		</span>
	{:else}
		<span class="value">{value}</span> <span class="unit">{unit}</span>
	{/if}
{/snippet}

{#snippet curve(value: Easing, change: (c: Easing) => void)}
	{#if $canEdit}
		<select class="select" {value} onchange={(e) => change(e.currentTarget.value as Easing)}>
			{#each CURVES as c (c)}
				<option value={c}>{CURVE_LABELS[c]}</option>
			{/each}
		</select>
	{:else}
		<span class="value">{CURVE_LABELS[value]}</span>
	{/if}
{/snippet}

{#snippet absent()}
	<span class="unit">—</span>
{/snippet}

{#snippet head()}
	<thead>
		<tr>
			<th></th>
			<th>
				This show
				<Help label="What a show setting is">
					Kept in the showfile. Every station working this show sees the same value, and it is
					still true when the file is opened next year on different hardware.
				</Help>
			</th>
			<th>
				New shows
				<Help label="What a console setting is">
					Kept on this machine, not in the show. It decides what a new show starts with and then
					stops mattering, so two stations can never disagree about a show they are both holding.
				</Help>
			</th>
		</tr>
	</thead>
{/snippet}

<div class="settings">
	{#if !show && !prefs}
		<p class="empty">No show is open, and this console could not read its settings.</p>
	{/if}

	<section>
		<h3><Undo2 size={14} />Undo history</h3>
		<table class="rack">
			{@render head()}
			<tbody>
				<tr>
					<th scope="row">
						<label for="show-history">History kept</label>
						<Help label="How history is counted">
							Counted in changes. An undo is a change too and shares the room with the ones it takes
							back, so the number of Ctrl-Z presses in a row is about half of it.
						</Help>
					</th>
					<td>
						{#if show}
							{@render number('show-history', show.history_depth, prefs?.historyDepthMin ?? 10, prefs?.historyDepthMax ?? 10_000, 10, 'changes', setShowDepth)}
							<span class="state">≈ {presses(show.history_depth)} undos in a row</span>
						{:else}{@render absent()}{/if}
					</td>
					<td>
						{#if prefs}
							{@render number('new-history', prefs.historyDepth, prefs.historyDepthMin, prefs.historyDepthMax, 10, 'changes', setConsoleDepth)}
						{:else}{@render absent()}{/if}
					</td>
				</tr>
			</tbody>
		</table>
	</section>

	<section>
		<h3><Spline size={14} />Fades</h3>
		<table class="rack">
			{@render head()}
			<tbody>
				<tr>
					<th scope="row">
						<label for="show-home-fade">Letting go takes</label>
						<Help label="What letting go is">
							How long a parameter takes to reach where it rests when nothing is left driving it: a
							sequence taken off, a selection sent home. Zero snaps. It is show data because two
							stations fading one rig home over different times is a disagreement the audience can
							watch.
						</Help>
					</th>
					<td>
						{#if show}
							{@render number('show-home-fade', show.home_fade_ms / 1000, 0, (prefs?.homeFadeMsMax ?? 30_000) / 1000, 0.5, 's', setShowHomeFade)}
						{:else}{@render absent()}{/if}
					</td>
					<td>
						{#if prefs}
							{@render number('new-home-fade', prefs.homeFadeMs / 1000, 0, prefs.homeFadeMsMax / 1000, 0.5, 's', setConsoleHomeFade)}
						{:else}{@render absent()}{/if}
					</td>
				</tr>
				{#each GROUPS as group, i (group.key)}
					<tr>
						<th scope="row">
							{#if i === 0}
								<span class="group-head"
									>Curve
									<Help label="What a fade curve is">
										The shape a fade has when neither the cue nor the capture says one. A dimmer has
										run linear since dimmers had handles; a head that runs linear into a mark and
										stops dead reads as a fault, which is why position starts eased and nothing else
										does.
									</Help></span
								>
							{/if}
							<span class="sub">{group.label}</span>
						</th>
						<td>
							{#if show}
								{@render curve(show.fade_curves[group.key], (c) => setShowCurve(group.key, c))}
							{:else}{@render absent()}{/if}
						</td>
						<td>
							{#if prefs}
								{@render curve(prefs.fadeCurves[group.key], (c) => setConsoleCurve(group.key, c))}
							{:else}{@render absent()}{/if}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</section>

	<section>
		<h3>
			<CloudFog size={14} />Haze
			<Help label="What the haze is">
				How much haze is in the room, for the rig view: at 0 the air is clear and no beam shows, at 1
				every beam does. Turbulence is how fast it drifts. It is a fact about the room rather than
				the screen, so a designer who lit a piece for a hazy house leaves it in the file. It reaches
				no lamp.
			</Help>
		</h3>
		<table class="rack">
			{@render head()}
			<tbody>
				<tr>
					<th scope="row"><label for="show-haze">Density</label></th>
					<td>
						{#if show}
							{@render number('show-haze', show.haze_density, 0, 1, 0.05, '', (v) => setShowHaze('haze_density', v))}
						{:else}{@render absent()}{/if}
					</td>
					<td>
						{#if prefs}
							{@render number('new-haze', prefs.hazeDensity, 0, 1, 0.05, '', (v) => setConsoleHaze('hazeDensity', v))}
						{:else}{@render absent()}{/if}
					</td>
				</tr>
				<tr>
					<th scope="row"><label for="show-haze-turbulence">Turbulence</label></th>
					<td>
						{#if show}
							{@render number('show-haze-turbulence', show.haze_turbulence, 0, 1, 0.05, '', (v) => setShowHaze('haze_turbulence', v))}
						{:else}{@render absent()}{/if}
					</td>
					<td>
						{#if prefs}
							{@render number('new-haze-turbulence', prefs.hazeTurbulence, 0, 1, 0.05, '', (v) => setConsoleHaze('hazeTurbulence', v))}
						{:else}{@render absent()}{/if}
					</td>
				</tr>
			</tbody>
		</table>
	</section>

	<section>
		<h3><Volume2 size={14} />Audio</h3>
		<div class="inline">
			<button class="btn" onclick={() => openSetup('audio')}>Sound card, channels and zones…</button>
		</div>
	</section>

	{#if trouble}
		<p class="trouble">{trouble}</p>
	{/if}
</div>

<style>
	.settings {
		display: flex;
		flex-direction: column;
		gap: 22px;
		max-width: 760px;
	}

	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	h3 {
		display: flex;
		align-items: center;
		gap: 7px;
		margin: 0;
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}
	h3 :global(svg) {
		color: var(--text-faint);
	}

	.rack {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
		table-layout: fixed;
	}
	.rack thead th {
		text-align: left;
		font-weight: 500;
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-faint);
		padding: 0 10px 6px 0;
	}
	.rack thead th:first-child {
		width: 34%;
	}
	.rack tbody th {
		text-align: left;
		font-weight: 400;
		color: var(--text);
		padding: 6px 10px 6px 0;
		border-top: 1px solid var(--line);
	}
	.rack td {
		padding: 6px 10px 6px 0;
		border-top: 1px solid var(--line);
		vertical-align: middle;
	}
	.rack tbody th label {
		color: inherit;
	}

	.cell {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.cell .input {
		width: 9ch;
	}
	.rack .select {
		width: 100%;
		max-width: 16ch;
	}

	.group-head {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		margin-right: 8px;
	}
	.sub {
		color: var(--text-dim);
	}

	.value {
		color: var(--text-bright);
		font-variant-numeric: tabular-nums;
	}
	.unit,
	.state {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.state {
		display: block;
		margin-top: 2px;
	}

	.empty {
		margin: 0;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.trouble {
		margin: 0;
		color: var(--live);
		font-size: var(--font-sm);
	}
</style>
