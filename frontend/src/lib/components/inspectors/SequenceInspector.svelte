<script lang="ts">
	/**
	 * A sequence's own settings: how its intensities meet another sequence's, and
	 * whether it goes around. See `Merge` and `Sequence::wrap` in
	 * `crates/pult-schema/src/types/sequence.rs`, which are the rules.
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

	/** Not going around is written as nothing, which is what every sequence made before this says. */
	function setWrap(id: string, wrap: boolean) {
		void showData().sequences.byId(id).wrap!.set(wrap ? true : null);
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

		<span class="label"
			>At the end <Help label="What Go does after the last cue"
				>Stay: Go on the last cue does nothing, and the look stays. Go around: Go on the last cue
				takes the first again, and Go back on the first takes the last — a loop of looks.</Help
			></span
		>
		<div class="segmented" role="group" aria-label="At the end">
			<button class:on={!sequence.wrap} aria-pressed={!sequence.wrap} disabled={!$canEdit} onclick={() => setWrap(sequence.id, false)}
				>Stay</button
			>
			<button class:on={!!sequence.wrap} aria-pressed={!!sequence.wrap} disabled={!$canEdit} onclick={() => setWrap(sequence.id, true)}
				>Go around</button
			>
		</div>
	</div>
{/each}
