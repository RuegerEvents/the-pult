<script lang="ts">
	/**
	 * Store, Update, Clear — the three verbs, in the top bar.
	 *
	 * Every other desk has these under a thumb, and this console had Store buried in a
	 * panel and Update as four clicks inside it. They are in the chrome rather than in
	 * the programmer panel because they are about the *show* and not about that tile:
	 * an operator programming in the rig view, in the plan, or from the command line
	 * means the same three things, and a verb that lives in one panel is a verb that is
	 * missing from the other three.
	 *
	 * **Update needs no target**, which is the whole reason it is one press. A
	 * parameter being driven by a cue says which cue, so the console already knows
	 * where a nudged value belongs — see `updateDriven`. The keys nothing is driving
	 * are the honest exception: the Store dialog opens with exactly those ticked.
	 */

	import { addToast } from '$lib/toasts.js';
	import { clear, entries, updateDriven } from '$lib/stores/programmer.js';
	import { clearStep, nextClear } from '$lib/stores/clear.js';
	import StoreMenu from './StoreMenu.svelte';

	/** Open with a subset ticked — what Update hands over when it cannot place a key. */
	let storing = $state<{ preselect: string[] | null } | null>(null);
	let busy = $state(false);

	const anything = $derived($entries.length > 0);

	export async function openStore(preselect: string[] | null = null) {
		storing = { preselect };
	}

	export async function update() {
		if (busy || !anything) return;
		busy = true;
		try {
			const { updated, orphans } = await updateDriven();
			if (orphans.length > 0) {
				// Not an error and not a silent partial write: the values that had a home
				// went home, and the ones that did not are the dialog's opening list.
				addToast(
					updated > 0
						? `Updated ${updated} ${updated === 1 ? 'cue' : 'cues'} — ${orphans.length} left to store`
						: 'Nothing here is being driven by a cue',
					updated > 0 ? 'success' : undefined
				);
				storing = { preselect: orphans };
			} else if (updated > 0) {
				addToast(`Updated ${updated} ${updated === 1 ? 'cue' : 'cues'}`, 'success');
				await clear({ keepLocked: true });
			}
		} catch (e) {
			addToast(`${e}`);
		} finally {
			busy = false;
		}
	}
</script>

<div class="verbs">
	<button class="verb" disabled={!anything} onclick={() => openStore()} title="⌘⏎">Store</button>
	<button class="verb" disabled={!anything || busy} onclick={update} title="⌘U">Update</button>
	<!-- The selection first, then the programmer: see `stores/clear.ts`. The title says
	     which, so the next press is never a surprise. -->
	<button
		class="verb quiet"
		disabled={$nextClear === null}
		onclick={() => clearStep()}
		title={$nextClear === 'selection'
			? 'Esc — clears the selection; the next press clears the programmer'
			: 'Esc — clears the programmer'}
	>
		Clear
	</button>
</div>

{#if storing}
	<StoreMenu preselect={storing.preselect} onclose={() => (storing = null)} />
{/if}

<style>
	/* One control rather than three buttons: the three are one thought — what to do
	   with what is in the programmer — and a group reads as that. */
	.verbs {
		display: inline-flex;
		height: 30px;
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: 6px;
		overflow: hidden;
	}
	.verb {
		background: none;
		border: none;
		border-right: 1px solid var(--line-strong, #3a3a3a);
		color: var(--text, #ddd);
		font: inherit;
		font-size: var(--font-sm, 12px);
		font-weight: 500;
		padding: 0 14px;
		cursor: pointer;
		white-space: nowrap;
	}
	.verb:last-child {
		border-right: none;
	}
	.verb:hover:not(:disabled) {
		background: var(--bg-hover, #2a2a2a);
		color: var(--text-bright, #fff);
	}
	.verb:disabled {
		color: var(--text-faint, #555);
		cursor: default;
	}
</style>
