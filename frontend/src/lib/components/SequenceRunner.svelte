<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type { Sequence, Cue } from '$lib/generated/index.js';
	import { createCue } from '$lib/cues.js';
	import { beginEdit, editingCue } from '$lib/stores/programmer.js';

	const data = getDataContext();

	let sequences = $state<Sequence[]>([]);
	let cues = $state<Record<string, Cue>>({});
	let expanded = $state<Record<string, boolean>>({});
	let newSeqName = $state('');
	let creatingSeq = $state(false);
	let addingCueTo = $state<string | null>(null);
	let newCueName = $state('');
	let editingSeqId = $state<string | null>(null);
	let editingSeqName = $state('');
	let editingCueId = $state<string | null>(null);
	let editingCueName = $state('');

	async function createSequence() {
		if (!newSeqName.trim()) return;
		await data.sequences.create({ id: crypto.randomUUID(), name: newSeqName.trim(), cue_ids: [], active_cue_index: null, went_at: null });
		newSeqName = '';
		creatingSeq = false;
	}

	async function addCue(seqId: string) {
		if (!newCueName.trim()) return;
		const seq = sequences.find((s) => s.id === seqId);
		if (!seq) return;
		await createCue(data, seq, { name: newCueName.trim() });
		newCueName = '';
		addingCueTo = null;
	}

	// The time goes with the command: every station runs it from the same arguments,
	// so a Go that carries when it happened anchors the cue's fades and effects at one
	// millisecond everywhere rather than at whenever each station got the message.
	async function goNext(seqId: string) {
		await data.sequences.byId(seqId).goNext({ at: Date.now() });
	}

	async function goToCue(seqId: string, cueId: string) {
		await data.sequences.byId(seqId).goToCue({ cueId, at: Date.now() });
	}

	async function resetSequence(seqId: string) {
		await data.sequences.byId(seqId).active_cue_index.set(null);
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

	async function deleteCue(seqId: string, cueId: string) {
		const seq = sequences.find((s) => s.id === seqId);
		if (!seq) return;
		await data.sequences.byId(seqId).cue_ids.set(seq.cue_ids.filter((id) => id !== cueId));
		await data.cues.byId(cueId).delete();
	}

	async function saveCueName(cueId: string) {
		const trimmed = editingCueName.trim();
		if (trimmed) await data.cues.byId(cueId).name.set(trimmed);
		editingCueId = null;
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
		{#if !creatingSeq}
			<button class="new-btn" onclick={() => (creatingSeq = true)}>+ New</button>
		{/if}
	</div>

	{#if creatingSeq}
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
					{:else}
						<span
							class="seq-name"
							title="Click to rename"
							role="button"
							tabindex="0"
							onclick={() => { editingSeqId = seq.id; editingSeqName = seq.name; }}
							onkeydown={(e) => { if (e.key === 'Enter') { editingSeqId = seq.id; editingSeqName = seq.name; } }}
						>{seq.name}</span>
					{/if}
					<div class="seq-header-right">
						<span class="seq-id dim mono">{seq.id.slice(0, 6)}</span>
						<button
							class="icon-btn delete-btn"
							title="Delete sequence"
							onclick={() => deleteSequence(seq.id)}
						>✕</button>
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
						onclick={() => resetSequence(seq.id)}
						disabled={seq.active_cue_index === null}
						title="Reset to beginning"
					>
						↺
					</button>
					<button
						class="expand-btn"
						onclick={() => (expanded[seq.id] = !expanded[seq.id])}
						title="Toggle cue list"
					>
						{expanded[seq.id] ? '▲' : '▼'} Cues ({seq.cue_ids.length})
					</button>
				</div>

				<!-- Cue list -->
				{#if expanded[seq.id]}
					<div class="cue-list">
						{#each seq.cue_ids as cueId, i (cueId)}
							{@const cue = cues[cueId]}
							{@const isActive = seq.active_cue_index === i}
							{#if cue}
								<div class="cue-row" class:active={isActive} class:editing={$editingCue === cueId}>
									<button
										class="cue-go-area"
										onclick={() => goToCue(seq.id, cueId)}
										title="Jump to this cue"
									>
										<span class="cue-num mono">{cue.number.toFixed(1)}</span>
										{#if editingCueId === cueId}
											<form
												class="inline-edit"
												onsubmit={(e) => { e.preventDefault(); saveCueName(cueId); }}
											>
												<input
													class="text-input inline-name-input"
													bind:value={editingCueName}
													use:focusOnMount
													onclick={(e) => e.stopPropagation()}
													onblur={() => saveCueName(cueId)}
													onkeydown={(e) => { if (e.key === 'Escape') editingCueId = null; e.stopPropagation(); }}
												/>
											</form>
										{:else}
											<span
												class="cue-row-name"
												title="Click to rename"
												role="button"
												tabindex="0"
												onclick={(e) => { e.stopPropagation(); editingCueId = cueId; editingCueName = cue.name; }}
												onkeydown={(e) => { if (e.key === 'Enter') { editingCueId = cueId; editingCueName = cue.name; } }}
											>{cue.name}</span>
										{/if}
										<span class="captures dim mono" title="Parameters this cue stores">
											{cue.captures.length}
										</span>
										{#if isActive}
											<span class="active-dot">●</span>
										{/if}
									</button>
									<button
										class="icon-btn edit-btn"
										class:on={$editingCue === cueId}
										title="Load this cue into the programmer to change it"
										onclick={() => beginEdit(cue, seq)}
									>Edit</button>
									<button
										class="icon-btn delete-btn"
										title="Delete cue"
										onclick={() => deleteCue(seq.id, cueId)}
									>✕</button>
								</div>
							{/if}
						{/each}

						<!-- Add cue -->
						{#if addingCueTo === seq.id}
							<form
								class="add-cue-form"
								onsubmit={(e) => {
									e.preventDefault();
									addCue(seq.id);
								}}
							>
								<input
									class="text-input sm"
									placeholder="Cue name…"
									bind:value={newCueName}
									use:focusOnMount
									onkeydown={(e) => {
										if (e.key === 'Escape') {
											addingCueTo = null;
											newCueName = '';
										}
									}}
								/>
								<button class="confirm-btn sm" type="submit">Add</button>
								<button
									class="cancel-btn sm"
									type="button"
									onclick={() => {
										addingCueTo = null;
										newCueName = '';
									}}>✕</button
								>
							</form>
						{:else}
							<button
								class="add-cue-btn"
								onclick={() => {
									addingCueTo = seq.id;
									newCueName = '';
								}}
							>
								+ Add Cue
							</button>
						{/if}
					</div>
				{/if}
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

	.new-seq-form,
	.add-cue-form {
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
	.text-input.sm {
		font-size: 0.78rem;
		padding: 3px 6px;
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
	.confirm-btn.sm {
		font-size: 0.72rem;
		padding: 3px 8px;
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
	.cancel-btn.sm {
		font-size: 0.72rem;
		padding: 3px 8px;
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

	.expand-btn {
		font-size: 0.72rem;
		padding: 4px 8px;
		border-radius: 4px;
		border: 1px solid #3a3a3a;
		background: transparent;
		color: #666;
		cursor: pointer;
		margin-left: auto;
	}
	.expand-btn:hover {
		border-color: #555;
		color: #aaa;
	}

	.cue-list {
		border-top: 1px solid #2e2e2e;
		padding: 6px;
	}

	.cue-row {
		display: flex;
		align-items: center;
		border-radius: 3px;
		color: #aaa;
		transition: background 0.1s;
	}
	.cue-row:hover {
		background: #2a2a2a;
		color: #e0e0e0;
	}
	.cue-row.active {
		background: #f59e0b18;
		color: #f0d090;
	}
	/* Outlined rather than filled: a cue can be the one playing and the one being
	   edited at the same time, and both have to stay readable. */
	.cue-row.editing {
		outline: 1px solid #4a9eff88;
		outline-offset: -1px;
	}
	.cue-row:hover .icon-btn {
		opacity: 1;
	}
	.cue-row .icon-btn {
		opacity: 0;
		padding: 4px 6px;
		transition: opacity 0.1s;
	}

	.cue-go-area {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 5px 8px;
		border: none;
		background: transparent;
		flex: 1;
		min-width: 0;
		text-align: left;
		cursor: pointer;
		color: inherit;
	}

	.cue-num {
		font-size: 0.72rem;
		width: 28px;
		color: #666;
		flex-shrink: 0;
	}
	.cue-row.active .cue-num {
		color: #f59e0b;
	}

	.cue-row-name {
		font-size: 0.82rem;
		flex: 1;
		min-width: 0;
		cursor: pointer;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.active-dot {
		font-size: 0.6rem;
		color: #f59e0b;
		flex-shrink: 0;
	}

	.captures {
		font-size: 0.68rem;
		flex-shrink: 0;
	}

	.edit-btn {
		font-size: 0.68rem;
		padding: 2px 6px;
	}
	.edit-btn:hover,
	.edit-btn.on {
		border-color: #4a9eff;
		color: #4a9eff;
	}
	.cue-row .edit-btn.on {
		opacity: 1;
	}

	.add-cue-btn {
		display: block;
		width: 100%;
		padding: 5px 8px;
		text-align: left;
		font-size: 0.75rem;
		color: #555;
		background: transparent;
		border: 1px dashed #333;
		border-radius: 3px;
		cursor: pointer;
		margin-top: 4px;
		transition: all 0.15s;
	}
	.add-cue-btn:hover {
		color: #4a9eff;
		border-color: #4a9eff44;
	}
</style>
