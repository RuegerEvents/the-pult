<script lang="ts">
	/**
	 * The quicksheet: one fixture's attributes, where the fixture is.
	 *
	 * The spec asks for this by name — select an effector and get "a quicksheet
	 * (similar to ETC) where all attributes that change the properties of that
	 * effector can be manipulated". The point of it is that it appears *at* the
	 * fixture in the stage view, so programming happens where the light is rather
	 * than in a panel somewhere else on the screen.
	 *
	 * It writes through the same store as the values panel, so a level set here shows
	 * up there and both end in the same buffer.
	 */

	import type { Fixture, ParameterValue } from '$lib/generated/index.js';
	import { commonValue, editableParameters } from '$lib/programmer.js';
	import { formatValue } from '$lib/patch.js';
	import { collection } from '$lib/stores/show.js';
	import { byKey, remove, setValue } from '$lib/stores/programmer.js';
	import PanTiltPad from './controls/PanTiltPad.svelte';
	import ValueControl from './controls/ValueControl.svelte';
	import { output, watching } from '$lib/stores/output.js';

	let { fixture, onclose }: { fixture: Fixture; onclose?: () => void } = $props();

	const types = collection('fixture_types');

	const rows = $derived(editableParameters($types, [fixture]));
	const held = $derived(rows.filter((row) => $byKey.has(`${fixture.id}/${row.key}`)));

	const panRow = $derived(rows.find((r) => r.kind === 'Pan') ?? null);
	const tiltRow = $derived(rows.find((r) => r.kind === 'Tilt') ?? null);
	const others = $derived(rows.filter((r) => r.kind !== 'Pan' && r.kind !== 'Tilt'));

	/// This sheet's own parameters, evaluated every frame for as long as it is open.
	$effect(() => {
		const registered = watching(rows.map((row) => `${fixture.id}/${row.key}`));
		return () => registered.stop();
	});

	function valueOf(key: string, fallback: ParameterValue): ParameterValue {
		return (
			$byKey.get(`${fixture.id}/${key}`)?.value ?? $output.value(fixture.id, key) ?? fallback
		);
	}

	const axis = (row: { key: string; defaultValue: ParameterValue } | null) => {
		if (!row) return null;
		const value = valueOf(row.key, row.defaultValue);
		return value.type === 'Float' ? value.value : null;
	};

	/// Only this fixture, whatever else is selected: the sheet is about the thing it
	/// is attached to.
	const put = (kind: (typeof rows)[number]['kind'], value: ParameterValue) =>
		setValue([fixture.id], kind, value);

	async function clearThis() {
		for (const row of held) {
			const entry = $byKey.get(`${fixture.id}/${row.key}`);
			if (entry) await remove(entry.id);
		}
	}
</script>

<div class="sheet">
	<header>
		<span class="name">{fixture.name}</span>
		<span class="spacer"></span>
		<button class="ghost" disabled={held.length === 0} onclick={clearThis}>Clear</button>
		{#if onclose}
			<button class="icon" aria-label="Close" onclick={onclose}>✕</button>
		{/if}
	</header>

	{#if panRow || tiltRow}
		<PanTiltPad
			pan={axis(panRow)}
			tilt={axis(tiltRow)}
			onpan={panRow ? (v) => put('Pan', { type: 'Float', value: v }) : undefined}
			ontilt={tiltRow ? (v) => put('Tilt', { type: 'Float', value: v }) : undefined}
		/>
	{/if}

	{#each others as row (row.key)}
		{@const value = valueOf(row.key, row.defaultValue)}
		<div class="row" class:held={$byKey.has(`${fixture.id}/${row.key}`)}>
			<span class="label">{row.label}</span>
			<span class="readout mono">
				{formatValue(commonValue([fixture], row.key, $output).value ?? undefined)}
			</span>
			<ValueControl
				{value}
				label={row.label}
				tint={$byKey.has(`${fixture.id}/${row.key}`) ? 'var(--live)' : 'var(--accent)'}
				oninput={(next) => put(row.kind, next)}
			/>
		</div>
	{/each}

	{#if rows.length === 0}
		<p class="empty">Nothing on this fixture can be set.</p>
	{/if}
</div>

<style>
	.sheet {
		display: flex;
		flex-direction: column;
		gap: 6px;
		width: 232px;
		padding: 8px;
		background: #1e1e1eee;
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 6px 20px #0008;
		backdrop-filter: blur(3px);
	}

	header {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.name {
		color: var(--text-bright);
		font-size: var(--font-sm);
		font-weight: 500;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.spacer {
		flex: 1;
	}

	.row {
		display: grid;
		grid-template-columns: 1fr auto;
		gap: 2px 6px;
		padding: 3px 4px;
		margin: 0 -4px;
		border-radius: 3px;
	}
	.row.held {
		background: #f59e0b14;
	}
	.row :global(> :nth-child(3)) {
		grid-column: 1 / -1;
	}

	.label {
		color: var(--text);
		font-size: var(--font-xs);
	}
	.readout {
		color: var(--text-dim);
		font-size: var(--font-xs);
		text-align: right;
	}
	.mono {
		font-family: monospace;
	}

	.empty {
		color: var(--text-faint);
		font-size: var(--font-xs);
		font-style: italic;
	}

	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: #bbb;
		padding: 2px 8px;
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

	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		line-height: 1;
		cursor: pointer;
	}
	.icon:hover {
		color: var(--bad);
	}
</style>
