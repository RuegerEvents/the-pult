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
	 * where a nudged value belongs — see `infra/storing.rs`, which the wing's UPDATE
	 * runs too. The keys nothing is driving are the honest exception: the Store dialog
	 * opens with exactly those ticked.
	 */

	import { addToast } from '$lib/toasts.js';
	import { cancelEdit, editingCue, entries, update as runUpdate } from '$lib/stores/programmer.js';
	import { collection } from '$lib/stores/show.js';
	import { clearStep, nextClear } from '$lib/stores/clear.js';
	import { wingStoreAsk } from '$lib/stores/wing.js';
	import StoreMenu from './StoreMenu.svelte';
	import WingEdit from '../wing/WingEdit.svelte';
	import type { StoreOnto } from '$lib/generated/index.js';

	/** Open with a subset ticked — what Update hands over when it cannot place a key. */
	let storing = $state<{ preselect: string[] | null; onto?: StoreOnto | null } | null>(null);
	let busy = $state(false);

	const anything = $derived($entries.length > 0);

	export async function openStore(preselect: string[] | null = null) {
		storing = { preselect };
	}

	// STORE and PLEASE on the wing, or an Update there that left keys no cue drives:
	// the station runs the wing and has no dialog to open, so it asks this one.
	$effect(() => {
		const ask = $wingStoreAsk;
		if (!ask) return;
		wingStoreAsk.set(null);
		storing = { preselect: ask.preselect.length > 0 ? ask.preselect : null, onto: ask.onto };
	});

	const cues = collection('cues');
	/** The cue loaded for editing, if one is: Update writes back into it, Cancel drops it. */
	const editing = $derived($cues.find((c) => c.id === $editingCue) ?? null);

	/**
	 * The station's Update (`programmer.update`), the one the wing's UPDATE runs: into
	 * the cue loaded for editing when there is one, else into the cue driving each value.
	 */
	export async function update() {
		if (busy || !anything) return;
		busy = true;
		try {
			const { updated, orphans, edited } = await runUpdate();
			if (edited) {
				addToast(`Updated cue ${edited.number.toFixed(1)} · ${edited.name}`, 'success');
			} else if (orphans.length > 0) {
				// Not an error and not a silent partial write: the values that had a home
				// went home, and the ones that did not are the dialog's opening list.
				addToast(
					updated > 0
						? `Updated ${updated} ${updated === 1 ? 'cue' : 'cues'} — ${orphans.length} left to store`
						: 'Nothing here is being driven by a cue',
					updated > 0 ? 'success' : undefined
				);
				storing = { preselect: orphans };
			} else {
				addToast(`Updated ${updated} ${updated === 1 ? 'cue' : 'cues'}`, 'success');
			}
		} catch (e) {
			addToast(e instanceof Error ? e.message : `${e}`);
		} finally {
			busy = false;
		}
	}
</script>

{#if editing}
	<!-- A cue loaded for editing: Update beside it writes the programmer back into it
	     and leaves, Cancel leaves without. Here and not only in the Values panel, which
	     a layout may not have open — and then there was no way out of editing at all. -->
	<div class="verbs editing" title="Editing cue {editing.number.toFixed(1)} · {editing.name}">
		<span class="what">Editing {editing.number.toFixed(1)} · {editing.name}</span>
		<button class="verb quiet" onclick={() => void cancelEdit()} title="Leave without changing the cue">Cancel</button>
	</div>
{/if}
<div class="verbs">
	<button class="verb" disabled={!anything} onclick={() => openStore()} title="⌘⏎">Store</button>
	<button
		class="verb"
		disabled={!anything || busy}
		onclick={update}
		title={editing ? '⌘U — write the programmer back into the cue being edited' : '⌘U'}>Update</button
	>
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

<!-- EDIT then a control on the wing asks here, beside the Store dialog the wing's
     STORE opens: the top bar is on every page that can claim the wing. -->
<WingEdit />

{#if storing}
	<!-- Keyed, so a second ask while one is open opens on what it asked for. -->
	{#key storing}
		<StoreMenu preselect={storing.preselect} onto={storing.onto ?? null} onclose={() => (storing = null)} />
	{/key}
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
	.verbs.editing {
		border-color: var(--warning, #d9a400);
		align-items: center;
		margin-right: 6px;
	}
	.what {
		padding: 0 10px;
		font-size: var(--font-sm, 12px);
		color: var(--warning, #d9a400);
		white-space: nowrap;
		border-right: 1px solid var(--line-strong, #3a3a3a);
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
