<script lang="ts">
	/**
	 * Which sensor of which node, for + Add → Sensor…: the fixtures that report something
	 * down the side, and the readings of the one picked, each a tick. All of a node's are
	 * ticked when it is picked, since a node is usually put on a map whole; one already
	 * on this layout says so, and stays addable, since a contact may be drawn twice.
	 */

	import type { Fixture, Layout } from '$lib/generated/index.js';
	import { inputsOf, sensingFixtures, type LayoutShow } from '$lib/layoutView.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import Dialog from '$lib/components/Dialog.svelte';

	let {
		layout,
		show,
		onadd,
		onclose
	}: {
		layout: Layout;
		show: LayoutShow;
		/** The fixture, and the keys of its readings to place. */
		onadd: (fixture: Fixture, keys: Set<string>) => void;
		onclose: () => void;
	} = $props();

	const sensing = $derived(sensingFixtures(show));
	let fixtureId = $state<string | null>(null);
	let keys = $state<Set<string>>(new Set());

	const fixture = $derived(sensing.find((f) => f.id === fixtureId) ?? null);
	const readings = $derived.by(() => {
		const type = fixture && show.types.find((t) => t.id === fixture.fixture_type_id);
		return type ? inputsOf(type).map(({ kind }) => ({ key: parameterKey(kind), label: displayLabel(kind) })) : [];
	});

	/** What this layout already draws of a fixture, by key. */
	const drawn = $derived(
		new Set(
			layout.elements.flatMap((e) =>
				e.target.kind === 'Parameter' && e.target.fixture_id === fixtureId ? [parameterKey(e.target.parameter)] : []
			)
		)
	);

	function pick(f: Fixture) {
		fixtureId = f.id;
		const type = show.types.find((t) => t.id === f.fixture_type_id);
		keys = new Set(type ? inputsOf(type).map(({ kind }) => parameterKey(kind)) : []);
	}

	function toggle(key: string, on: boolean) {
		const next = new Set(keys);
		if (on) next.add(key);
		else next.delete(key);
		keys = next;
	}

	function add() {
		if (!fixture || keys.size === 0) return;
		onadd(fixture, keys);
	}
</script>

<Dialog title="Add a sensor" size="wide" {onclose} onaccept={add}>
	{#if sensing.length === 0}
		<p class="hint">No fixture reports anything. A sensor node's contacts and readings appear here once it is adopted.</p>
	{:else}
		<div class="picker">
			<div class="nodes" role="listbox" aria-label="Fixtures that report something">
				{#each sensing as f (f.id)}
					<button role="option" aria-selected={f.id === fixtureId} class:on={f.id === fixtureId} onclick={() => pick(f)}>{f.name}</button>
				{/each}
			</div>
			<div class="readings">
				{#if !fixture}
					<p class="hint">Pick a node or fixture on the left.</p>
				{:else}
					<label class="check all">
						<input
							type="checkbox"
							checked={keys.size === readings.length}
							indeterminate={keys.size > 0 && keys.size < readings.length}
							onchange={(e) => (keys = new Set(e.currentTarget.checked ? readings.map((r) => r.key) : []))}
						/>
						All of {fixture.name}
					</label>
					{#each readings as r (r.key)}
						<label class="check">
							<input type="checkbox" checked={keys.has(r.key)} onchange={(e) => toggle(r.key, e.currentTarget.checked)} />
							{r.label}
							{#if drawn.has(r.key)}<span class="hint">already on this layout</span>{/if}
						</label>
					{/each}
				{/if}
			</div>
		</div>
	{/if}
	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>Cancel</button>
		<button class="btn btn-primary" disabled={!fixture || keys.size === 0} onclick={add}>
			Add{keys.size > 1 ? ` ${keys.size}` : ''}
		</button>
	{/snippet}
</Dialog>

<style>
	.picker {
		display: grid;
		grid-template-columns: minmax(160px, 1fr) 2fr;
		gap: 12px;
		min-height: 220px;
		max-height: 60vh;
	}
	.nodes,
	.readings {
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.nodes {
		border-right: 1px solid #2a2a2a;
		padding-right: 8px;
	}
	.nodes button {
		text-align: left;
		padding: 7px 9px;
		border-radius: 4px;
		border: 1px solid #333;
		background: #1f2227;
		color: #ddd;
		font-size: 13px;
		cursor: pointer;
	}
	.nodes button.on {
		border-color: var(--accent, #4a90d9);
		color: #fff;
		background: #1e3a5f44;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 7px;
		font-size: 13px;
		color: #ddd;
		padding: 3px 0;
	}
	.check.all {
		border-bottom: 1px solid #2a2a2a;
		padding-bottom: 7px;
		margin-bottom: 3px;
		font-weight: 600;
	}
	.hint {
		color: #777;
		font-size: 12px;
	}
</style>
