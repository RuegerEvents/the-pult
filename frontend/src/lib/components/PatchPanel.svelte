<script lang="ts">
	/**
	 * The patch: which lights the show has, and where their bytes go.
	 *
 * Two columns, one selection. On the left the **universe map** — 512 tiles, what is
	 * where and what clashes, dragged to re-address — with the **detail** of what is
	 * picked under it, which is where anything is changed (`patch/PatchDetail.svelte`),
	 * one fixture or forty. On the right the **fixtures**, grouped by universe, reading
	 * as a patch rather than a wall of number fields: nothing in a row is a control, so a
	 * row is something to pick. A Setup section only — it wants a dialog's whole width.
	 *
	 * *Live* and *Home* are columns somebody switches on, kept per browser: what a light
	 * is doing is the sheet's question and where it rests is the detail's, so by default
	 * they are not in the way of reading the patch.
	 *
	 * A type or a mode is not changed in place. It moves every channel a fixture has and
	 * can drop parameters this show's cues capture, so it is asked as a dialog that says
	 * what it will cost, and nothing is written until Save.
	 */

	import { onMount } from 'svelte';
	import Columns3 from '@lucide/svelte/icons/columns-3';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';

	import type { Cue, Fixture, FixtureType } from '$lib/generated/index.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { output, watching } from '$lib/stores/output.js';
	import { select, selectIds, selected, selection, toggle } from '$lib/stores/selection.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { rangeOf } from '$lib/selection.js';
	import {
		addressLabel,
		clashingFixtures,
		DEFAULT_MODE,
		dmxAddress,
		dmxBreaks,
		droppedByMode,
		fixtureMode,
		footprint,
		formatValue,
		parameterKey
	} from '$lib/patch.js';
	import { CHANNELS, patchOrder } from '$lib/patching.js';
	import { getDataContext } from '$lib/ws/context.js';
	import Dialog from './Dialog.svelte';
	import Help from './Help.svelte';
	import AddFixtures from './patch/AddFixtures.svelte';
	import PatchDetail from './patch/PatchDetail.svelte';
	import UniverseMap from './patch/UniverseMap.svelte';

	const data = getDataContext();

	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	// Only to say what a mode change would cost; nothing here writes a cue.
	let cues = $state<Cue[]>([]);
	let adding = $state(false);
	let search = $state('');

	const COLUMNS_KEY = 'pult.patch.columns';
	let columns = $state<{ live: boolean; home: boolean }>(loadColumns());
	let columnsMenu = $state(false);
	function loadColumns() {
		try {
			return { live: false, home: false, ...JSON.parse(localStorage.getItem(COLUMNS_KEY) ?? '{}') };
		} catch {
			return { live: false, home: false };
		}
	}
	function setColumn(which: 'live' | 'home', on: boolean) {
		columns = { ...columns, [which]: on };
		try {
			localStorage.setItem(COLUMNS_KEY, JSON.stringify(columns));
		} catch {
			// A browser with storage off still shows them, until it reloads.
		}
	}

	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);
	// The span a fixture occupies is its mode's first break, and `channel_count` only for
	// a type that names no modes — which is what `footprint` answers.
	const spanOf = (fixture: Fixture) => footprint(typeOf(fixture), fixture.address)[0] || 1;

	const clashes = $derived(clashingFixtures(fixtures, spanOf));
	const ordered = $derived(patchOrder(fixtures));
	const shown = $derived.by(() => {
		const q = search.trim().toLowerCase();
		if (!q) return ordered;
		return ordered.filter((f) => `${f.name} ${typeOf(f)?.name ?? ''} ${addressLabel(f)}`.toLowerCase().includes(q));
	});
	const chosen = $derived(fixtures.filter((f) => $selected.has(f.id)));

	/** The list, in groups: a heading per universe, then whatever is not on DMX. */
	const groups = $derived.by(() => {
		const out: { key: string; title: string; used: number | null; rows: Fixture[] }[] = [];
		for (const fixture of shown) {
			const dmx = dmxAddress(fixture.address);
			const key = dmx ? `u${dmx.universe}` : 'other';
			let group = out.find((g) => g.key === key);
			if (!group) {
				group = { key, title: dmx ? `Universe ${dmx.universe}` : 'Not on DMX', used: dmx ? 0 : null, rows: [] };
				out.push(group);
			}
			group.rows.push(fixture);
		}
		for (const group of out) {
			if (group.used === null) continue;
			group.used = group.rows.reduce((sum, f) => sum + spanOf(f), 0);
		}
		return out;
	});

	/// The live column's values, evaluated every frame while it is shown — and only then,
	/// because a patch of five hundred lights is a lot of numbers nobody asked for.
	$effect(() => {
		if (!columns.live) return;
		const keys = fixtures.flatMap((fixture) => (typeOf(fixture)?.parameters ?? []).map((p) => `${fixture.id}/${parameterKey(p.kind)}`));
		const registered = watching(keys);
		return () => registered.stop();
	});

	// ── Picking ───────────────────────────────────────────────────────────────

	let anchor = $state<string | null>(null);
	/** Click picks one; Ctrl or Cmd adds or takes away; Shift picks the run from the last one. */
	function pick(event: MouseEvent | PointerEvent, id: string) {
		if (event.metaKey || event.ctrlKey) {
			toggle(id);
			anchor = id;
		} else if (event.shiftKey && anchor) {
			const range = rangeOf(
				shown.map((f) => f.id),
				anchor,
				id
			);
			if (range) selectIds(range);
		} else {
			select(id);
			anchor = id;
		}
	}

	async function moveTo(moves: { id: string; universe: number; address: number }[]) {
		beginGesture();
		try {
			for (const move of moves) {
				const fixture = fixtures.find((f) => f.id === move.id);
				if (!fixture) continue;
				const breaks = dmxBreaks(fixture.address);
				if (breaks.length === 0) continue;
				const mode = fixtureMode(fixture.address) ?? DEFAULT_MODE;
				await data.fixtures.byId(fixture.id).address.set({
					Dmx: { mode, breaks: [{ ...breaks[0], universe: move.universe, address: move.address }, ...breaks.slice(1)] }
				});
			}
		} finally {
			endGesture();
		}
	}

	// ── A type or a mode, asked ───────────────────────────────────────────────

	let pending = $state<{ fixtures: Fixture[]; what: 'mode' | 'type'; value: string; dropped: string[] } | null>(null);

	/** What this show's cues capture on these fixtures, by parameter key. */
	function capturedOn(on: Fixture[]): string[] {
		const ids = new Set(on.map((f) => f.id));
		return [
			...new Set(
				cues
					.flatMap((cue) => cue.captures)
					.filter((capture) => ids.has(capture.fixture_id))
					.map((capture) => parameterKey(capture.parameter_kind))
			)
		];
	}

	function ask(on: Fixture[], what: 'mode' | 'type', value: string) {
		if (what === 'mode') {
			const wanted = typeOf(on[0])?.dmx_modes.find((m) => m.name === value);
			if (!wanted) return;
			pending = { fixtures: on, what, value, dropped: droppedByMode(wanted, capturedOn(on)) };
		} else {
			const next = types.find((t) => t.id === value);
			if (!next) return;
			const keeps = new Set(next.parameters.map((p) => parameterKey(p.kind)));
			pending = { fixtures: on, what, value, dropped: capturedOn(on).filter((key) => !keeps.has(key)) };
		}
	}

	async function savePending() {
		const change = pending;
		pending = null;
		if (!change) return;
		beginGesture();
		try {
			for (const fixture of change.fixtures) {
				if (change.what === 'mode') await setMode(fixture, change.value);
				else await data.fixtures.byId(fixture.id).fixture_type_id.set(change.value);
			}
		} finally {
			endGesture();
		}
	}

	/**
	 * Put a fixture into another of its type's modes. A mode with more breaks than the
	 * fixture has addresses gets the missing ones at the universe of the first, one after
	 * the other: a starting point somebody can correct, rather than a break with nowhere
	 * to go, which sends nothing.
	 */
	async function setMode(fixture: Fixture, mode: string) {
		const wanted = typeOf(fixture)?.dmx_modes.find((m) => m.name === mode);
		if (!wanted) return;
		const breaks = dmxBreaks(fixture.address);
		const first = breaks[0] ?? { universe: 1, address: 1 };
		let next = first.address;
		const filled = wanted.breaks.map((span, index) => {
			if (breaks[index]) return breaks[index];
			const entry = { universe: first.universe, address: next };
			next += Math.max(span, 1);
			return entry;
		});
		await data.fixtures.byId(fixture.id).address.set({ Dmx: { mode, breaks: filled } });
	}

	onMount(() => {
		const stopCues = data.cues.subscribeDeep((v) => (cues = v));
		const stopFixtures = data.fixtures.subscribeDeep((v) => (fixtures = v));
		const stopTypes = data.fixture_types.subscribeDeep((v) => (types = v));
		return () => {
			stopFixtures();
			stopTypes();
			stopCues();
		};
	});

	const modeText = (fixture: Fixture) => {
		const type = typeOf(fixture);
		if (!dmxAddress(fixture.address)) return addressLabel(fixture);
		const span = footprint(type, fixture.address);
		const name = type && type.dmx_modes.length > 0 ? `${fixtureMode(fixture.address) ?? DEFAULT_MODE} · ` : '';
		return `${name}${span.join('+')} ch`;
	};
	const addressText = (fixture: Fixture) => {
		const dmx = dmxAddress(fixture.address);
		if (!dmx) return '—';
		const end = dmx.address + Math.max(1, spanOf(fixture)) - 1;
		return `${dmx.universe}.${String(dmx.address).padStart(3, '0')}–${String(end).padStart(3, '0')}`;
	};
