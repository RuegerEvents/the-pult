<script lang="ts">
	/**
	 * The rig as a table: fixture down, parameter across, and a colour per cell that
	 * says **where the value came from**.
	 *
	 * Every other desk has this and puts a flag on a value to colour it. This console
	 * does not have to: the model already keeps what is *driving* each parameter, so
	 * the colour falls out of `driving.ts`'s four layers plus whether a take is rolling
	 * over the key — `lib/sheet.ts` is that rule, pure and tested, and the panel calls
	 * it rather than restating it.
	 *
	 * Three things it shows, and only ever one at a time:
	 *
	 * - **Live** — what the rig is doing now, evaluated by the same wasm the station
	 *   runs, at animation-frame rate over exactly the cells on screen.
	 * - **A cue** — the stack *up to* that cue, which is what taking it would assert.
	 *   Hard against tracked, with the number of the cue a tracked value came from.
	 *   This reaches no output: looking at a cue is not taking it.
	 * - **A preset** — its values, and nothing where it says nothing.
	 *
	 * Rows are the selection, in the selection's own order, because that is the thing
	 * an operator is working on. With nothing selected it is the whole patch, and the
	 * *All* switch is there for the moment somebody wants the rig with their selection
	 * marked in it rather than filtered to it.
	 */

	import type { Cue, Fixture, ParameterCapture, ParameterKind } from '$lib/generated/index.js';
	import { cueIdsThrough, trackedThrough } from '$lib/cues.js';
	import { drivenBy, drivingKey } from '$lib/driving.js';
	import { recordingKeys } from '$lib/evaluator.js';
	import { CURVE_LABELS, CURVES, fadeGroup } from '$lib/fade.js';
	import { formatValue, kindLabel, parameterKey } from '$lib/patch.js';
	import { asFloat, withFloat } from '$lib/programmer.js';
	import { SOURCE_LABELS, source, trackedSource, type Source } from '$lib/sheet.js';
	import { collection } from '$lib/stores/show.js';
	import { cueInView, presetInView, viewCue, viewPreset } from '$lib/stores/cues.js';
	import { setPresetValue } from '$lib/stores/programmer.js';
	import { output, watching } from '$lib/stores/output.js';
	import { byKey, setValue } from '$lib/stores/programmer.js';
	import { select, selected, selection } from '$lib/stores/selection.js';
	import { getDataContext } from '$lib/ws/context.js';

	const data = getDataContext();
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const cues = collection('cues');
	const sequences = collection('sequences');
	const timelines = collection('timelines');
	const presets = collection('presets');

	/** Show the whole patch with the selection marked, rather than the selection alone. */
	let all = $state(false);
	/** The cell an operator has picked, which is what the inspector strip edits. */
	let picked = $state<{ fixtureId: string; key: string } | null>(null);

	const typeOf = (fixture: Fixture) => $types.find((t) => t.id === fixture.fixture_type_id);

	/**
	 * The rows.
	 *
	 * The selection in the order the selection is in — which is the order a chase runs
	 * in and the order an operator dragged the panel into — and the patch order when
	 * there is no selection to have an order.
	 */
	const rows = $derived.by(() => {
		if (all || $selection.length === 0) return $fixtures;
		const byId = new Map($fixtures.map((f) => [f.id, f]));
		return $selection.map((id) => byId.get(id)).filter((f): f is Fixture => !!f);
	});

	/**
	 * The columns: every output parameter any row has, grouped the way a fade is
	 * grouped, so intensity comes first and the beam parameters sit together.
	 *
	 * `fadeGroup` rather than a second ordering of its own — the console already has
	 * one answer to "what sort of parameter is this" and a table with a different one
	 * would put colour in two places on two screens.
	 */
	const GROUPS = ['Intensity', 'Position', 'Color', 'Beam', 'Other'] as const;

	const columns = $derived.by(() => {
		const found = new Map<string, { key: string; kind: ParameterKind; label: string }>();
		for (const fixture of rows) {
			for (const parameter of typeOf(fixture)?.parameters ?? []) {
				if (parameter.direction !== 'Output') continue;
				const key = parameterKey(parameter.kind);
				if (!found.has(key)) {
					found.set(key, { key, kind: parameter.kind, label: kindLabel(parameter.kind) });
				}
			}
		}
		return [...found.values()].sort(
			(a, b) => GROUPS.indexOf(fadeGroup(a.key)) - GROUPS.indexOf(fadeGroup(b.key))
		);
	});

	// ── What is being looked at ─────────────────────────────────────────────────

	const shownCue = $derived($cues.find((c) => c.id === $cueInView) ?? null);
	const shownPreset = $derived($presets.find((p) => p.id === $presetInView) ?? null);
	/** A preset's values, keyed the way a cell asks for them. */
	const presetCells = $derived(
		new Map(
			(shownPreset?.values ?? []).map((v) => [
				drivingKey(v.fixture_id, parameterKey(v.parameter_kind)),
				v
			])
		)
	);
	const sequenceOf = (cue: Cue) => $sequences.find((s) => s.cue_ids.includes(cue.id)) ?? null;

	/**
	 * The cue's own stack: the latest capture of every key over the cues up to it.
	 *
	 * The browser's `trackedThrough`, which mirrors `cue::tracked_through` and is held
	 * to it by `testdata/tracking.json` — because a cue clicked in a list has to colour
	 * this table in the same frame, and a round trip inside that is a table that
	 * flickers.
	 */
	const tracked = $derived.by(() => {
		const cue = shownCue;
		if (!cue) return new Map<string, { cue: Cue; capture: ParameterCapture }>();
		const sequence = sequenceOf(cue);
		const through = sequence ? cueIdsThrough(sequence, cue.id) : [cue.id];
		const byId = new Map($cues.map((c) => [c.id, c]));
		const out = new Map<string, { cue: Cue; capture: ParameterCapture }>();
		for (const entry of trackedThrough(through, (id) => byId.get(id))) {
			out.set(
				drivingKey(entry.capture.fixture_id, parameterKey(entry.capture.parameter_kind)),
				entry
			);
		}
		return out;
	});

	/**
	 * What is driving each parameter of each row, as the evaluator is handed it.
	 *
	 * Assembled here rather than in `stores/output.ts` because that one keeps the whole
	 * rig and this is the rows on screen — and because what the sheet wants out of it
	 * is not the value, which the evaluator answers, but which layer won.
	 */
	const driving = $derived.by(() => {
		const out = new Map<string, ReturnType<typeof drivenBy>>();
		for (const fixture of rows) out.set(fixture.id, drivenBy(fixture, typeOf(fixture), $byKey));
		return out;
	});

	/// This panel's cells, evaluated every frame while it is up — the selection's
	/// parameters and nothing else, so a rig of thousands with a dozen on screen
	/// costs a dozen.
	$effect(() => {
		const keys = rows.flatMap((fixture) =>
			columns.map((column) => drivingKey(fixture.id, column.key))
		);
		const registered = watching(keys);
		return () => registered.stop();
	});

	/**
	 * Which cells a recording is asserting.
	 *
	 * Asked of the evaluator, because a take is bytes that never leave it — and asked
	 * only while something is actually rolling, so a console with no timeline running
	 * never crosses the boundary for it.
	 */
	const recorded = $derived.by(() => {
		if ($output.at === null) return new Set<string>();
		if (!$timelines.some((t) => t.running)) return new Set<string>();
		return new Set(recordingKeys($output.at));
	});

	// ── One cell ────────────────────────────────────────────────────────────────

	type Cell = { text: string; source: Source; from: string | null };

	function cell(fixture: Fixture, key: string): Cell {
		const at = drivingKey(fixture.id, key);
		if (shownPreset) {
			// A preset says nothing about most of the rig, and an empty cell is the
			// honest way to draw that: it is not a zero, and it is not a home value.
			const value = presetCells.get(at);
			return value
				? { text: formatValue(value.value), source: 'cue', from: null }
				: { text: '', source: 'none', from: null };
		}
		if (shownCue) {
			const entry = tracked.get(at);
			if (!entry) return { text: '', source: 'none', from: null };
			// A capture that names a preset shows the palette rather than the number,
			// because that is what it *is* — and one whose preset is gone says so, since
			// the number it is running on is the literal it was stored with.
			const named = entry.capture.preset
				? ($presets.find((p) => p.id === entry.capture.preset)?.name ?? 'preset missing')
				: null;
			return {
				text: named ?? (entry.capture.effect ? '∿' : formatValue(entry.capture.value)),
				source: trackedSource(entry.cue, shownCue.id),
				from: entry.cue.id === shownCue.id ? null : entry.cue.number.toFixed(1)
			};
		}
		const row = driving.get(fixture.id)?.get(key);
		const kind = source(row, recorded.has(at), null);
		const value = $output.value(fixture.id, key);
		return {
			text: kind === 'none' ? '' : $output.at === null ? '·' : formatValue(value ?? undefined),
			source: kind,
			from: null
		};
	}

	// ── Editing ─────────────────────────────────────────────────────────────────

	const pickedCapture = $derived.by(() => {
		if (!picked || !shownCue) return null;
		const at = drivingKey(picked.fixtureId, picked.key);
		const entry = tracked.get(at);
		return entry && entry.cue.id === shownCue.id ? entry.capture : null;
	});

	/**
	 * Write one capture back into its cue.
	 *
	 * The whole array, because `captures` is one column — the same reason a dragged
	 * sheet block rewrites `Sheet::blocks` whole. One write, so one Ctrl-Z.
	 */
	async function editCapture(patch: Partial<ParameterCapture>) {
		const cue = shownCue;
		const capture = pickedCapture;
		if (!cue || !capture) return;
		const next = cue.captures.map((c) =>
			c.fixture_id === capture.fixture_id &&
			parameterKey(c.parameter_kind) === parameterKey(capture.parameter_kind)
				? { ...c, ...patch }
				: c
		);
		await data.cues.byId(cue.id).captures.set(next);
	}

	/** Typing a number into a live cell is the programmer, which is what a sheet is for. */
	async function typeInto(fixture: Fixture, column: { key: string; kind: ParameterKind }, text: string) {
		const percent = Number(text);
		if (!Number.isFinite(percent)) return;
		const current =
			$output.value(fixture.id, column.key) ??
			typeOf(fixture)?.parameters.find((p) => parameterKey(p.kind) === column.key)?.default_value;
		if (!current) return;
		setValue([fixture.id], column.kind, withFloat(current, percent / 100));
	}

	const percentOf = (value: ParameterCapture['value']) => {
		const n = asFloat(value);
		return n === null ? '' : String(Math.round(n * 100));
	};
