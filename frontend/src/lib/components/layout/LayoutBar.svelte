<script lang="ts">
	/**
	 * The layout menu in the top bar: which arrangement is on screen, and what to do
	 * with it.
	 *
	 * Presets are always there and cannot be saved over; a layout in the show can be.
	 * A dot beside the name means the arrangement has been changed and not written
	 * down, which is the whole reason rearranging does not save on its own.
	 */

	import { dropdown } from '$lib/actions.js';
	import { PRESETS } from '$lib/layout/presets.js';
	import {
		active,
		applyLayout,
		applyPreset,
		dirty,
		layouts,
		removeLayout,
		rename,
		resetLayout,
		save,
		saveAs
	} from '$lib/stores/layout.js';
	import NameDialog from '$lib/components/NameDialog.svelte';
	import LayoutGrid from '@lucide/svelte/icons/layout-grid';

	let open = $state(false);
	let naming = $state<'new' | 'rename' | null>(null);
	let draft = $state('');

	const current = $derived.by(() => {
		const a = $active;
		if (a.kind === 'preset') return PRESETS.find((p) => p.key === a.key)?.name ?? 'Layout';
		return $layouts.find((l) => l.id === a.id)?.name ?? 'Layout';
	});
	const savedHere = $derived($active.kind === 'show' ? $active.id : null);

	function start(kind: 'new' | 'rename') {
		draft = kind === 'rename' ? current : '';
		naming = kind;
		open = false;
	}

	async function commit(name: string) {
		if (naming === 'new') await saveAs(name);
		else if (savedHere) await rename(savedHere, name);
	}
</script>

<div class="bar">

		<!-- Saving is in the menu: rearranging is constant and writing it down is rare,
		     and two permanent chips for the rare half crowded the bar. The dot still
		     says that something has not been written down. -->
		<button class="tb-menu" aria-expanded={open} onclick={() => (open = !open)}>
			<LayoutGrid size={16} /><span class="tb-label">{current}</span>{#if $dirty}<span class="dot" title="Changed since it was saved">●</span>{/if}
			<span class="tb-caret">▾</span>
		</button>

	{#if open}
		<div class="menu" use:dropdown={{ close: () => (open = false) }}>
			<span class="heading">Presets</span>
			{#each PRESETS as preset (preset.key)}
				<button
					class:on={$active.kind === 'preset' && $active.key === preset.key}
					onclick={() => {
						applyPreset(preset.key);
						open = false;
					}}
				>{preset.name}</button>
			{/each}

			<span class="heading">In this show</span>
			{#if $layouts.length === 0}
				<span class="none">None yet — arrange the tiles and Save as…</span>
			{:else}
				{#each $layouts as layout (layout.id)}
					<div class="row">
						<button
							class="grow"
							class:on={$active.kind === 'show' && $active.id === layout.id}
							onclick={() => {
								applyLayout(layout);
								open = false;
							}}
						>{layout.name}</button>
						<button
							class="icon"
							aria-label="Delete {layout.name}"
							title="Delete"
							onclick={() => removeLayout(layout.id)}
						>✕</button>
					</div>
				{/each}
			{/if}

			<span class="rule"></span>
			{#if savedHere}
				<button
					disabled={!$dirty}
					onclick={() => {
						save();
						open = false;
					}}
				>Save</button>
			{/if}
			<button onclick={() => start('new')}>Save as new…</button>
			{#if savedHere}
				<button
					onclick={() => {
						start('rename');
					}}
				>Rename…</button>
			{/if}
			<button
				disabled={!$dirty}
				onclick={() => {
					resetLayout();
					open = false;
				}}
			>Discard changes</button>
		</div>
	{/if}
</div>

{#if naming}
	<NameDialog
		title={naming === 'new' ? 'Save the layout as new' : 'Rename the layout'}
		placeholder="Layout name…"
		initial={draft}
		confirm={naming === 'new' ? 'Save' : 'Rename'}
		onsubmit={commit}
		onclose={() => (naming = null)}
	/>
{/if}

<style>
	.bar {
		position: relative;
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.dot {
		color: var(--live);
		font-size: 8px;
	}



	.menu {
		position: absolute;
		top: calc(100% + 6px);
		left: 0;
		z-index: 50;
		display: flex;
		flex-direction: column;
		min-width: 190px;
		padding: 4px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		box-shadow: 0 8px 24px #0009;
	}
	.menu > button,
	.menu .grow {
		text-align: left;
		background: none;
		border: none;
		border-radius: 3px;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 4px 8px;
		cursor: pointer;
	}
	.menu > button:hover:not(:disabled),
	.menu .grow:hover {
		background: var(--bg-hover);
	}
	.menu > button:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}
	.menu .on {
		color: var(--accent);
	}

	.row {
		display: flex;
		align-items: center;
	}
	.grow {
		flex: 1;
		min-width: 0;
	}
	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		padding: 4px 7px;
		cursor: pointer;
	}
	.icon:hover {
		color: var(--bad);
	}

	.heading {
		color: var(--text-faint);
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		padding: 6px 8px 2px;
	}
	.none {
		color: var(--text-faint);
		font-size: var(--font-xs);
		font-style: italic;
		padding: 2px 8px 4px;
	}
	.rule {
		height: 1px;
		background: var(--line);
		margin: 4px 0;
	}
</style>
