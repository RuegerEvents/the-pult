<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { editing } from '$lib/stores/editing.js';
	import PositionEditor from './PositionEditor.svelte';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import { select, selected, toggle } from '$lib/stores/selection.js';
	import { output, watching } from '$lib/stores/output.js';
	import type { Cue, Fixture, FixtureType, ParameterValue } from '$lib/generated/index.js';
	import FixtureTypeEditor from './FixtureTypeEditor.svelte';
	import HomeValue from './HomeValue.svelte';
	import {
		addressLabel,
		channelRange,
		clashingFixtures,
		DEFAULT_MODE,
		dmxAddress,
		droppedByMode,
		dmxBreaks,
		fixtureMode,
		footprint,
		formatValue,
		kindLabel,
		nextFreeAddress,
		parameterKey
	} from '$lib/patch.js';

	const data = getDataContext();
	// Selecting is always live; changing the rig is not. See `stores/editing.ts`.
	const unlocked = editing('patch');

	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	let creating = $state(false);
	// Only to say what a mode change would cost; nothing here writes a cue.
	let cues = $state<Cue[]>([]);
	let newName = $state('');
	let newTypeId = $state('');

	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);
	// The span a fixture occupies is its mode's first break, and `channel_count`
	// only for a type that names no modes — which is what `footprint` answers.
	const spanOf = (fixture: Fixture) => footprint(typeOf(fixture), fixture.address)[0] || 1;

	const clashes = $derived(clashingFixtures(fixtures, spanOf));

	/// The whole table's live column, evaluated every frame while the panel is up. The
	/// patch is what this panel is *about*, so its own list is exactly the superset.
	$effect(() => {
		const keys = fixtures.flatMap((fixture) =>
			(typeOf(fixture)?.parameters ?? []).map((p) => `${fixture.id}/${parameterKey(p.kind)}`)
		);
		const registered = watching(keys);
		return () => registered.stop();
	});

	async function createFixture() {
		const name = newName.trim();
		if (!name || !newTypeId) return;
		await data.fixtures.create({
			id: crypto.randomUUID(),
			name,
			fixture_type_id: newTypeId,
			address: {
				Dmx: {
					mode: DEFAULT_MODE,
					breaks: [{ universe: 1, address: nextFreeAddress(fixtures, 1, spanOf) }]
				}
			},
			position: null,
			// Nothing has been reported about it and nothing is driving it yet. Both
			// are the station's to fill in; a client creating a fixture has nothing to
			// say about either.
			sensed_values: {},
			live_effects: {},
			live_fades: {},
			// Nothing to say: it rests where its type says it does, until somebody
			// decides this particular unit is different.
			home_values: {}
		});
		newName = '';
		creating = false;
	}

	/// Click selects this fixture alone; shift-click adds it to (or removes it from)
	/// the selection, the same gesture the plan and the rig use.
	function pick(event: MouseEvent, id: string) {
		if (event.shiftKey) toggle(id);
		else select(id);
	}

	/// Re-address a DMX fixture. Universe and address travel together in the schema,
	/// so changing one has to carry the other along.
	/// Set or clear one parameter's home value on one fixture.
	///
	/// The whole map is written rather than the one key, because that is the shape of
	/// the field: a map is one JSON column and one undoable change. `null` clears the
	/// override, which is not the same as an override of zero — one says "whatever the
	/// type says" and the other says "dark".
	async function setHome(fixture: Fixture, key: string, next: ParameterValue | null) {
		const home = { ...fixture.home_values };
		if (next === null) {
			delete home[key];
		} else {
			home[key] = next;
		}
		await data.fixtures.byId(fixture.id).home_values.set(home);
	}

	/// Move a fixture's *first* break. The others follow their own editor.
	async function setDmx(fixture: Fixture, next: { universe?: number; address?: number }) {
		const breaks = dmxBreaks(fixture.address);
		if (breaks.length === 0) return;
		const mode = fixtureMode(fixture.address) ?? DEFAULT_MODE;
		await data.fixtures.byId(fixture.id).address.set({
			Dmx: { mode, breaks: [{ ...breaks[0], ...next }, ...breaks.slice(1)] }
		});
	}

	/// Move one of a fixture's later breaks, for a mode that has more than one.
	async function setBreak(
		fixture: Fixture,
		index: number,
		next: { universe?: number; address?: number }
	) {
		const breaks = dmxBreaks(fixture.address);
		if (!breaks[index]) return;
		const mode = fixtureMode(fixture.address) ?? DEFAULT_MODE;
		await data.fixtures.byId(fixture.id).address.set({
			Dmx: {
				mode,
				breaks: breaks.map((b, i) => (i === index ? { ...b, ...next } : b))
			}
		});
	}

	/// Put a fixture into another of its type's modes.
	///
	/// A mode with more breaks than the fixture has addresses gets the missing ones at
	/// the universe of the first, one after the other: a starting point somebody can
	/// correct, rather than a break with nowhere to go, which sends nothing.
	/**
	 * Change a fixture's mode, having said what it will cost.
	 *
	 * A basic mode has no zoom, and a cue that captured one goes on saying so while
	 * nothing sends it. Finding that out on stage is the failure this dialogue exists
	 * to prevent — so the parameters this show's cues capture and the new mode does
	 * not place are named *before* the write, and the operator decides.
	 */
	async function changeMode(fixture: Fixture, mode: string) {
		const type = typeOf(fixture);
		const wanted = type?.dmx_modes.find((m) => m.name === mode);
		if (!wanted) return;

		const captured = [
			...new Set(
				cues
					.flatMap((cue) => cue.captures)
					.filter((capture) => capture.fixture_id === fixture.id)
					.map((capture) => parameterKey(capture.parameter_kind))
			)
		];
		const dropped = droppedByMode(wanted, captured);
		if (dropped.length > 0) {
			const list = dropped.join(', ');
			const ok = window.confirm(
				`${mode} does not carry ${list}. ` +
					`${dropped.length === 1 ? 'That parameter is' : 'Those parameters are'} captured in this ` +
					`show's cues and will stop being sent. Change the mode anyway?`
			);
			if (!ok) return;
		}
		await setMode(fixture, mode);
	}

	async function setMode(fixture: Fixture, mode: string) {
		const type = typeOf(fixture);
		const wanted = type?.dmx_modes.find((m) => m.name === mode);
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
		const stopCues = data.cues.subscribeDeep((v) => { cues = v; });
		const stopFixtures = data.fixtures.subscribeDeep((v) => { fixtures = v; });
		const stopTypes = data.fixture_types.subscribeDeep((v) => {
			types = v;
			if (!newTypeId && v.length) newTypeId = v[0].id;
		});
		return () => { stopFixtures(); stopTypes(); stopCues(); };
	});
