<script lang="ts">
	/**
	 * The programmer, as a panel.
	 *
	 * The selection's parameters and the controls that move them — what an operator
	 * uses to build a look — grouped the way a cue's fades are: intensity, position,
	 * color, beam, the rest. What the programmer is *holding* is its own panel,
	 * `HeldPanel`, because it answers a different question.
	 *
	 * A control shows its own value — a fader in its track, a switch by where its knob
	 * is, a select by what it says — so there is no readout beside it repeating that.
	 *
	 * A row that the programmer holds is tinted amber, the same amber that marks a
	 * live cue elsewhere. That is the whole of the priority rule, said in a color:
	 * amber is what is reaching the rig.
	 */

	import { addToast } from '$lib/toasts.js';
	import type { FadeGroup, ParameterKind, ParameterValue } from '$lib/generated/index.js';
	import { fadeGroup } from '$lib/fade.js';
	import { commonValueOf, editableParameters, rowTargets, type Target } from '$lib/programmer.js';
	import { modeHas, modeOf, parameterKey } from '$lib/patch.js';
	import { cellSpansText, parseCellSpans } from '$lib/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { elements, hasCellsSelected, query, selection, setCells, targetElements } from '$lib/stores/selection.js';
	import { output, watching } from '$lib/stores/output.js';
	import { byKey, cancelEdit, editingCue, entries, home, setValueOn, update } from '$lib/stores/programmer.js';

	import PanTiltPad from './controls/PanTiltPad.svelte';
	import ValueControl from './controls/ValueControl.svelte';
	import StoreMenu from './StoreMenu.svelte';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const cues = collection('cues');

	let storeOpen = $state(false);

	const chosen = $derived($fixtures.filter((f) => $selection.includes(f.id)));
	const rows = $derived(editableParameters($types, chosen));
	const typeById = $derived(new Map($types.map((t) => [t.id, t])));
	const fixtureTypeOf = $derived(new Map($fixtures.map((f) => [f.id, typeById.get(f.fixture_type_id)])));
	/// Where each row lands: a fixture's own key, or each selected cell's.
	const targetsOf = (kind: ParameterKind): Target[] =>
		rowTargets(kind, $targetElements, (id) => fixtureTypeOf.get(id), reaches);
	const fixtureById = $derived(new Map($fixtures.map((f) => [f.id, f])));
	/// A cut selection whose value lands on a whole-fixture key the type also declares
	/// per cell: a bar in its one-color mode, where the cells are one color.
	const wholeForCells = (kind: ParameterKind) =>
		targetsOf(kind).some(
			(t) =>
				parameterKey(t.kind) === parameterKey(kind) &&
				(fixtureTypeOf.get(t.fixtureId)?.parameters ?? []).some(
					(p) => p.per_cell && parameterKey(p.kind) === parameterKey(kind)
				)
		);
	/// Whether the mode a fixture is patched in carries a key: what decides between a
	/// bar's master and its cells, and between its one-color mode and its pixel mode.
	const reaches = (id: string, key: string) => {
		const fixture = fixtureById.get(id);
		return !fixture || modeHas(modeOf(fixtureTypeOf.get(id), fixture.address), key);
	};
	const targetKey = (t: Target) => `${t.fixtureId}/${parameterKey(t.kind)}`;
	const cellCount = $derived($elements.filter((e) => e.cell !== null).length);
	let cellText = $state('');
	$effect(() => {
		cellText = cellSpansText($query.cells ?? []);
	});
	const cellError = $derived(parseCellSpans(cellText) === null);
	const editing = $derived($cues.find((c) => c.id === $editingCue) ?? null);

	/** The groups rows are shown in, in the order a look is usually built. */
	const GROUPS: { group: FadeGroup; label: string }[] = [
		{ group: 'Intensity', label: 'Intensity' },
		{ group: 'Position', label: 'Position' },
		{ group: 'Color', label: 'Color' },
		{ group: 'Beam', label: 'Beam' },
		{ group: 'Other', label: 'Other' }
	];
	const grouped = $derived(
		GROUPS.map((g) => ({ ...g, rows: rows.filter((row) => fadeGroup(row.key) === g.group) })).filter(
			(g) => g.rows.length > 0 || (g.group === 'Position' && (panRow || tiltRow))
		)
	);

	/// What this panel is showing, so it is evaluated every frame. The selection's
	/// parameters and nothing else: a rig of thousands with a dozen selected costs a
	/// dozen.
	$effect(() => {
		const keys = rows.flatMap((row) => targetsOf(row.kind).map(targetKey));
		const registered = watching(keys);
		return () => registered.stop();
	});

	/** The value a control should be showing: what is held, else what is on stage. */
	function valueOf(kind: ParameterKind, fallback: ParameterValue): ParameterValue {
		const targets = targetsOf(kind);
		const first = targets[0];
		const entry = first ? $byKey.get(targetKey(first)) : undefined;
		if (entry) return entry.value;
		return commonValueOf(targets, $output).value ?? fallback;
	}

	const isHeld = (kind: ParameterKind) => targetsOf(kind).some((t) => $byKey.has(targetKey(t)));

	const put = (kind: ParameterKind, value: ParameterValue) => setValueOn(targetsOf(kind), value);

	/// Pan and tilt also get a pad, because two numbers is not how anyone thinks
	/// about where a light is pointing.
	const panRow = $derived(rows.find((r) => r.kind === 'Pan') ?? null);
	const tiltRow = $derived(rows.find((r) => r.kind === 'Tilt') ?? null);
	const axisValue = (row: { kind: ParameterKind; defaultValue: ParameterValue } | null) => {
		if (!row) return null;
		const value = valueOf(row.kind, row.defaultValue);
		return value.type === 'Float' ? value.value : null;
	};

