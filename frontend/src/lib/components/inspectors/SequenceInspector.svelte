<script lang="ts">
	/**
	 * A sequence's own settings: for now, how its intensities meet another sequence's.
	 * See `Merge` in `crates/pult-schema/src/types/sequence.rs`, which is the rule.
	 */

	import type { Merge, Sequence } from '$lib/generated/index.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { showData } from '$lib/stores/show.js';
	import Help from '../Help.svelte';

	let { rows }: { rows: Sequence[] } = $props();

	/** LTP is written as nothing, which is what every sequence made before this says. */
	function setMerge(id: string, merge: Merge) {
		// Optional on the row, so the proxy types it as maybe absent; as a path it is always there.
		void showData().sequences.byId(id).merge!.set(merge === 'Ltp' ? null : merge);
	}

	const MERGES: [Merge, string][] = [
		['Ltp', 'LTP'],
		['Htp', 'HTP']
	];
</script>

{#each rows as sequence (sequence.id)}
	{@const merge = sequence.merge ?? 'Ltp'}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{sequence.name}</span>{/if}

		<span class="label"
			>Merge <Help label="How it meets other sequences"
				>LTP: the sequence that went last owns a light outright, and taking it off gives the light back
				to whatever is still on. HTP: a light another sequence is playing shows whichever is higher, each
				at its own fader — so pulling this one down leaves the other's level where it was. Intensities
				only; a color or a position always follows the last Go.</Help
			></span
		>
		<div class="segmented" role="group" aria-label="Merge">
			{#each MERGES as [value, label] (value)}
				<button
					class:on={merge === value}
					aria-pressed={merge === value}
					disabled={!$canEdit}
					onclick={() => void setMerge(sequence.id, value)}
					>{label}</button
				>
			{/each}
		</div>
	</div>
{/each}
