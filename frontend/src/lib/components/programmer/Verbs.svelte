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
	import { clearSelection } from '$lib/stores/selection.js';
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
	<button
		class="verb quiet"
		disabled={!anything}
		onclick={() => clear({ keepLocked: true })}
		title="Esc — a second Esc clears the selection too"
		ondblclick={() => clearSelection()}
	>
		Clear
	</button>
</div>

{#if storing}
	<StoreMenu preselect={storing.preselect} onclose={() => (storing = null)} />
{/if}

<style>
	.verbs {
		display: flex;
		gap: 4px;
	}
	.verb {
		background: var(--bg-raised, #252525);
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: var(--radius, 4px);
		color: var(--text-dim, #888);
		font: inherit;
		font-size: var(--font-sm, 12px);
		font-weight: 600;
		padding: 4px 12px;
		cursor: pointer;
		white-space: nowrap;
	}
	.verb:hover:not(:disabled) {
		color: var(--text-bright, #fff);
		border-color: var(--live, #f59e0b);
	}
	.verb:disabled {
		color: var(--text-faint, #555);
		cursor: not-allowed;
	}
	.quiet:hover:not(:disabled) {
		border-color: var(--line-input, #555);
	}
</style>
