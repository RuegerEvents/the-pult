<script lang="ts">
	/**
	 * What the programmer is holding: the values reaching the rig over playback, by
	 * fixture, with what can be done to each.
	 *
	 * Its own panel rather than the bottom of the programmer, because it answers a
	 * different question — the programmer is the selection and the controls that
	 * move it, this is what has been moved — and the two were fighting over one
	 * scroll. Clearing the selection does not clear these, which is the point of
	 * keeping them apart: a value can be parked and reached again from another
	 * selection.
	 *
	 * Amber is the priority rule said in a color: amber is what is reaching the rig.
	 */

	import type { EffectSpec } from '$lib/generated/index.js';
	import { displayLabel, formatValue } from '$lib/patch.js';
	import { collection } from '$lib/stores/show.js';
	import { revealPanel } from '$lib/stores/layout.js';
	import { clear, entries, lockAll, remove, removeEffect, toggleLock } from '$lib/stores/programmer.js';
	import { rgbToHex } from '$lib/programmer.js';
	import Lock from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import CircleStop from '@lucide/svelte/icons/circle-stop';
	import X from '@lucide/svelte/icons/x';

	const fixtures = collection('fixtures');
	const presets = collection('presets');

	const anyUnlocked = $derived($entries.some((e) => !e.locked));

	/** What an effect chip says: the shape and the rate, which is all that fits. */
	function effectSummary(effect: EffectSpec): string {
		const shape = 'Steps' in effect.curve ? `${effect.curve.Steps.length} steps` : effect.curve.Shape.toLowerCase();
		const rate = 'Hz' in effect.rate ? `${effect.rate.Hz} Hz` : 'master';
		return `${shape} · ${rate}`;
	}

	/// Grouped the way an operator reads it: by fixture, in rig order.
	const heldByFixture = $derived(
		$fixtures
			.map((fixture) => ({
				fixture,
				held: $entries.filter((entry) => entry.fixture_id === fixture.id)
			}))
			.filter((group) => group.held.length > 0)
	);
</script>

<div class="held-panel">
	<header>
		<h2>In programmer</h2>
		<span class="count">{$entries.length === 0 ? 'nothing held' : `${$entries.length} held`}</span>
		<span class="spacer"></span>
		<button class="ghost" disabled={!anyUnlocked} onclick={() => clear()}>Clear</button>
		<button
			class="ghost"
			disabled={$entries.length === 0}
			onclick={() => lockAll(anyUnlocked)}
			title="Park these values so they survive Clear and Store"
		>
			{anyUnlocked ? 'Park all' : 'Release all'}
		</button>
	</header>

	<div class="body">
		{#if $entries.length === 0}
			<p class="empty">Nothing held. Move a control in the programmer and it turns up here.</p>
		{:else}
			<div class="rows">
				{#each heldByFixture as group (group.fixture.id)}
					<div class="group">
						<span class="fixture">{group.fixture.name}</span>
						{#each group.held as entry (entry.id)}
							<div class="entry" class:locked={entry.locked}>
								<span class="param">{displayLabel(entry.parameter_kind)}</span>
								{#if entry.effect}
									<!-- The value under an effect is only where it falls back to, so
									     showing it would be showing a number the light is not at.
									     What is worth saying is that something periodic has this
									     parameter, and what it is. -->
									<button
										class="chip-effect"
										title="Open the effect on this parameter"
										onclick={() => revealPanel('effects')}
									>{effectSummary(entry.effect)}</button>
								{:else}
									{#if entry.value.type === 'Color'}
										<!-- A color is seen, not read: the swatch, and the hex beside it
										     for somebody who has to type it somewhere else. -->
										<span class="value color">
											<span class="swatch" style:background={rgbToHex(entry.value.value)}></span>
											<span class="mono">{rgbToHex(entry.value.value)}</span>
										</span>
									{:else}
										<span class="mono value">{formatValue(entry.value)}</span>
									{/if}
									<!-- Where it came from, if it came from a palette. A value
									     moved by hand writes `preset: null` in the same write, so
									     this chip disappears the moment somebody touches the fader —
									     which is the whole of what "breaking the link" looks like. -->
									{#if entry.preset}
										<span class="chip-preset" title="From a preset — a store carries the reference">
											{$presets.find((p) => p.id === entry.preset)?.name ?? 'preset missing'}
										</span>
									{/if}
								{/if}
								<button
									class="icon lock"
									title={entry.locked ? 'Release this value' : 'Park this value'}
									aria-label={entry.locked ? 'Release' : 'Park'}
									onclick={() => toggleLock(entry.id)}
								>
									{#if entry.locked}<Lock size={13} />{:else}<LockOpen size={13} />{/if}
								</button>
								{#if entry.effect}
									<button
										class="icon"
										title="Stop this effect, leaving the value where it is"
										aria-label="Stop the effect"
										onclick={() => removeEffect(entry.effect!.effect_id)}
									><CircleStop size={13} /></button>
								{/if}
								<button
									class="icon drop"
									title="Give this parameter back to playback"
									aria-label="Remove {displayLabel(entry.parameter_kind)}"
									onclick={() => remove(entry.id)}
								>
									<X size={13} />
								</button>
							</div>
						{/each}
					</div>
				{/each}
			</div>
		{/if}
	</div>
</div>

<style>
	.held-panel {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
	header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--line);
		flex: none;
	}
	h2 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	.count {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.spacer {
		flex: 1;
	}
	.body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 4px 12px 12px;
	}
	.rows {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 2px 22px;
		align-items: start;
	}
	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 12px 0;
	}
	.group {
		margin-top: 8px;
		min-width: 0;
	}
	.fixture {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.entry {
		display: grid;
		grid-template-columns: 1fr auto auto auto;
		align-items: center;
		gap: 6px;
		padding: 2px 4px;
		border-radius: 3px;
		color: var(--text);
		font-size: var(--font-sm);
	}
	.entry:hover {
		background: var(--bg-hover);
	}
	/* A reference rather than a number. Cyan, the same color the fixture sheet uses
	   for a value that came from somewhere other than here. */
	.chip-preset {
		font-size: 10px;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--src-tracked);
		color: var(--src-tracked);
		white-space: nowrap;
	}
	.chip-effect {
		font-size: var(--font-xs);
		padding: 2px 8px;
		border-radius: 999px;
		border: 1px solid var(--live);
		background: none;
		color: var(--live);
		font-family: inherit;
		cursor: pointer;
		white-space: nowrap;
	}
	.chip-effect:hover {
		background: var(--live);
		color: var(--bg);
	}
	.entry.locked {
		color: var(--live);
	}
	.entry .value {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.entry.locked .value {
		color: inherit;
	}
	.mono {
		font-family: monospace;
	}
	.color {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.swatch {
		width: 14px;
		height: 14px;
		border-radius: 3px;
		border: 1px solid #ffffff30;
		flex: none;
	}
	.icon {
		display: inline-flex;
		align-items: center;
		background: none;
		border: none;
		color: var(--text-faint);
		line-height: 1;
		padding: 2px 3px;
		cursor: pointer;
	}
	/* A parked value's lock is amber, like the row: it is holding. */
	.entry.locked .lock {
		color: var(--live);
	}
	.icon:hover {
		color: var(--text-bright);
	}
	.icon.drop:hover {
		color: var(--bad);
	}
	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: #bbb;
		padding: 3px 10px;
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.ghost:hover:not(:disabled) {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.ghost:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}
</style>
