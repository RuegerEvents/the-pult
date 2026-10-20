<script lang="ts">
	/**
	 * How each pixel strip on one node is patched: the node drawing it from nine
	 * parameters, or the console drawing every pixel and sending it as sACN.
	 *
	 * The station works the cells out — where each pixel lands, the spill into the next
	 * universe — so this asks with `device.strip` and shows what comes back. Nothing
	 * here knows the spill rule; the universes it prints are read off the cells.
	 */
	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { collection } from '$lib/stores/show.js';
	import { recall, select } from '$lib/stores/selection.js';
	import type { DiscoveredDevice, Fixture, PixelStrip, StripMode } from '$lib/generated/index.js';
	import { stripRows, stripSummary } from '$lib/strips.js';

	let { device, unlocked }: { device: DiscoveredDevice; unlocked: boolean } = $props();

	const client = getClientContext();
	const fixtures = collection('fixtures');
	const groups = collection('groups');

	const rows = $derived(stripRows(device, $fixtures as Fixture[], $groups));

	/** What the operator is changing, per head, until Apply. */
	let drafts = $state<Record<number, PixelStrip>>({});
	let busy = $state<number | null>(null);

	function draftOf(head: number, strip: PixelStrip): PixelStrip {
		return drafts[head] ?? strip;
	}

	function edit(head: number, strip: PixelStrip, patch: Partial<PixelStrip>) {
		drafts = { ...drafts, [head]: { ...draftOf(head, strip), ...patch } };
	}

	async function apply(head: number, strip: PixelStrip) {
		const next = draftOf(head, strip);
		busy = head;
		try {
			await client.call('device.strip', {
				serial: device.serial,
				head,
				mode: next.mode,
				count: next.count,
				order: next.order,
				universe: next.mode === 'sacn' && next.universe > 0 ? next.universe : undefined,
				address: next.address,
				pitchM: next.pitch_m
			});
			const { [head]: _, ...rest } = drafts;
			drafts = rest;
		} catch (e) {
			addToast(`${device.name}: ${e}`);
		} finally {
			busy = null;
		}
	}

	function selectCells(groupId: string | null) {
		const group = $groups.find((g) => g.id === groupId);
		if (group) recall(group.query);
	}

	const orders = $derived(device.pixels?.orders ?? ['GRB', 'RGB']);
	const max = $derived(device.pixels?.max_count ?? 1024);
</script>

{#if rows.length > 0}
	<div class="strips">
		{#each rows as row (row.head)}
			{@const draft = draftOf(row.head, row.strip)}
			{@const changed = !!drafts[row.head]}
			<div class="strip">
				<div class="strip-head">
					<span class="name">{row.fixture.name}</span>
					<span class="summary dim">{stripSummary(row.strip, row.cells)}</span>
					{#if row.mode === 'sacn'}
						<button class="chip-btn" onclick={() => selectCells(row.groupId)} title="Select every pixel, in order along the strip">
							Select pixels
						</button>
					{:else}
						<button class="chip-btn" onclick={() => select(row.fixture.id)} title="Select the strip, to set what the node draws">
							Select
						</button>
					{/if}
				</div>
				{#if row.running && row.running.mode !== row.strip.mode}
					<p class="warn">The node is running it as {row.running.mode}; it is being put back.</p>
				{/if}
				{#if unlocked}
					<div class="controls">
						<div class="segmented" role="radiogroup" aria-label="Mode">
							{#each [['effect', 'Node effect'], ['sacn', 'sACN pixels']] as [mode, label] (mode)}
								<button
									role="radio"
									aria-checked={draft.mode === mode}
									class:on={draft.mode === mode}
									onclick={() => edit(row.head, row.strip, { mode: mode as StripMode })}
								>{label}</button>
							{/each}
						</div>
						<label>Pixels
							<input type="number" min="1" max={max} value={draft.count}
								oninput={(e) => edit(row.head, row.strip, { count: Number(e.currentTarget.value) })} />
						</label>
						<label>Order
							<select value={draft.order}
								onchange={(e) => edit(row.head, row.strip, { order: e.currentTarget.value })}>
								{#each orders as order (order)}<option value={order}>{order}</option>{/each}
							</select>
						</label>
						{#if draft.mode === 'sacn'}
							<label>Universe
								<input type="number" min="1" max="63999" value={draft.universe || ''} placeholder="next free"
									oninput={(e) => edit(row.head, row.strip, { universe: Number(e.currentTarget.value) })} />
							</label>
							<label>Address
								<input type="number" min="1" max="512" value={draft.address}
									oninput={(e) => edit(row.head, row.strip, { address: Number(e.currentTarget.value) })} />
							</label>
							<label>Per meter
								<input type="number" min="1" max="200" value={Math.round(1 / draft.pitch_m)}
									oninput={(e) => edit(row.head, row.strip, { pitch_m: 1 / Math.max(1, Number(e.currentTarget.value)) })} />
							</label>
						{/if}
						<button class="chip-btn apply" disabled={!changed || busy === row.head} onclick={() => apply(row.head, row.strip)}>
							{busy === row.head ? 'Patching…' : 'Apply'}
						</button>
					</div>
				{/if}
			</div>
		{/each}
		{#each device.faults as fault (fault)}
			<p class="bad">{fault}</p>
		{/each}
	</div>
{/if}

<style>
	.strips {
		grid-column: 1 / -1;
		display: flex;
		flex-direction: column;
		gap: 6px;
		margin-top: 4px;
	}
	.strip {
		border-left: 2px solid var(--accent);
		padding: 4px 0 4px 8px;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.strip-head {
		display: flex;
		align-items: baseline;
		gap: 8px;
		font-size: 12px;
	}
	.name { color: var(--text); }
	.summary { font-size: 11px; }
	.dim { color: var(--text-dim); }
	.controls {
		display: flex;
		flex-wrap: wrap;
		align-items: end;
		gap: 8px;
		font-size: 11px;
	}
	label {
		display: flex;
		flex-direction: column;
		gap: 2px;
		color: var(--text-dim);
	}
	input, select {
		width: 72px;
		font: inherit;
		background: #1a1a1a;
		color: var(--text);
		border: 1px solid #444;
		border-radius: 3px;
		padding: 2px 4px;
	}
	.segmented {
		display: flex;
		border: 1px solid #444;
		border-radius: 3px;
		overflow: hidden;
	}
	.segmented button {
		font: inherit;
		background: transparent;
		color: #888;
		border: none;
		padding: 4px 8px;
		cursor: pointer;
	}
	.segmented button.on {
		background: var(--accent);
		color: #111;
	}
	.chip-btn {
		font-size: 0.68rem;
		padding: 3px 8px;
		border-radius: 3px;
		border: 1px solid #444;
		background: transparent;
		color: #888;
		cursor: pointer;
	}
	.chip-btn:disabled { opacity: 0.4; cursor: not-allowed; }
	.chip-btn.apply:not(:disabled) { border-color: var(--accent); color: var(--accent); }
	.warn { color: #e0a355; font-size: 11px; margin: 0; }
	.bad { color: var(--bad); font-size: 11px; margin: 0; }
</style>