</script>

<div class="values">
	<header>
		<h2>Programmer</h2>
		<span class="count">
			{$selection.length === 0
				? 'nothing selected'
				: cellCount > 0
					? `${$selection.length} selected · ${cellCount} cells`
					: `${$selection.length} selected`}
		</span>
		{#if $hasCellsSelected || ($query.cells ?? []).length > 0}
			<!-- MA's `.`: which cells of every multi-cell fixture selected. Empty is all. -->
			<label class="cells" title="Which cells: 1-20, 5, 20-1. Empty is every cell.">
				cells
				<input
					class="input mono"
					class:bad={cellError}
					placeholder="all"
					bind:value={cellText}
					onchange={() => {
						const spans = parseCellSpans(cellText);
						if (spans) setCells(spans);
					}}
				/>
			</label>
		{/if}
		<span class="spacer"></span>
		<button
			class="ghost"
			disabled={$selection.length === 0}
			onclick={() => home($selection)}
			title="Every parameter of the selection back to where it rests"
		>
			Home
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
			<button
				class="primary"
				onclick={() =>
					update()
						.then((u) => u.edited && addToast(`Updated cue ${u.edited.number.toFixed(1)} · ${u.edited.name}`, 'success'))
						.catch((e) => addToast(e instanceof Error ? e.message : String(e)))}>Update</button
			>
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
			{#each grouped as g (g.group)}
				<h3 class="group-head">{g.label}</h3>
				<div class="rows">
					{#if g.group === 'Position' && (panRow || tiltRow)}
						<div class="row pad">
							<span class="label">Pan / tilt</span>
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
					{#each g.rows as row (row.key)}
						{@const value = valueOf(row.kind, row.defaultValue)}
						{@const unreachable = row.unreachable === row.count}
						{@const shown = commonValueOf(targetsOf(row.kind), $output)}
						<div class="row" class:held={isHeld(row.kind)} class:unreachable>
							<span class="label" title={row.label}>
								<span class="name">{row.label}</span>
								{#if row.mixed}<span class="mixed" title="Not every selected fixture has this">mixed</span>{/if}
								{#if shown.mixed}<span class="mixed" title="The selected fixtures are at different values; the control shows the first">differs</span>{/if}
								{#if ($query.cells ?? []).length > 0 && wholeForCells(row.kind)}
									<!-- Cells are cut, but the mode has one of these for the whole
									     fixture (a bar's one-color mode): the value goes there. -->
									<span class="mixed" title="This fixture's mode has one of these for the whole fixture, not one per cell"
										>whole</span
									>
								{/if}
								{#if row.unreachable > 0}
									<!-- The light can do it; the mode it is patched in has nowhere to
									     send it. Saying so beats a fader that silently does nothing. -->
									<span
										class="mixed"
										title={unreachable
											? 'The mode this fixture is patched in does not carry this parameter'
											: `${row.unreachable} of ${row.count} are in a mode that does not carry this`}
										>not in mode</span
									>
								{/if}
							</span>
							<div class="control">
								<ValueControl
									{value}
									label={row.label}
									choices={row.choices}
									range={row.physical}
									flash
									tint={isHeld(row.kind) ? 'var(--live)' : 'var(--accent)'}
									oninput={(next) => put(row.kind, next)}
								/>
							</div>
						</div>
					{/each}
				</div>
			{/each}
		{/if}
	</div>
</div>

{#if storeOpen}
	<StoreMenu onclose={() => (storeOpen = false)} />
{/if}

<style>
	/* Dimmed, not hidden, and still movable: the value is stored and takes effect the
	   moment the fixture is patched in a mode that carries it. */
	.row.unreachable .label,
	.row.unreachable .control { opacity: 0.45; }

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
	.cells {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.cells .input {
		width: 7em;
	}
	.cells .bad {
		border-color: var(--bad);
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
		grid-template-columns: 104px minmax(0, 1fr);
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
	.row.pad {
		align-items: start;
	}
	/* A heading per group, quiet: it says where Color starts, not what Color is. */
	.group-head {
		margin: 12px 0 4px;
		font-size: 10px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint);
	}
	.group-head:first-child {
		margin-top: 2px;
	}

	.label {
		color: var(--text);
		font-size: var(--font-sm);
		display: flex;
		flex-direction: column;
		gap: 1px;
		min-width: 0;
	}
	.label .name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.mixed {
		color: var(--live);
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
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
