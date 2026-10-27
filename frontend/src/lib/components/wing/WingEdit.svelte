<script lang="ts">
	/**
	 * EDIT and then a control on the wing: what is on it, to edit.
	 *
	 * Three different things live on a playback key, and an operator who presses EDIT
	 * on one means one of them: the cue it is on (loaded into the programmer, UPDATE
	 * writes it back), the sequence's own settings, or what the key does at all. The
	 * station works out which cue — a Go-to-cue key's own, else the sequence's active
	 * cue, else the one its Go takes next — and this asks which of the three.
	 *
	 * A control with no sequence on it has only the last, so it opens straight there.
	 */

	import { untrack } from 'svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import SequenceInspector from '$lib/components/inspectors/SequenceInspector.svelte';
	import { beginEdit } from '$lib/stores/programmer.js';
	import { collection, showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { controlName, learned, wingEditAsk } from '$lib/stores/wing.js';
	import { addToast } from '$lib/toasts.js';
	import BindingEditor from './BindingEditor.svelte';

	const sequences = collection('sequences');
	const cues = collection('cues');

	const ask = $derived($wingEditAsk);
	const sequence = $derived(ask?.sequenceId ? ($sequences.find((s) => s.id === ask.sequenceId) ?? null) : null);
	const cue = $derived(ask?.cueId ? ($cues.find((c) => c.id === ask.cueId) ?? null) : null);

	let step = $state<'choose' | 'sequence' | 'binding'>('choose');

	// Each ask starts over, and one with no sequence on it has only one thing to edit.
	$effect(() => {
		const a = $wingEditAsk;
		if (!a) return;
		untrack(() => {
			if (a.sequenceId) step = 'choose';
			else whatItDoes();
		});
	});

	function close() {
		if (step === 'binding') learned.set(null);
		wingEditAsk.set(null);
	}

	/**
	 * What the control does, through the same editor LEARN opens — and as a learned
	 * control, so touching a parameter, a sequence's level or a tile binds it the way
	 * learning does. That binding closes this too.
	 */
	function whatItDoes() {
		if (!ask) return;
		learned.set(ask.control);
		step = 'binding';
	}
	$effect(() => {
		if (step === 'binding' && $learned === null) untrack(() => wingEditAsk.set(null));
	});

	async function editCue() {
		if (!cue) return;
		try {
			await beginEdit(cue, sequence);
			addToast(`Editing cue ${cue.number.toFixed(1)} · ${cue.name} — UPDATE writes it back`, 'success');
		} catch (e) {
			addToast(e instanceof Error ? e.message : String(e));
		}
		close();
	}

	function rename(value: string) {
		const name = value.trim();
		if (sequence && name && name !== sequence.name) void showData().sequences.byId(sequence.id).name.set(name);
	}
</script>

{#if ask}
	<Dialog title="Edit {controlName(ask.control)}" size={step === 'binding' ? 'wide' : 'small'} onclose={close}>
		<div class="pad">
		{#if step === 'choose' && sequence}
			<div class="choices">
				<button class="choice" disabled={!cue || !$canEdit} onclick={() => void editCue()}>
					<span class="what">The cue</span>
					<span class="which">
						{cue ? `${cue.number.toFixed(1)} · ${cue.name}` : `${sequence.name} has no cues`}
					</span>
					{#if cue}<span class="dim">Loaded into the programmer and taken; UPDATE writes it back.</span>{/if}
				</button>
				<button class="choice" onclick={() => (step = 'sequence')}>
					<span class="what">The sequence</span>
					<span class="which">{sequence.name}</span>
					<span class="dim">Its name, how it merges, whether it goes around.</span>
				</button>
				<button class="choice" disabled={!$canEdit} onclick={whatItDoes}>
					<span class="what">What it does</span>
					<span class="which">{controlName(ask.control)}</span>
					<span class="dim">Go, Off, Flash, Go to cue — or something else entirely.</span>
				</button>
			</div>
		{:else if step === 'sequence' && sequence}
			<div class="sequence">
				<label class="field">
					<span class="dim">Name</span>
					<input
						class="input"
						value={sequence.name}
						disabled={!$canEdit}
						onchange={(e) => rename(e.currentTarget.value)}
						onkeydown={(e) => e.key === 'Enter' && rename(e.currentTarget.value)}
					/>
				</label>
				<SequenceInspector rows={[sequence]} />
			</div>
		{:else if step === 'binding'}
			<BindingEditor control={ask.control} onclose={close} />
		{/if}
		</div>
	</Dialog>
{/if}

<style>
	.pad {
		padding: 14px 16px;
	}
	.choices {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.choice {
		display: grid;
		grid-template-columns: 7.5em 1fr;
		gap: 2px 10px;
		text-align: left;
		padding: 10px 12px;
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: 6px;
		background: none;
		color: var(--text, #ddd);
		font: inherit;
		cursor: pointer;
	}
	.choice:hover:not(:disabled) {
		background: var(--bg-hover, #2a2a2a);
	}
	.choice:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.what {
		font-weight: 600;
	}
	.choice .dim {
		grid-column: 2;
		font-size: var(--font-sm, 12px);
	}
	.dim {
		color: var(--text-dim, #888);
	}
	.sequence {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.field {
		display: flex;
		align-items: center;
		gap: 10px;
	}
	.field .input {
		flex: 1;
	}
</style>
