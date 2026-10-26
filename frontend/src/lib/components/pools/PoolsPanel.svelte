<script lang="ts">
	/**
	 * The pools: saved groups, and presets.
	 *
	 * Two rows of cards (`objects/Card.svelte`), which is what every other desk calls
	 * direct selects. Tap acts, ⌘-click or the corner dot inspects, right-click or a
	 * long-press is the shared menu — the kinds themselves are in `$lib/kinds/`. A group
	 * button recalls a *question* about the rig; a preset button recalls a look. Both
	 * are the show's, so they are here on every console rather than in one browser's
	 * storage.
	 *
	 * Sequences are deliberately not here. A pool of sequences would be an executor
	 * page, which is a bound physical thing and a different piece of work — the
	 * Playback and Cues panels are what runs a sequence today.
	 *
	 * **The group tags are derived and never stored.** A preset that grows a color is
	 * a color preset from that moment, which is why the pool is one flat list with a
	 * filter over it rather than five pools somebody has to file things into.
	 *
	 * And a preset button says **how much of it applies**: with a selection it is the
	 * intersection, and "4 of 6" is the difference between a button that will do what
	 * it looks like and one that will not.
	 */

	import type { Preset } from '$lib/generated/index.js';
	import type { FadeGroup } from '$lib/generated/index.js';
	import { GROUP_TAGS, presetGroups, presetReach } from '$lib/programmer.js';
	import { collection } from '$lib/stores/show.js';
	import { entries, storePreset } from '$lib/stores/programmer.js';
	import { presetInView } from '$lib/stores/cues.js';
	import { asSavedQuery, selection } from '$lib/stores/selection.js';
	import { showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { addToast } from '$lib/toasts.js';
	import NameDialog from '$lib/components/NameDialog.svelte';
	import Card from '$lib/components/objects/Card.svelte';
	import Foldered from '$lib/components/objects/Foldered.svelte';
	import { uuid } from '$lib/uuid.js';

	const groups = collection('groups');
	const presets = collection('presets');

	const GROUP_ORDER: FadeGroup[] = ['Intensity', 'Position', 'Color', 'Beam', 'Other'];
	let filter = $state<FadeGroup | null>(null);
	let naming = $state(false);

	const shown = $derived(
		filter === null
			? ($presets as Preset[])
			: ($presets as Preset[]).filter((p) => presetGroups(p).includes(filter as FadeGroup))
	);

	/** A group from what is selected: the question, not the list, so a light patched
	 *  later that answers it joins it. */
	let namingGroup = $state(false);
	async function makeGroup(name: string) {
		try {
			await showData().groups.create({ id: uuid(), name, query: asSavedQuery(), color: null });
			addToast(`Group “${name}” saved`, 'success');
		} catch (e) {
			addToast(`${e}`);
		}
	}

	async function makePreset(name: string) {
		try {
			await storePreset(name);
			addToast(`Preset “${name}” stored`, 'success');
		} catch (e) {
			addToast(`${e}`);
		}
	}
</script>

<div class="pools">
	<section>
		<header>
			<h2>Groups</h2>
			{#if $canEdit}
				<button
					class="chip"
					disabled={$selection.length === 0}
					title={$selection.length === 0 ? 'Select some fixtures first' : `Save the ${$selection.length} selected as a group`}
					onclick={() => (namingGroup = true)}
				>
					+ From selection
				</button>
			{/if}
		</header>
		{#if $groups.length === 0}
			<p class="empty">No saved groups. Select some fixtures and press <b>+ From selection</b>.</p>
		{:else}
			<Foldered kind="group" rows={$groups}>
				{#snippet item(group)}
					<Card ref={{ kind: 'group', id: group.id }} name={group.name} color={group.color} />
				{/snippet}
			</Foldered>
		{/if}
	</section>

	<section>
		<header>
			<h2>Presets</h2>
			<div class="filter">
				<button class="tag" class:on={filter === null} onclick={() => (filter = null)}>All</button>
				{#each GROUP_ORDER as group (group)}
					<button
						class="tag"
						class:on={filter === group}
						title={group}
						onclick={() => (filter = filter === group ? null : group)}
					>
						{GROUP_TAGS[group]}
					</button>
				{/each}
			</div>
			{#if $canEdit}
				<button
					class="chip"
					disabled={$entries.length === 0}
					title={$entries.length === 0
						? 'Set something in the programmer first'
						: 'Store what the programmer holds as a preset'}
					onclick={() => (naming = true)}
				>
					+ From programmer
				</button>
			{/if}
		</header>

		{#if $presets.length === 0}
			<p class="empty">
				No presets yet. Set a look in the programmer, then <b>+ From programmer</b>. A cue
				that uses a preset changes when the preset does.
			</p>
		{:else if shown.length === 0}
			<p class="empty">No preset holds anything in that group.</p>
		{:else}
			<Foldered kind="preset" rows={shown}>
				{#snippet item(preset)}
					{@const reach = presetReach(preset, $selection)}
					<Card
						ref={{ kind: 'preset', id: preset.id }}
						name={preset.name}
						color={preset.color}
						dim={reach.of === 0}
						on={$presetInView === preset.id}
						state={[
							presetGroups(preset).map((g) => GROUP_TAGS[g]).join(' · '),
							$selection.length > 0 ? `${reach.of} of ${reach.all}` : ''
						]
							.filter(Boolean)
							.join('  ·  ')}
					/>
				{/snippet}
			</Foldered>
			<p class="note">
				Tap to apply · ⌘-click or the corner dot to inspect · right-click or hold for more.
				Deleting a preset changes no cue: each one keeps the value it was stored with
				and says the preset is missing.
			</p>
		{/if}
	</section>
</div>

{#if namingGroup}
	<NameDialog
		title="Save the selection as a group"
		placeholder="Group name…"
		confirm="Save"
		hint="The group keeps the question, not the list: a light patched later that answers it joins it."
		onsubmit={makeGroup}
		onclose={() => (namingGroup = false)}
	/>
{/if}
{#if naming}
	<NameDialog
		title="Store the programmer as a preset"
		placeholder="Preset name…"
		confirm="Store"
		onsubmit={makePreset}
		onclose={() => (naming = false)}
	/>
{/if}

<style>
	.pools {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 10px;
	}

	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	header {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
	}

	h2 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}

	.filter {
		display: flex;
		gap: 2px;
		margin-left: auto;
	}
	.tag {
		background: none;
		border: 1px solid transparent;
		border-radius: var(--radius);
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		padding: 2px 7px;
		cursor: pointer;
	}
	.tag:hover {
		color: var(--text);
	}
	.tag.on {
		border-color: var(--accent);
		color: var(--accent);
	}





	.empty,
	.note {
		color: var(--text-faint);
		font-size: var(--font-xs);
		font-style: italic;
		max-width: 60ch;
	}

	.chip {
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 4px 10px;
		cursor: pointer;
	}
	.chip:hover:not(:disabled) {
		color: var(--text-bright);
		border-color: var(--line-input);
	}
	.chip:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}

</style>