</script>

<div class="patch">
	<header class="bar">
		<span class="title">Patch <span class="count">{fixtures.length} fixture{fixtures.length === 1 ? '' : 's'}</span></span>
		<Help label="How the patch works"
			>Pick lights in the list or on the map — Ctrl or Cmd adds one, Shift a run — and change them in the column beside it, one or many at once. Drag a block on the map to re-address it; dragging a picked one takes every picked light in that universe with it. Each change is one Ctrl-Z.</Help
		>
		<label class="search">
			<Search size={13} />
			<input placeholder="Search…" bind:value={search} />
		</label>
		<span class="menu-anchor">
			<button class="btn btn-ghost" aria-haspopup="menu" aria-expanded={columnsMenu} onclick={() => (columnsMenu = !columnsMenu)}
				><Columns3 size={14} />Columns</button
			>
			{#if columnsMenu}
				<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
				<div class="catch" onclick={() => (columnsMenu = false)}></div>
				<div class="menu" role="menu">
					<label><input type="checkbox" checked={columns.live} onchange={(e) => setColumn('live', e.currentTarget.checked)} /> Live — what it is putting out</label>
					<label><input type="checkbox" checked={columns.home} onchange={(e) => setColumn('home', e.currentTarget.checked)} /> Home — where it rests</label>
				</div>
			{/if}
		</span>
		{#if $canEdit}
			<button class="btn btn-primary" onclick={() => (adding = true)}><Plus size={14} />Add fixtures</button>
		{/if}
	</header>

	<div class="body">
		<div class="universes">
			<UniverseMap {fixtures} {spanOf} selected={$selected} canEdit={$canEdit} onpick={(id, e) => pick(e, id)} onmove={moveTo} />
			<PatchDetail {chosen} {types} canEdit={$canEdit} {spanOf} onask={ask} />
		</div>
		<div class="list">
			{#if fixtures.length === 0}
				<div class="empty">
					<p>Nothing patched yet.</p>
					{#if $canEdit}<button class="btn btn-primary" onclick={() => (adding = true)}><Plus size={14} />Add fixtures</button>{/if}
				</div>
			{:else if shown.length === 0}
				<p class="none">Nothing matches “{search}”.</p>
			{:else}
				<table class="rig">
					<thead>
						<tr>
							<th>Name</th>
							<th>Type</th>
							<th>Mode</th>
							<th>Address</th>
							<th>Placed</th>
							{#if columns.live}<th>Live</th>{/if}
							{#if columns.home}<th>Home</th>{/if}
						</tr>
					</thead>
					<tbody>
						{#each groups as group (group.key)}
							<tr class="group">
								<td colspan={5 + (columns.live ? 1 : 0) + (columns.home ? 1 : 0)}>
									{group.title}{#if group.used !== null}<span> · {group.used} / {CHANNELS}</span>{/if}
								</td>
							</tr>
							{#each group.rows as fixture (fixture.id)}
								{@const type = typeOf(fixture)}
								<tr
									class:clash={clashes.has(fixture.id)}
									class:selected={$selected.has(fixture.id)}
									onclick={(e) => pick(e, fixture.id)}
									aria-selected={$selected.has(fixture.id)}
								>
									<td class="name">{fixture.name}</td>
									<td class="dim">{type?.name ?? '—'}</td>
									<td><span class="chip">{modeText(fixture)}</span></td>
									<td class="mono" class:bad={clashes.has(fixture.id)}>{addressText(fixture)}</td>
									<td class="dim">{fixture.position ? '✓' : '—'}</td>
									{#if columns.live}
										<td class="values">
											{#each type?.parameters ?? [] as param (parameterKey(param.kind))}
												<span class="chip">{formatValue($output.value(fixture.id, parameterKey(param.kind)) ?? undefined)}</span>
											{/each}
										</td>
									{/if}
									{#if columns.home}
										<td class="values">
											{#each type?.parameters ?? [] as param (parameterKey(param.kind))}
												{@const overridden = fixture.home_values[parameterKey(param.kind)]}
												<span class="chip" class:overridden={overridden !== undefined}>{formatValue(overridden ?? param.default_value)}</span>
											{/each}
										</td>
									{/if}
								</tr>
							{/each}
						{/each}
					</tbody>
				</table>
				{#if clashes.size > 0}
					<p class="warn">Red fixtures share channels with another in the same universe.</p>
				{/if}
			{/if}
		</div>
	</div>
</div>

{#if adding}
	<AddFixtures {fixtures} {types} {spanOf} onclose={() => (adding = false)} />
{/if}

{#if pending}
	{@const change = pending}
	{@const type = typeOf(change.fixtures[0])}
	<Dialog title={change.what === 'mode' ? 'Change the mode' : 'Change the fixture type'} onclose={() => (pending = null)}>
		<div class="ask">
			<p>
				<b>{change.fixtures.length === 1 ? change.fixtures[0].name : `${change.fixtures.length} fixtures`}</b>:
				{#if change.what === 'mode'}
					→ <b>{change.value}</b>
					{@const wanted = type?.dmx_modes.find((m) => m.name === change.value)}
					{#if wanted}<span class="dim">({wanted.breaks.join('+')} channels)</span>{/if}
				{:else}
					→ <b>{types.find((t) => t.id === change.value)?.name}</b>
				{/if}
			</p>
			<p class="dim">Every channel {change.fixtures.length === 1 ? 'this fixture sends' : 'these fixtures send'} moves, the moment it is saved.</p>
			{#if change.dropped.length > 0}
				<p class="warn">
					The new {change.what} does not carry {change.dropped.join(', ')}.
					{change.dropped.length === 1 ? 'That parameter is' : 'Those are'} captured in this show's cues and will stop being sent.
				</p>
			{/if}
		</div>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (pending = null)}>Cancel</button>
			<button class="btn btn-primary" onclick={savePending}>Save</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.patch {
		container-type: inline-size;
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 14px 16px;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}
	.title {
		font-weight: 600;
		margin-right: 2px;
	}
	.count {
		color: var(--text-faint);
		font-weight: 400;
		font-size: var(--font-xs);
		margin-left: 6px;
	}
	.search {
		display: flex;
		align-items: center;
		gap: 6px;
		margin-left: auto;
		padding: 0 8px;
		min-height: 30px;
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		color: var(--text-faint);
	}
	.search input {
		width: 140px;
		border: none;
		background: none;
		color: var(--text);
		font: inherit;
		outline: none;
	}
	.menu-anchor {
		position: relative;
	}
	.catch {
		position: fixed;
		inset: 0;
		z-index: 19;
	}
	.menu {
		position: absolute;
		top: calc(100% + 4px);
		right: 0;
		z-index: 20;
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding: 6px;
		min-width: 240px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.45);
	}
	.menu label {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 8px;
		border-radius: 4px;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.menu label:hover {
		background: var(--bg-hover);
	}
	/* Universes on the left, fixtures on the right when there is room; stacked, with
	   the fixtures first, when not. */
	.body {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 16px;
		align-items: start;
	}
	.universes {
		display: flex;
		flex-direction: column;
		gap: 12px;
		min-width: 0;
		order: 2;
	}
	@container (min-width: 760px) {
		.body {
			grid-template-columns: minmax(340px, 0.9fr) minmax(0, 1.1fr);
		}
		.universes {
			order: 0;
			position: sticky;
			top: 0;
		}
	}
	.list {
		min-width: 0;
		overflow-x: auto;
	}
	.rig {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	.rig th {
		text-align: left;
		color: var(--text-dim);
		font-weight: 500;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 4px 8px 6px;
		border-bottom: 1px solid var(--line);
	}
	.rig td {
		padding: 7px 8px;
		border-bottom: 1px solid var(--line);
		vertical-align: middle;
	}
	.rig tbody tr:not(.group) {
		cursor: pointer;
	}
	.rig tbody tr:not(.group):hover td {
		background: var(--bg-hover);
	}
	.rig tr.group td {
		padding: 10px 8px 4px;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
		border-bottom: none;
	}
	.rig tr.group span {
		color: var(--text-faint);
	}
	.rig tr.selected td {
		background: color-mix(in srgb, var(--accent) 16%, transparent);
	}
	.rig tr.clash td {
		background: color-mix(in srgb, var(--bad) 18%, transparent);
	}
	.name {
		font-weight: 500;
	}
	.dim {
		color: var(--text-dim);
	}
	.mono {
		font-variant-numeric: tabular-nums;
		font-family: var(--font-mono, monospace);
		font-size: var(--font-xs);
	}
	.mono.bad {
		color: var(--bad);
	}
	.values {
		display: flex;
		gap: 3px;
		flex-wrap: wrap;
	}
	.chip {
		display: inline-block;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: 9px;
		padding: 0 7px;
		font-size: 10px;
		color: var(--text-dim);
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}
	.chip.overridden {
		border-color: var(--accent);
	}
	.empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 10px;
		padding: 32px 0;
		color: var(--text-faint);
	}
	.empty p,
	.none {
		margin: 0;
		color: var(--text-faint);
		font-size: var(--font-sm);
	}
	.warn {
		color: var(--live);
		font-size: var(--font-xs);
		margin: 8px 0 0;
	}
	.ask {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 14px;
		font-size: var(--font-sm);
	}
	.ask p {
		margin: 0;
	}
</style>
