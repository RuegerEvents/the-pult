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
	 */

	import { onMount } from 'svelte';

	import type { Show } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { readPreferences, writePreferences, type Preferences } from '$lib/preferences.js';
	import { editing } from '$lib/stores/editing.js';

	const data = getDataContext();
	const unlocked = editing('settings');

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

<div class="settings">
	<section>
		<header>
			<h2>This show</h2>
			<p>Kept in the showfile. Every station working this show sees the same values.</p>
		</header>

		{#if !show}
			<p class="empty">No show is open.</p>
		{:else}
			<div class="row">
				<label for="show-history">History kept</label>
				{#if $unlocked}
					<input
						id="show-history"
						class="input"
						type="number"
						min={prefs?.historyDepthMin ?? 10}
						max={prefs?.historyDepthMax ?? 10_000}
						step="10"
						value={show.history_depth}
						onchange={(e) => setShowDepth(e.currentTarget.valueAsNumber)}
					/>
				{:else}
					<span class="value">{show.history_depth}</span>
				{/if}
				<span class="unit">changes</span>
			</div>
			<p class="note">
				About {presses(show.history_depth)} presses of Ctrl-Z in a row. An undo is a change
				too and shares the room with the ones it takes back, so the two numbers are not the
				same.
			</p>

			<div class="row">
				<label for="show-home-fade">Letting go takes</label>
				{#if $unlocked}
					<input
						id="show-home-fade"
						class="input"
						type="number"
						min="0"
						max={(prefs?.homeFadeMsMax ?? 30_000) / 1000}
						step="0.5"
						value={show.home_fade_ms / 1000}
						onchange={(e) => setShowHomeFade(e.currentTarget.valueAsNumber)}
					/>
				{:else}
					<span class="value">{show.home_fade_ms / 1000}</span>
				{/if}
				<span class="unit">seconds</span>
			</div>
			<p class="note">
				How long a parameter takes to reach where it rests when nothing is left driving it —
				a sequence taken off, a selection sent home. Zero snaps. Show data rather than a
				console setting, because two stations fading one rig home over different times is
				not a preference but a disagreement the audience can watch.
			</p>
		{/if}
	</section>

	<section>
		<header>
			<h2>This console</h2>
			<p>
				Kept on this machine, not in the show. Decides what a new show starts with and then
				stops mattering — so two stations can never disagree about a show they are both
				holding.
			</p>
		</header>

		{#if !prefs}
			<p class="empty">This console could not read its settings.</p>
		{:else}
			<div class="row">
				<label for="new-history">New shows keep</label>
				{#if $unlocked}
					<input
						id="new-history"
						class="input"
						type="number"
						min={prefs.historyDepthMin}
						max={prefs.historyDepthMax}
						step="10"
						value={prefs.historyDepth}
						onchange={(e) => setConsoleDepth(e.currentTarget.valueAsNumber)}
					/>
				{:else}
					<span class="value">{prefs.historyDepth}</span>
				{/if}
				<span class="unit">changes</span>
			</div>
			<div class="row">
				<label for="new-home-fade">New shows let go in</label>
				{#if $unlocked}
					<input
						id="new-home-fade"
						class="input"
						type="number"
						min="0"
						max={prefs.homeFadeMsMax / 1000}
						step="0.5"
						value={prefs.homeFadeMs / 1000}
						onchange={(e) => setConsoleHomeFade(e.currentTarget.valueAsNumber)}
					/>
				{:else}
					<span class="value">{prefs.homeFadeMs / 1000}</span>
				{/if}
				<span class="unit">seconds</span>
			</div>
		{/if}
	</section>

	{#if trouble}
		<p class="trouble">{trouble}</p>
	{/if}
</div>

<style>
	.settings {
		padding: 12px 14px;
		display: flex;
		flex-direction: column;
		gap: 22px;
	}

	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	h2 {
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}

	header p {
		color: var(--text-faint);
		font-size: var(--font-xs);
		max-width: 62ch;
		line-height: 1.5;
	}

	.row {
		display: flex;
		align-items: center;
		gap: 10px;
		min-height: var(--hit);
	}

	label {
		color: var(--text);
		font-size: var(--font-sm);
		min-width: 12ch;
	}

	.input {
		width: 9ch;
	}

	.value {
		color: var(--text-bright);
		font-variant-numeric: tabular-nums;
		font-size: var(--font-sm);
	}

	.unit,
	.note {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}

	.note {
		max-width: 62ch;
		line-height: 1.5;
	}

	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}

	.trouble {
		color: var(--live);
		font-size: var(--font-sm);
	}
</style>
