<script lang="ts">
	/**
	 * The programmer, as a panel.
	 *
	 * Two halves. The top is the selection's parameters and the controls that move
	 * them — what an operator uses to build a look. The bottom is what the programmer
	 * is actually holding, which is a different question: the spec keeps the two
	 * apart deliberately, so clearing the selection does not clear the buffer and a
	 * value can be parked and then reached again from a different selection.
	 *
	 * A row that the programmer holds is tinted amber, the same amber that marks a
	 * live cue elsewhere. That is the whole of the priority rule, said in a colour:
	 * amber is what is reaching the rig.
	 */

	import type { ParameterKind, ParameterValue } from '$lib/generated/index.js';
	import { commonValue, editableParameters } from '$lib/programmer.js';
	import { formatValue, kindLabel } from '$lib/patch.js';
	import { collection } from '$lib/stores/show.js';
	import { selection } from '$lib/stores/selection.js';
	import {
		byKey,
		cancelEdit,
		clear,
		editingCue,
		entries,
		lockAll,
		remove,
		setValue,
		toggleLock,
		updateEdit
	} from '$lib/stores/programmer.js';
	import PanTiltPad from './controls/PanTiltPad.svelte';
	import ValueControl from './controls/ValueControl.svelte';
	import StoreMenu from './StoreMenu.svelte';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const cues = collection('cues');

	let storeOpen = $state(false);

	const chosen = $derived($fixtures.filter((f) => $selection.includes(f.id)));
	const rows = $derived(editableParameters($types, chosen));
	const editing = $derived($cues.find((c) => c.id === $editingCue) ?? null);
	const anyUnlocked = $derived($entries.some((e) => !e.locked));

	/** The value a control should be showing: what is held, else what is on stage. */
	function valueOf(key: string, fallback: ParameterValue): ParameterValue {
		const first = chosen[0];
		const entry = first ? $byKey.get(`${first.id}/${key}`) : undefined;
		if (entry) return entry.value;
		return commonValue(chosen, key).value ?? fallback;
	}

	const isHeld = (key: string) => chosen.some((f) => $byKey.has(`${f.id}/${key}`));

	const put = (kind: ParameterKind, value: ParameterValue) =>
		setValue($selection, kind, value);

	/// Pan and tilt also get a pad, because two numbers is not how anyone thinks
	/// about where a light is pointing.
	const panRow = $derived(rows.find((r) => r.kind === 'Pan') ?? null);
	const tiltRow = $derived(rows.find((r) => r.kind === 'Tilt') ?? null);
	const axisValue = (row: { key: string; defaultValue: ParameterValue } | null) => {
		if (!row) return null;
		const value = valueOf(row.key, row.defaultValue);
		return value.type === 'Float' ? value.value : null;
	};

	const nameOf = (fixtureId: string) =>
		$fixtures.find((f) => f.id === fixtureId)?.name ?? fixtureId.slice(0, 6);

	/// The buffer grouped the way an operator reads it: by fixture, in rig order.
	const heldByFixture = $derived(
		$fixtures
			.map((fixture) => ({
				fixture,
				held: $entries.filter((entry) => entry.fixture_id === fixture.id)
			}))
			.filter((group) => group.held.length > 0)
	);
</script>