</script>

<div class="patch">
	<FixtureTypeEditor />

	<section class="block">
		<header class="block-head">
			<h2>Fixtures</h2>
			<!-- Removed from the DOM rather than disabled or dimmed. A control that is
			     visible but inert invites a second, harder press; one that is not there
			     says what it means. -->
			{#if $unlocked}
				<button class="btn btn-ghost" disabled={types.length === 0} onclick={() => (creating = !creating)}>
					{creating ? 'Cancel' : '+ Fixture'}
				</button>
			{/if}
		</header>

		{#if creating && $unlocked}
			<form class="new-row" onsubmit={(e) => { e.preventDefault(); createFixture(); }}>
				<input class="input" placeholder="Fixture name" bind:value={newName} use:focusOnMount />
				<select class="select" bind:value={newTypeId}>
					{#each types as type (type.id)}
						<option value={type.id}>{type.name}</option>
					{/each}
				</select>
				<button class="btn btn-primary" type="submit">Patch</button>
			</form>
		{/if}

		{#if types.length === 0}
			<p class="empty">Add a fixture type first. A fixture is an instance of one.</p>
		{:else if fixtures.length === 0}
			<p class="empty">Nothing patched yet.</p>
		{:else}
			<table class="rig">
				<thead>
					<tr>
						<th></th><th>Name</th><th>Type</th><th>Mode</th><th>Uni</th><th>Address</th><th>Position</th><th>Live</th><th>Home</th><th></th>
					</tr>
				</thead>
				<tbody>
					{#each fixtures as fixture (fixture.id)}
						{@const type = typeOf(fixture)}
						{@const dmx = dmxAddress(fixture.address)}
						<tr class:clash={clashes.has(fixture.id)} class:selected={$selected.has(fixture.id)}>
							<td>
								<button
									class="pick hit"
									class:on={$selected.has(fixture.id)}
									title="Select — shift-click to add to the selection"
									aria-label="Select {fixture.name}"
									aria-pressed={$selected.has(fixture.id)}
									onclick={(e) => pick(e, fixture.id)}
								></button>
							</td>
							<td>
								{#if $unlocked}
									<input
										class="input"
										value={fixture.name}
										onchange={(e) => data.fixtures.byId(fixture.id).name.set(e.currentTarget.value)}
									/>
								{:else}
									{fixture.name}
								{/if}
							</td>
							<td>
								{#if $unlocked}
									<select
										class="select"
										value={fixture.fixture_type_id}
										onchange={(e) => data.fixtures.byId(fixture.id).fixture_type_id.set(e.currentTarget.value)}
									>
										{#each types as t (t.id)}
											<option value={t.id}>{t.name}</option>
										{/each}
									</select>
								{:else}
									{type?.name ?? '—'}
								{/if}
							</td>
							<td class="mode-cell">
								{#if !type || type.dmx_modes.length === 0}
									<!-- A type that names no modes has one all the same, computed
									     by the station from its parameters. There is nothing to
									     pick between, so there is nothing to show. -->
									<span class="hint">—</span>
								{:else if $unlocked}
									<select
										class="select"
										value={fixtureMode(fixture.address) ?? DEFAULT_MODE}
										onchange={(e) => changeMode(fixture, e.currentTarget.value)}
									>
										{#each type.dmx_modes as mode (mode.name)}
											<option value={mode.name}>
												{mode.name} · {mode.breaks.join('+')} ch
											</option>
										{/each}
									</select>
								{:else}
									{fixtureMode(fixture.address) ?? DEFAULT_MODE}
								{/if}
							</td>
							{#if dmx}
								<td>
									{#if $unlocked}
										<input
											class="input narrow"
											type="number"
											min="0"
											value={dmx.universe}
											onchange={(e) => setDmx(fixture, { universe: Number(e.currentTarget.value) })}
										/>
									{:else}
										{dmx.universe}
									{/if}
								</td>
								<td>
									{#if $unlocked}
										<input
											class="input narrow"
											type="number"
											min="1"
											max="512"
											value={dmx.address}
											onchange={(e) => setDmx(fixture, { address: Number(e.currentTarget.value) })}
										/>
									{:else}
										{dmx.address}
									{/if}
									<span class="hint">{channelRange(fixture, spanOf(fixture))}</span>
									{#each dmxBreaks(fixture.address).slice(1) as entry, i}
										<!-- A mode with more than one break sits in more than one
										     place, and they need not be in the same universe. -->
										<span class="brk">
											break {i + 2}:
											{#if $unlocked}
												<input
													class="input tiny"
													type="number"
													min="0"
													value={entry.universe}
													onchange={(e) => setBreak(fixture, i + 1, { universe: Number(e.currentTarget.value) })}
												/>/<input
													class="input tiny"
													type="number"
													min="1"
													max="512"
													value={entry.address}
													onchange={(e) => setBreak(fixture, i + 1, { address: Number(e.currentTarget.value) })}
												/>
											{:else}
												{entry.universe} / {entry.address}
											{/if}
										</span>
									{/each}
								</td>
							{:else}
								<!-- A node fixture is addressed by the device it was adopted from,
								     so there is nothing here to type into. -->
								<td colspan="2" class="node-address">{addressLabel(fixture)}</td>
							{/if}

							<td class="position-cell">
								{#if $unlocked}
									<!-- Dragging in the plan is right for a whole rig at once and
									     useless for the one light that has to be at exactly 4.2
									     metres because the drawing says so. -->
									<PositionEditor
										position={fixture.position}
										onchange={(next) => data.fixtures.byId(fixture.id).position.set(next)}
									/>
								{:else if fixture.position}
									{@const p = 'Point' in fixture.position ? fixture.position.Point : fixture.position.Axial.position}
									<span class="coords">{p.x.toFixed(1)}, {p.y.toFixed(1)}, {p.z.toFixed(1)}</span>
								{:else}
									<span class="hint">not placed</span>
								{/if}
							</td>
							<td class="live">
								{#if type}
									{#each type.parameters as param (parameterKey(param.kind))}
										<span class="chip"
											>{formatValue(
												$output.value(fixture.id, parameterKey(param.kind)) ?? undefined
											)}</span
										>
									{/each}
								{/if}
							</td>
							<td class="home">
								<!-- Where each parameter rests when nothing is driving it. The
								     type's answer is what the node said about its own port; an
								     override is this unit's, and is the only place a house light
								     can say that it comes up rather than going dark. -->
								{#if type && $unlocked}
									<!-- How an operator actually sets a house light's: aim it, look
									     at it, keep it. The station reads its own output, so nothing
									     here has to know what is on stage. -->
									<button
										class="take-home"
										title="Keep what {fixture.name} is putting out now"
										aria-label="Take {fixture.name}'s home values from its output"
										onclick={() => data.fixtures.takeHome({ fixtureId: fixture.id })}
									>take</button>
								{/if}
								{#if type}
									{#each type.parameters as param (parameterKey(param.kind))}
										{@const key = parameterKey(param.kind)}
										{@const overridden = fixture.home_values[key]}
										{#if $unlocked}
											<span class="home-cell" class:overridden={overridden !== undefined}>
												<HomeValue
													label={kindLabel(param.kind)}
													value={overridden ?? param.default_value}
													onchange={(next) => setHome(fixture, key, next)}
												/>
												{#if overridden !== undefined}
													<button
														class="clear-home"
														title="Back to what the type says"
														aria-label="Clear the home value for {kindLabel(param.kind)}"
														onclick={() => setHome(fixture, key, null)}
													>×</button>
												{/if}
											</span>
										{:else}
											<span class="chip" class:overridden={overridden !== undefined}>
												{formatValue(overridden ?? param.default_value)}
											</span>
										{/if}
									{/each}
								{/if}
							</td>
							<td>
								{#if $unlocked}
									<button
										class="btn btn-danger btn-icon"
										title="Unpatch {fixture.name}"
										onclick={() => data.fixtures.byId(fixture.id).delete()}
									>×</button>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
			{#if clashes.size > 0}
				<p class="warn">Highlighted fixtures share channels with another fixture in the same universe.</p>
			{/if}
		{/if}
	</section>
</div>

<style>
	/* Two numbers side by side for a break, in a cell already holding an address. */
	.brk { display: block; color: #999; font-size: 11px; }
	.input.tiny { width: 46px; }
	.mode-cell .select { max-width: 170px; }

	.patch { padding: 16px 20px; }
	/* An overridden home value is this rig's answer rather than the type's, and it
	   should be possible to see which is which at a glance down the column. */
	.home-cell { display: inline-flex; align-items: center; gap: 2px; margin-right: 6px; }
	.home-cell.overridden, .chip.overridden { outline: 1px solid var(--accent, #c90); border-radius: 3px; }
	.clear-home { background: none; border: 0; color: inherit; cursor: pointer; opacity: 0.6; padding: 0 2px; }
	.clear-home:hover { opacity: 1; }
	/* Before the values rather than beside one of them: it takes the whole fixture,
	   and a button sitting next to Intensity would read as Intensity's. */
	.take-home {
		font: inherit;
		font-size: 11px;
		margin-right: 6px;
		padding: 1px 6px;
		border: 1px solid var(--border, #444);
		border-radius: 3px;
		background: transparent;
		color: #999;
		cursor: pointer;
	}
	.take-home:hover { color: inherit; border-color: #666; }
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.rig { width: 100%; border-collapse: collapse; font-size: 13px; }
	.rig th { text-align: left; color: #777; font-weight: 500; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; padding: 0 6px 6px 0; }
	/* Rows tall enough to hit standing up. Reading is the common case, so the height
	   comes from padding rather than from every cell holding a 44px control. */
	.rig td { padding: 8px 6px; vertical-align: middle; height: var(--hit); }
	.rig tr.clash td { background: #3a1f1f; }
	.rig tr.selected td { background: #1a2a40; }
	/* Drawn small so a row of them is still a table; `.hit` puts a finger-sized
	   target around each one, because selecting is what an operator does most and
	   the one thing that stays live when the panel is locked. */
	.pick { width: 14px; height: 14px; border-radius: 50%; border: 1px solid var(--line-input); background: none; padding: 0; cursor: pointer; display: block; }
	.pick:hover { border-color: var(--accent); }
	.pick.on { background: var(--accent); border-color: var(--accent); }
	.coords { color: #bbb; font-variant-numeric: tabular-nums; }
	.node-address { color: #bbb; font-family: monospace; font-size: 12px; }
	.live { display: flex; gap: 3px; flex-wrap: wrap; padding-top: 6px; }
	.chip { background: #262626; border: 1px solid #333; border-radius: 3px; padding: 1px 6px; font-size: 11px; color: #bbb; font-variant-numeric: tabular-nums; }
	.hint { color: #777; font-size: 11px; margin-left: 6px; }
	.new-row { display: flex; gap: 6px; margin-bottom: 8px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.warn { color: #e08a55; font-size: 12px; margin-top: 8px; }
	/* Buttons and inputs come from `styles/controls.css` now; `.narrow` is the one
	   size this table needs that the shared sheet has no opinion about. */
	.input.narrow { width: 5rem; }
	.position-cell { min-width: 20rem; }
</style>
