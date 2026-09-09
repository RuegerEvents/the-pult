<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type { Sequence, Cue } from '$lib/generated/index.js';
	import { editing } from '$lib/stores/editing.js';
	import { collection } from '$lib/stores/show.js';

	const data = getDataContext();
	// GO and reset stay live: they are what a show is run with. What the lock covers
	// is rewriting the cue list while it is being run from.
	const unlocked = editing('playback');
	/** For the running strip: what each station is actually rendering. */
	const fixtures = collection('fixtures');

	let sequences = $state<Sequence[]>([]);
	let cues = $state<Record<string, Cue>>({});
	let newSeqName = $state('');
	let creatingSeq = $state(false);
	let editingSeqId = $state<string | null>(null);
	let editingSeqName = $state('');

	async function createSequence() {
		if (!newSeqName.trim()) return;
		await data.sequences.create({ id: crypto.randomUUID(), name: newSeqName.trim(), cue_ids: [], active_cue_index: null, went_at: null });
		newSeqName = '';
		creatingSeq = false;
	}

	/** What is moving on the cue that is up, for the strip under it. */
	function runningOn(seq: Sequence): { label: string; kind: 'effect' | 'fade' }[] {
		const cueId = seq.active_cue_index !== null ? seq.cue_ids[seq.active_cue_index] : null;
		if (!cueId) return [];
		const out: { label: string; kind: 'effect' | 'fade' }[] = [];
		for (const fixture of $fixtures) {
			for (const [key, effect] of Object.entries(fixture.live_effects ?? {})) {
				// Only what this cue put there. A programmer effect over the top is the
				// operator's, not the cue's, and saying otherwise would be a lie.
				if (effect && typeof effect.source === 'object' && effect.source.Cue === cueId) {
					out.push({ label: `${fixture.name} · ${key}`, kind: 'effect' });
				}
			}
			for (const [key, fade] of Object.entries(fixture.live_fades ?? {})) {
				if (fade && fade.cue_id === cueId) {
					out.push({ label: `${fixture.name} · ${key}`, kind: 'fade' });
				}
			}
		}
		return out;
	}

	// The time goes with the command: every station runs it from the same arguments,
	// so a Go that carries when it happened anchors the cue's fades and effects at one
	// millisecond everywhere rather than at whenever each station got the message.
	async function goNext(seqId: string) {
		await data.sequences.byId(seqId).goNext({ at: Date.now() });
	}

	// Off is a command rather than a write of `active_cue_index`, so it carries its
	// time the way Go does: every station releases what the sequence was driving from
	// the same millisecond, and a rig with a home fade time fades home together.
	async function takeOff(seqId: string) {
		await data.sequences.byId(seqId).off({ at: Date.now() });
	}

	async function deleteSequence(seqId: string) {
		const seq = sequences.find((s) => s.id === seqId);
		if (!seq) return;
		for (const cueId of seq.cue_ids) {
			await data.cues.byId(cueId).delete();
		}
		await data.sequences.byId(seqId).delete();
	}

	async function saveSeqName(seqId: string) {
		const trimmed = editingSeqName.trim();
		if (trimmed) await data.sequences.byId(seqId).name.set(trimmed);
		editingSeqId = null;
	}

	onMount(() => {
		// subscribeDeep auto-fetches initial value, re-fetches full collection on any change,
		// and handles reconnects — no manual fetchAll or addConnectListener needed
		const unsubSeqs = data.sequences.subscribeDeep(seqs => { sequences = seqs; });
		const unsubCues = data.cues.subscribeDeep(cueList => {
			const map: Record<string, Cue> = {};
			for (const c of cueList) map[c.id] = c;
			cues = map;
		});
		return () => { unsubSeqs(); unsubCues(); };
	});
</script>