<div class="values">
	<header>
		<h2>Programmer</h2>
		<span class="count">
			{$selection.length === 0 ? 'nothing selected' : `${$selection.length} selected`}
		</span>
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
		<button class="primary" disabled={$entries.length === 0} onclick={() => (storeOpen = true)}>
			Store
		</button>
	</header>

	{#if editing}
		<div class="editing">
			<span class="tag">Editing</span>
			<span class="name">{editing.number.toFixed(1)} · {editing.name}</span>
			<span class="spacer"></span>
			<button class="ghost" onclick={cancelEdit}>Cancel</button>
			<button class="primary" onclick={updateEdit}>Update</button>
		</div>
	{/if}

	<div class="body">
		{#if rows.length === 0}
			<p class="empty">
				{$selection.length === 0
					? 'Select a fixture in the rig or on the plan to program it.'
					: 'Nothing in this selection has a parameter that can be set.'}
			</p>
		{:else}
			<div class="rows">
			{#if panRow || tiltRow}
				<div class="row pad">
					<span class="label">Position</span>
					<div class="control">
						<PanTiltPad
							pan={axisValue(panRow)}
							tilt={axisValue(tiltRow)}
							onpan={panRow ? (v) => put('Pan', { type: 'Float', value: v }) : undefined}
							ontilt={tiltRow ? (v) => put('Tilt', { type: 'Float', value: v }) : undefined}
						/>
					</div>
				</div>
			{/if}

			{#each rows as row (row.key)}
				{@const value = valueOf(row.key, row.defaultValue)}
				<div class="row" class:held={isHeld(row.key)}>
					<span class="label">
						{row.label}
						{#if row.mixed}<span class="mixed" title="Not every selected fixture has this">mixed</span>{/if}
					</span>
					<span class="readout mono">
						{#if commonValue(chosen, row.key).mixed}
							—
						{:else}
							{formatValue(commonValue(chosen, row.key).value ?? undefined)}
						{/if}
					</span>
					<div class="control">
						<ValueControl
							{value}
							label={row.label}
							tint={isHeld(row.key) ? 'var(--live)' : 'var(--accent)'}
							oninput={(next) => put(row.kind, next)}
						/>
					</div>
				</div>
			{/each}
			</div>
		{/if}

		<section class="buffer">
			<h3>In programmer</h3>
			{#if $entries.length === 0}
				<p class="empty small">Nothing held. Move a control above and it turns up here.</p>
			{:else}
				<div class="rows">
				{#each heldByFixture as group (group.fixture.id)}
					<div class="group">
						<span class="fixture">{group.fixture.name}</span>
						{#each group.held as entry (entry.id)}
							<div class="entry" class:locked={entry.locked}>
								<span class="param">{kindLabel(entry.parameter_kind)}</span>
								<span class="mono value">{formatValue(entry.value)}</span>
								<button
									class="icon"
									title={entry.locked ? 'Release this value' : 'Park this value'}
									aria-label={entry.locked ? 'Release' : 'Park'}
									onclick={() => toggleLock(entry.id)}
								>
									{entry.locked ? '🔒' : '🔓'}
								</button>
								<button
									class="icon drop"
									title="Give this parameter back to playback"
									aria-label="Remove {kindLabel(entry.parameter_kind)}"
									onclick={() => remove(entry.id)}
								>
									✕
								</button>
							</div>
						{/each}
					</div>
				{/each}
				</div>
			{/if}
		</section>
	</div>
</div>

{#if storeOpen}
	<StoreMenu onclose={() => (storeOpen = false)} />
{/if}

<style>
	.values {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	header,
	.editing {
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

	.editing {
		background: #f59e0b12;
		border-bottom-color: #f59e0b44;
	}
	.tag {
		color: var(--live);
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.editing .name {
		color: #f0d090;
		font-size: var(--font-sm);
	}

	.body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 8px 12px 12px;
	}

	/*
	 * Rows flow into columns as the panel widens.
	 *
	 * A fader is a control, not a progress bar: an operator has to be able to put it
	 * where they mean, and one stretched across two thousand pixels is a fader where
	 * a pixel is a tenth of a percent. So a row is given a workable width and the rest
	 * of the panel is spent on *more* rows — which is the arrangement that makes the
	 * programmer worth putting along the bottom of the screen in the first place.
	 */
	.rows {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
		gap: 2px 22px;
		align-items: start;
	}

	.row {
		display: grid;
		grid-template-columns: 92px 52px minmax(0, 1fr);
		align-items: center;
		gap: var(--pad);
		padding: 4px 6px;
		margin: 0 -6px;
		border-radius: var(--radius);
		min-width: 0;
		/* Tall enough to hit standing up, from the fader inside it rather than from
		   padding: a row of these is what an operator spends the show in. */
		min-height: var(--hit);
	}
	.row.held {
		background: #f59e0b14;
	}
	/* The pad has no readout of its own, so it takes the readout column as well —
	   otherwise it lands in the 52px one and comes out the size of a postage stamp. */
	.row.pad {
		align-items: start;
	}
	.row.pad .control {
		grid-column: 2 / -1;
	}

	.label {
		color: var(--text);
		font-size: var(--font-sm);
		display: flex;
		align-items: baseline;
		gap: 5px;
	}
	.mixed {
		color: var(--live);
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}

	.readout {
		color: var(--text-dim);
		font-size: var(--font-xs);
		text-align: right;
	}
	.mono {
		font-family: monospace;
	}

	.control {
		min-width: 0;
	}

	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 12px 0;
	}
	.empty.small {
		font-size: var(--font-xs);
		padding: 6px 0;
	}

	.buffer {
		margin-top: 14px;
		padding-top: 10px;
		border-top: 1px solid var(--line);
	}
	h3 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
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

	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		line-height: 1;
		padding: 2px 3px;
		cursor: pointer;
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

	.primary {
		background: var(--accent-solid);
		border: none;
		border-radius: var(--radius);
		color: #fff;
		padding: 4px 12px;
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.primary:disabled {
		background: var(--line-strong);
		color: var(--text-faint);
		cursor: not-allowed;
	}
</style>
