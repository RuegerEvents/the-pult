<script lang="ts">
	/**
	 * What a preset holds: which families of parameter, for how many fixtures, and how
	 * much of it the current selection takes. Update and Apply are here as well as in
	 * the menu, because the Inspector is where somebody is already looking.
	 */

	import type { Preset } from '$lib/generated/index.js';
	import { GROUP_TAGS, presetGroups, presetReach } from '$lib/programmer.js';
	import { viewPreset } from '$lib/stores/cues.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { entries, recallPreset, updatePreset } from '$lib/stores/programmer.js';
	import { selection } from '$lib/stores/selection.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { addToast } from '$lib/toasts.js';

	let { rows }: { rows: Preset[] } = $props();

	async function update(preset: Preset) {
		beginGesture();
		try {
			const n = await updatePreset(preset);
			addToast(n > 0 ? `“${preset.name}” updated` : 'the programmer is empty');
		} finally {
			endGesture();
		}
	}
</script>

{#each rows as preset (preset.id)}
	{@const reach = presetReach(preset, $selection)}
	{@const fixtures = new Set(preset.values.map((v) => v.fixture_id)).size}
	<div class="detail">
		{#if rows.length > 1}<h4>{preset.name}</h4>{/if}
		<div class="facts">
			<span class="label">Holds</span>
			<span class="tags">
				{#each presetGroups(preset) as group (group)}<i title={group}>{GROUP_TAGS[group]}</i>{/each}
			</span>
			<span>{preset.values.length} values for {fixtures} fixture{fixtures === 1 ? '' : 's'}</span>
			{#if $selection.length > 0}
				<span class="label">Takes</span><span>{reach.of} of the {reach.all} selected</span>
			{/if}
		</div>
		<div class="row">
			<button class="btn btn-primary" onclick={() => recallPreset(preset, $selection)}>Apply</button>
			{#if $canEdit}
				<button class="btn" disabled={$entries.length === 0} onclick={() => update(preset)}>
					Update from programmer
				</button>
			{/if}
			<button class="btn btn-ghost" onclick={() => viewPreset(preset.id)}>Show in the sheet</button>
		</div>
	</div>
{/each}

<style>
	.detail {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	h4 {
		font-size: var(--font-sm);
	}
	.facts {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 4px 10px;
		align-items: center;
		font-size: var(--font-sm);
	}
	.facts > span:nth-child(3) {
		grid-column: 2;
	}
	.label {
		color: var(--text-dim);
	}
	.tags {
		display: flex;
		gap: 4px;
	}
	.tags i {
		font-style: normal;
		font-size: 10px;
		padding: 2px 5px;
		border-radius: 2px;
		background: var(--bg-sunken);
		color: var(--text-dim);
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
</style>