<div class="runner">
	<div class="runner-header">
		<h2 class="section-title">Sequences</h2>
		{#if !creatingSeq && $unlocked}
			<button class="new-btn" onclick={() => (creatingSeq = true)}>+ New</button>
		{/if}
	</div>

	{#if creatingSeq && $unlocked}
		<form
			class="new-seq-form"
			onsubmit={(e) => {
				e.preventDefault();
				createSequence();
			}}
		>
			<input
				class="text-input"
				placeholder="Sequence name…"
				bind:value={newSeqName}
				use:focusOnMount
				onkeydown={(e) => {
					if (e.key === 'Escape') {
						creatingSeq = false;
						newSeqName = '';
					}
				}}
			/>
			<button class="confirm-btn" type="submit">Create</button>
			<button
				class="cancel-btn"
				type="button"
				onclick={() => {
					creatingSeq = false;
					newSeqName = '';
				}}>Cancel</button
			>
		</form>
	{/if}

	{#if sequences.length === 0 && !creatingSeq}
		<p class="empty-hint">No sequences yet — create one above.</p>
	{/if}

	<div class="seq-grid">
		{#each sequences as seq (seq.id)}
			{@const activeCue =
				seq.active_cue_index !== null ? cues[seq.cue_ids[seq.active_cue_index]] : null}
			<div class="seq-card" class:has-active={seq.active_cue_index !== null}>
				<!-- Header row -->
				<div class="seq-header">
					{#if editingSeqId === seq.id}
						<form
							class="inline-edit"
							onsubmit={(e) => { e.preventDefault(); saveSeqName(seq.id); }}
						>
							<input
								class="text-input inline-name-input"
								bind:value={editingSeqName}
								use:focusOnMount
								onblur={() => saveSeqName(seq.id)}
								onkeydown={(e) => { if (e.key === 'Escape') editingSeqId = null; }}
							/>
						</form>
					{:else if $unlocked}
						<span
							class="seq-name"
							title="Click to rename"
							role="button"
							tabindex="0"
							onclick={() => { editingSeqId = seq.id; editingSeqName = seq.name; }}
							onkeydown={(e) => { if (e.key === 'Enter') { editingSeqId = seq.id; editingSeqName = seq.name; } }}
						>{seq.name}</span>
					{:else}
						<span class="seq-name">{seq.name}</span>
					{/if}
					<div class="seq-header-right">
						<span class="seq-id dim mono">{seq.id.slice(0, 6)}</span>
						{#if $unlocked}
							<button
								class="icon-btn delete-btn"
								title="Delete sequence"
								onclick={() => deleteSequence(seq.id)}
							>✕</button>
						{/if}
					</div>
				</div>

				<!-- Active cue indicator -->
				<div class="active-cue-bar">
					{#if activeCue}
						<span class="cue-number">CUE {activeCue.number.toFixed(0)}</span>
						<span class="cue-name">{activeCue.name}</span>
					{:else}
						<span class="cue-idle">— no active cue —</span>
					{/if}
				</div>

				<!-- What the cue that is up is actually doing. A cue list says what was
				     asked for; this says what is happening, which during a three second
				     fade or a running chase is a different thing. -->
				{#if runningOn(seq).length > 0}
					<div class="running">
						{#each runningOn(seq) as item (item.label + item.kind)}
							<span class="run-chip" class:fade={item.kind === 'fade'}>
								{item.kind === 'effect' ? '∿' : '→'} {item.label}
							</span>
						{/each}
					</div>
				{/if}

				<!-- Controls -->
				<div class="controls">
					<button
						class="go-btn"
						onclick={() => goNext(seq.id)}
						disabled={seq.cue_ids.length === 0}
						title={seq.cue_ids.length === 0 ? 'No cues in sequence' : 'Advance to next cue'}
					>
						GO
					</button>
					<button
						class="reset-btn"
						onclick={() => takeOff(seq.id)}
						disabled={seq.active_cue_index === null}
						title="Take it off: what it was driving goes back to where it rests"
					>
						OFF
					</button>
					<!-- The cue list itself is the `cues` panel now: this is the runner, and
					     an expander inside it was a second, worse cue sheet. What is left is
					     the count, so a card still says how long the sequence is. -->
					<span class="cue-count">{seq.cue_ids.length} cues</span>
				</div>

			</div>
		{/each}
	</div>
</div>

<style>
	.runner {
		padding: 16px;
	}

	.runner-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 14px;
	}

	.section-title {
		font-size: 0.68rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #777;
	}

	.new-btn {
		font-size: 0.78rem;
		padding: 4px 12px;
		border-radius: 4px;
		border: 1px solid #4a9eff;
		background: transparent;
		color: #4a9eff;
		cursor: pointer;
		transition: background 0.15s;
	}
	.new-btn:hover {
		background: #4a9eff22;
	}

	.new-seq-form {
		display: flex;
		gap: 6px;
		align-items: center;
		margin-bottom: 12px;
	}

	.text-input {
		background: #1a1a1a;
		border: 1px solid #555;
		border-radius: 4px;
		color: #e0e0e0;
		font-size: 0.85rem;
		padding: 4px 8px;
		flex: 1;
		min-width: 0;
	}
	.text-input:focus {
		outline: none;
		border-color: #4a9eff;
	}

	.confirm-btn {
		font-size: 0.78rem;
		padding: 4px 10px;
		border-radius: 4px;
		border: 1px solid #22c55e;
		background: transparent;
		color: #22c55e;
		cursor: pointer;
	}
	.confirm-btn:hover {
		background: #22c55e22;
	}

	.cancel-btn {
		font-size: 0.78rem;
		padding: 4px 10px;
		border-radius: 4px;
		border: 1px solid #555;
		background: transparent;
		color: #888;
		cursor: pointer;
	}
	.cancel-btn:hover {
		border-color: #ef4444;
		color: #ef4444;
	}

	.empty-hint {
		font-size: 0.82rem;
		color: #555;
		font-style: italic;
	}

	.seq-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
		gap: 12px;
	}

	.seq-card {
		background: #222;
		border: 1px solid #333;
		border-radius: 6px;
		overflow: hidden;
		transition: border-color 0.2s;
	}
	.seq-card.has-active {
		border-color: #f59e0b55;
	}

	.seq-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 10px 12px 6px;
		gap: 6px;
	}

	.seq-header-right {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-shrink: 0;
	}

	.seq-name {
		font-size: 0.9rem;
		font-weight: 500;
		color: #e0e0e0;
		cursor: pointer;
		flex: 1;
		min-width: 0;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.seq-name:hover {
		color: #fff;
	}

	.inline-edit {
		flex: 1;
		min-width: 0;
	}

	.inline-name-input {
		width: 100%;
		font-size: inherit;
		font-weight: inherit;
	}

	.icon-btn {
		font-size: 0.65rem;
		padding: 2px 5px;
		border-radius: 3px;
		border: 1px solid transparent;
		background: transparent;
		color: #555;
		cursor: pointer;
		line-height: 1;
		flex-shrink: 0;
	}
	.delete-btn:hover {
		border-color: #ef4444;
		color: #ef4444;
	}

	.dim {
		color: #555;
	}
	.mono {
		font-family: monospace;
	}
	.seq-id {
		font-size: 0.7rem;
	}

	.active-cue-bar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 12px;
		background: #1a1a1a;
		min-height: 32px;
	}

	.cue-number {
		font-family: monospace;
		font-size: 0.72rem;
		font-weight: 700;
		color: #f59e0b;
		letter-spacing: 0.05em;
	}

	.cue-name {
		font-size: 0.82rem;
		color: #d0d0d0;
	}

	.cue-idle {
		font-size: 0.78rem;
		color: #444;
		font-style: italic;
	}

	.cue-count {
		margin-left: auto;
		font-size: 0.72rem;
		color: #555;
	}

	.controls {
		display: flex;
		gap: 6px;
		padding: 8px 12px;
		align-items: center;
	}

	.go-btn {
		font-size: 0.85rem;
		font-weight: 700;
		letter-spacing: 0.1em;
		padding: 6px 20px;
		border-radius: 4px;
		border: 2px solid #f59e0b;
		background: transparent;
		color: #f59e0b;
		cursor: pointer;
		transition: all 0.1s;
	}
	.go-btn:hover:not(:disabled) {
		background: #f59e0b22;
	}
	.go-btn:active:not(:disabled) {
		background: #f59e0b44;
		transform: scale(0.97);
	}
	.go-btn:disabled {
		border-color: #444;
		color: #444;
		cursor: not-allowed;
	}

	.reset-btn {
		font-size: 0.9rem;
		padding: 5px 8px;
		border-radius: 4px;
		border: 1px solid #444;
		background: transparent;
		color: #888;
		cursor: pointer;
	}
	.reset-btn:hover:not(:disabled) {
		border-color: #888;
		color: #ccc;
	}
	.reset-btn:disabled {
		color: #444;
		cursor: not-allowed;
	}
	/* What the cue that is up is actually doing, as opposed to what it asked for. */
	.running {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		padding: 6px 10px 0;
	}
	.run-chip {
		font-size: 10px;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--live);
		color: var(--live);
		white-space: nowrap;
	}
	/* A fade is on its way somewhere and will stop; an effect will not. Worth
	   telling apart at a glance when a cue is half in. */
	.run-chip.fade {
		border-color: var(--accent);
		color: var(--accent);
	}</style>
