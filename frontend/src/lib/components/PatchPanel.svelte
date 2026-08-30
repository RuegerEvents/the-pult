<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { editing } from '$lib/stores/editing.js';
	import PositionEditor from './PositionEditor.svelte';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import { select, selected, toggle } from '$lib/stores/selection.js';
	import type { Fixture, FixtureType } from '$lib/generated/index.js';
	import FixtureTypeEditor from './FixtureTypeEditor.svelte';
	import {
		addressLabel,
		channelRange,
		clashingFixtures,
		dmxAddress,
		formatValue,
		nextFreeAddress,
		parameterKey
	} from '$lib/patch.js';

	const data = getDataContext();
	// Selecting is always live; changing the rig is not. See `stores/editing.ts`.
	const unlocked = editing('patch');

	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	let creating = $state(false);
	let newName = $state('');
	let newTypeId = $state('');

	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);
	const spanOf = (fixture: Fixture) => typeOf(fixture)?.channel_count ?? 1;

	const clashes = $derived(clashingFixtures(fixtures, spanOf));

	async function createFixture() {
		const name = newName.trim();
		if (!name || !newTypeId) return;
		await data.fixtures.create({
			id: crypto.randomUUID(),
			name,
			fixture_type_id: newTypeId,
			address: { Dmx: { universe: 1, address: nextFreeAddress(fixtures, 1, spanOf) } },
			position: null,
			live_values: {},
			live_effects: {},
			live_fades: {}
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
	async function setDmx(fixture: Fixture, next: { universe?: number; address?: number }) {
		const current = dmxAddress(fixture.address);
		if (!current) return;
		await data.fixtures.byId(fixture.id).address.set({ Dmx: { ...current, ...next } });
	}

	onMount(() => {
		const stopFixtures = data.fixtures.subscribeDeep((v) => { fixtures = v; });
		const stopTypes = data.fixture_types.subscribeDeep((v) => {
			types = v;
			if (!newTypeId && v.length) newTypeId = v[0].id;
		});
		return () => { stopFixtures(); stopTypes(); };
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
						<th></th><th>Name</th><th>Type</th><th>Uni</th><th>Address</th><th>Position</th><th>Live</th><th></th>
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
									<span class="hint">{channelRange(fixture, type?.channel_count ?? 1)}</span>
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
										<span class="chip">{formatValue(fixture.live_values[parameterKey(param.kind)])}</span>
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
	.patch { padding: 16px 20px; }
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