</script>

<div class="sheet">
	<header>
		{#if shownCue}
			<span class="mode">
				Cue <strong>{shownCue.number.toFixed(1)}</strong> · {shownCue.name}
			</span>
			<span class="note">what taking it would assert — nothing here reaches the rig</span>
			<button class="chip" onclick={() => viewCue(null)}>Show the rig</button>
		{:else if shownPreset}
			<span class="mode">Preset <strong>{shownPreset.name}</strong></span>
			<span class="note">what it says, and nowhere it says nothing</span>
			<button class="chip" onclick={() => viewPreset(null)}>Show the rig</button>
		{:else}
			<span class="mode">Live</span>
			<span class="note">what the rig is doing, and what is driving it</span>
		{/if}
		<span class="spacer"></span>
		<label class="check">
			<input type="checkbox" bind:checked={all} />
			All fixtures
		</label>
	</header>

	{#if rows.length === 0}
		<p class="empty">Nothing patched yet.</p>
	{:else if columns.length === 0}
		<p class="empty">These fixtures have no parameters anybody can set.</p>
	{:else}
		<div class="scroll">
			<table>
				<thead>
					<tr>
						<th class="name">Fixture</th>
						{#each columns as column (column.key)}
							<th class="group-{fadeGroup(column.key).toLowerCase()}">{column.label}</th>
						{/each}
					</tr>
				</thead>
				<tbody>
					{#each rows as fixture (fixture.id)}
						<tr class:picked={$selected.has(fixture.id)}>
							<th class="name">
								<button class="row-name" onclick={() => select(fixture.id)}>{fixture.name}</button>
							</th>
							{#each columns as column (column.key)}
								{@const c = cell(fixture, column.key)}
								<td
									class="src-{c.source}"
									class:on={picked?.fixtureId === fixture.id && picked?.key === column.key}
								>
									<button
										class="cell"
										title="{c.text || 'nothing'} · {SOURCE_LABELS[c.source]}{c.from
											? ` from cue ${c.from}`
											: ''}"
										onclick={() => (picked = { fixtureId: fixture.id, key: column.key })}
									>
										{c.text}
										{#if c.from}<span class="from">{c.from}</span>{/if}
									</button>
								</td>
							{/each}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}

	<!-- The inspector. One capture at a time, and the only place `fade_out_ms` and a
	     capture's own delay have ever been reachable after the moment they were
	     stored. -->
	{#if picked}
		{@const fixture = rows.find((f) => f.id === picked?.fixtureId)}
		<div class="inspector">
			<span class="what">
				{fixture?.name ?? '—'} · {picked.key}
			</span>
			{#if pickedCapture}
				{@const capture = pickedCapture}
				<label>
					value
					<input
						class="num"
						type="number"
						min="0"
						max="100"
						value={percentOf(capture.value)}
						onchange={(e) => {
							const n = Number(e.currentTarget.value);
							if (Number.isFinite(n)) editCapture({ value: withFloat(capture.value, n / 100) });
						}}
					/>
					<span class="unit">%</span>
				</label>
				<label>
					in
					<input
						class="num"
						type="number"
						min="0"
						step="100"
						placeholder="cue"
						value={capture.fade_in_ms || ''}
						onchange={(e) => editCapture({ fade_in_ms: Number(e.currentTarget.value) || 0 })}
					/>
				</label>
				<label>
					out
					<input
						class="num"
						type="number"
						min="0"
						step="100"
						placeholder="in"
						value={capture.fade_out_ms || ''}
						onchange={(e) => editCapture({ fade_out_ms: Number(e.currentTarget.value) || 0 })}
					/>
				</label>
				<label>
					delay
					<input
						class="num"
						type="number"
						min="0"
						step="100"
						placeholder="0"
						value={capture.delay_in_ms || ''}
						onchange={(e) => editCapture({ delay_in_ms: Number(e.currentTarget.value) || 0 })}
					/>
				</label>
				<label>
					curve
					<select
						value={capture.easing ?? ''}
						onchange={(e) =>
							editCapture({ easing: (e.currentTarget.value || null) as ParameterCapture['easing'] })}
					>
						<!-- `null` is not linear: it is this capture saying nothing, so the cue
						     answers and the show answers for the cue. -->
						<option value="">inherited</option>
						{#each CURVES as curve (curve)}
							<option value={curve}>{CURVE_LABELS[curve]}</option>
						{/each}
					</select>
				</label>
			{:else if shownPreset}
				{@const value = presetCells.get(drivingKey(picked.fixtureId, picked.key))}
				{@const column = columns.find((c) => c.key === picked?.key)}
				{#if value}
					<label>
						value
						<input
							class="num"
							type="number"
							min="0"
							max="100"
							value={percentOf(value.value)}
							onchange={(e) => {
								const n = Number(e.currentTarget.value);
								if (Number.isFinite(n) && shownPreset) {
									setPresetValue(
										shownPreset,
										picked!.fixtureId,
										value.parameter_kind,
										withFloat(value.value, n / 100)
									);
								}
							}}
						/>
						<span class="unit">%</span>
					</label>
					<button
						class="chip"
						onclick={() =>
							shownPreset &&
							setPresetValue(shownPreset, picked!.fixtureId, value.parameter_kind, null)}
						>Remove</button
					>
					<span class="note">every cue referencing this preset follows, at once</span>
				{:else if column}
					<span class="note">
						This preset says nothing about it. Set it in the programmer and use
						<em>Update from programmer</em>.
					</span>
				{/if}
			{:else if shownCue}
				<span class="note">
					This cue does not capture it — what is shown is tracked from an earlier one.
				</span>
			{:else if fixture}
				{@const column = columns.find((c) => c.key === picked?.key)}
				<label>
					set to
					<input
						class="num"
						type="number"
						min="0"
						max="100"
						placeholder="%"
						onchange={(e) => {
							if (column) typeInto(fixture, column, e.currentTarget.value);
							e.currentTarget.value = '';
						}}
					/>
					<span class="unit">%</span>
				</label>
				<span class="note">into the programmer, for every fixture selected</span>
			{/if}
			<span class="spacer"></span>
			<button class="chip" onclick={() => (picked = null)}>Close</button>
		</div>
	{/if}

	<!-- The legend, which is what makes the colours readable to somebody who has not
	     stood behind this desk before. -->
	<footer>
		{#each ['programmer', 'track', 'effect', 'cue', 'tracked', 'home'] as const as kind (kind)}
			<span class="key src-{kind}"><i></i>{SOURCE_LABELS[kind]}</span>
		{/each}
	</footer>
</div>

<style>
	.sheet {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	header,
	.inspector,
	footer {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 6px 10px;
		font-size: var(--font-sm);
		flex-shrink: 0;
	}
	header {
		border-bottom: 1px solid var(--line);
	}
	.mode {
		color: var(--text);
	}
	.mode strong {
		color: var(--live);
		font-family: monospace;
	}
	.note {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.spacer {
		flex: 1;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 5px;
		color: var(--text-dim);
		cursor: pointer;
	}

	.empty {
		padding: 20px 10px;
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
	}

	.scroll {
		flex: 1;
		min-height: 0;
		overflow: auto;
	}

	table {
		border-collapse: collapse;
		font-size: var(--font-sm);
		width: 100%;
	}
	thead th {
		position: sticky;
		top: 0;
		z-index: 1;
		background: var(--bg-panel);
		text-align: left;
		font-weight: 500;
		color: var(--text-dim);
		padding: 5px 8px;
		border-bottom: 1px solid var(--line-strong);
		white-space: nowrap;
	}
	/* The fade groups, tinted only in the heading: a column of colour would fight
	   the colours that carry the meaning. */
	.group-intensity { color: #d4d4d8; }
	.group-position { color: #93c5fd; }
	.group-color { color: #f0abfc; }
	.group-beam { color: #fcd34d; }

	th.name {
		left: 0;
		z-index: 2;
		background: var(--bg-panel);
		max-width: 14rem;
	}
	tbody th.name {
		position: sticky;
		border-bottom: 1px solid #ffffff08;
		font-weight: 400;
	}
	.row-name {
		background: none;
		border: none;
		color: var(--text-dim);
		font: inherit;
		cursor: pointer;
		padding: 0;
		max-width: 13rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	tr.picked .row-name {
		color: var(--text-bright);
	}

	td {
		border-bottom: 1px solid #ffffff08;
		border-left: 1px solid #ffffff08;
		padding: 0;
		text-align: right;
	}
	.cell {
		display: block;
		width: 100%;
		background: none;
		border: none;
		font: inherit;
		font-family: monospace;
		font-size: var(--font-xs);
		color: inherit;
		padding: 4px 8px;
		text-align: right;
		cursor: pointer;
		min-height: 1.6em;
	}
	td.on {
		outline: 1px solid var(--accent);
		outline-offset: -1px;
	}
	.from {
		margin-left: 5px;
		opacity: 0.6;
		font-size: 9px;
	}

	/* The whole claim of this panel, in six colours. `lib/sheet.ts` decides which. */
	.src-programmer { color: var(--src-programmer); }
	.src-track { color: var(--src-track); }
	.src-effect { color: var(--src-effect); }
	.src-cue { color: var(--src-cue); }
	.src-tracked { color: var(--src-tracked); }
	.src-home { color: var(--src-home); }
	.src-none { color: var(--text-faint); }

	.inspector {
		border-top: 1px solid var(--line);
		background: var(--bg-sunken);
		flex-wrap: wrap;
		color: var(--text-dim);
	}
	.inspector .what {
		color: var(--text);
	}
	.inspector label {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.num {
		width: 4.5rem;
	}
	.num,
	.inspector select {
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-xs);
		padding: 3px 6px;
	}
	.unit {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}

	footer {
		border-top: 1px solid var(--line);
		flex-wrap: wrap;
		gap: 12px;
	}
	.key {
		display: flex;
		align-items: center;
		gap: 5px;
		font-size: var(--font-xs);
		color: var(--text-faint);
	}
	.key i {
		width: 9px;
		height: 9px;
		border-radius: 2px;
		background: currentColor;
	}
	.key.src-programmer i { background: var(--src-programmer); }
	.key.src-track i { background: var(--src-track); }
	.key.src-effect i { background: var(--src-effect); }
	.key.src-cue i { background: var(--src-cue); }
	.key.src-tracked i { background: var(--src-tracked); }
	.key.src-home i { background: var(--src-home); }

	.chip {
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 3px 8px;
		cursor: pointer;
	}
	.chip:hover {
		color: var(--text-bright);
		border-color: var(--line-input);
	}
</style>
