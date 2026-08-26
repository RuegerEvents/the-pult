<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type { Fixture, FixtureType } from '$lib/generated/index.js';
	import FixtureTypeEditor from './FixtureTypeEditor.svelte';
	import { channelRange, clashingFixtures, formatValue, parameterKey } from '$lib/patch.js';

	const data = getDataContext();

	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	let creating = $state(false);
	let newName = $state('');
	let newTypeId = $state('');

	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);
	const spanOf = (fixture: Fixture) => typeOf(fixture)?.channel_count ?? 1;

	const clashes = $derived(clashingFixtures(fixtures, spanOf));

	/// The address after the last fixture in a universe, so patching is one click.
	function nextFreeAddress(universe: number): number {
		const used = fixtures.filter((f) => f.universe === universe);
		if (used.length === 0) return 1;
		return Math.max(...used.map((f) => f.dmx_address + Math.max(spanOf(f), 1))) || 1;
	}

	async function createFixture() {
		const name = newName.trim();
		if (!name || !newTypeId) return;
		await data.fixtures.create({
			id: crypto.randomUUID(),
			name,
			fixture_type_id: newTypeId,
			universe: 1,
			dmx_address: nextFreeAddress(1),
			position: null,
			live_values: {},
			active_preset: null
		});
		newName = '';
		creating = false;
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
			<button class="ghost" disabled={types.length === 0} onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Fixture'}
			</button>
		</header>

		{#if creating}
			<form class="new-row" onsubmit={(e) => { e.preventDefault(); createFixture(); }}>
				<input class="text-input" placeholder="Fixture name" bind:value={newName} use:focusOnMount />
				<select class="text-input" bind:value={newTypeId}>
					{#each types as type (type.id)}
						<option value={type.id}>{type.name}</option>
					{/each}
				</select>
				<button class="primary" type="submit">Patch</button>
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
						<th>Name</th><th>Type</th><th>Uni</th><th>Address</th><th>Position</th><th>Live</th><th></th>
					</tr>
				</thead>
				<tbody>
					{#each fixtures as fixture (fixture.id)}
						{@const type = typeOf(fixture)}
						<tr class:clash={clashes.has(fixture.id)}>
							<td>
								<input
									class="text-input"
									value={fixture.name}
									onchange={(e) => data.fixtures.byId(fixture.id).name.set(e.currentTarget.value)}
								/>
							</td>
							<td>
								<select
									class="text-input"
									value={fixture.fixture_type_id}
									onchange={(e) => data.fixtures.byId(fixture.id).fixture_type_id.set(e.currentTarget.value)}
								>
									{#each types as t (t.id)}
										<option value={t.id}>{t.name}</option>
									{/each}
								</select>
							</td>
							<td>
								<input
									class="text-input narrow"
									type="number"
									min="0"
									value={fixture.universe}
									onchange={(e) => data.fixtures.byId(fixture.id).universe.set(Number(e.currentTarget.value))}
								/>
							</td>
							<td>
								<input
									class="text-input narrow"
									type="number"
									min="1"
									max="512"
									value={fixture.dmx_address}
									onchange={(e) => data.fixtures.byId(fixture.id).dmx_address.set(Number(e.currentTarget.value))}
								/>
								<span class="hint">{channelRange(fixture, type?.channel_count ?? 1)}</span>
							</td>
							<td>
								{#if fixture.position}
									{@const p = 'Point' in fixture.position ? fixture.position.Point : fixture.position.Axial.position}
									<span class="coords">{p.x.toFixed(1)}, {p.y.toFixed(1)}, {p.z.toFixed(1)}</span>
								{:else}
									<button
										class="ghost small"
										title="Give this fixture a place in the rig"
										onclick={() => data.fixtures.byId(fixture.id).position.set({ Point: { x: 0, y: 0, z: 0 } })}
									>
										Place
									</button>
								{/if}
							</td>
							<td class="live">
								{#if type}
									{#each type.parameters as param (param.dmx_channel)}
										<span class="chip">{formatValue(fixture.live_values[parameterKey(param.kind)])}</span>
									{/each}
								{/if}
							</td>
							<td><button class="danger" title="Unpatch" onclick={() => data.fixtures.byId(fixture.id).delete()}>×</button></td>
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
	.rig td { padding: 3px 6px 3px 0; vertical-align: middle; }
	.rig tr.clash td { background: #3a1f1f; }
	.coords { color: #bbb; font-variant-numeric: tabular-nums; }
	.live { display: flex; gap: 3px; flex-wrap: wrap; padding-top: 6px; }
	.chip { background: #262626; border: 1px solid #333; border-radius: 3px; padding: 1px 6px; font-size: 11px; color: #bbb; font-variant-numeric: tabular-nums; }
	.hint { color: #777; font-size: 11px; margin-left: 6px; }
	.new-row { display: flex; gap: 6px; margin-bottom: 8px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.warn { color: #e08a55; font-size: 12px; margin-top: 8px; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.text-input.narrow { width: 72px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; cursor: pointer; }
	.ghost.small { padding: 2px 8px; font-size: 12px; }
	.ghost:hover:not(:disabled) { border-color: #555; color: #fff; }
	.ghost:disabled { opacity: 0.4; cursor: not-allowed; }
	.danger { background: none; border: none; color: #777; font-size: 16px; line-height: 1; padding: 4px 8px; cursor: pointer; }
	.danger:hover { color: #e05555; }
</style>
