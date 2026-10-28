<script lang="ts">
	/**
	 * Playback: a row of executors, one per sequence — the cue that is up, GO, BACK and
	 * OFF, and the sequence's level on a vertical fader.
	 *
	 * Nothing about fixtures. What a sequence's cues *say* is the Cues panel, and
	 * clicking an executor's name opens it there; what the rig is *doing* is the
	 * Fixtures sheet. An executor that listed every fixture its cue was moving was a
	 * third, worse copy of both, and grew taller with the rig.
	 */

	import { actAt } from '$lib/ws/clock.js';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type { Sequence, Cue } from '$lib/generated/index.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { asOneGesture, beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { sequenceInView } from '$lib/stores/cues.js';
	import { revealPanel } from '$lib/stores/layout.js';
	import Dialog from './Dialog.svelte';
	import NameDialog from './NameDialog.svelte';
	import { offer } from '$lib/stores/learn.js';
	import { collection } from '$lib/stores/show.js';
	import X from '@lucide/svelte/icons/x';
	import { uuid } from '$lib/uuid.js';
	import { byFolder, flatten, isWithin } from '$lib/folders.js';
	import { objectHandle } from '$lib/objectHandle.js';
	import { folders, newFolder } from '$lib/stores/folders.js';

	const data = getDataContext();
	// GO and OFF stay live under the Show lock: they are what a show is run with. What
	// the lock covers is rewriting the cue list while it is being run from.
	const fixtures = collection('fixtures');

	let sequences = $state<Sequence[]>([]);

	// Which folder the bank shows, on this screen: an executor bank scrolls sideways, so
	// folders filter it rather than nesting in it. Subfolders are in their parent. What
	// is shown is grouped by folder along the row, each group under its path.
	const FOLDER_KEY = 'pult.sequences.folder';
	let shownFolder = $state((typeof localStorage !== 'undefined' && localStorage.getItem(FOLDER_KEY)) || '');
	const sequenceFolders = $derived(flatten($folders, 'sequence'));
	const shownSequences = $derived(
		shownFolder && sequenceFolders.some((f) => f.folder.id === shownFolder)
			? sequences.filter((s) => isWithin($folders, s.folder, shownFolder))
			: sequences
	);
	const groups = $derived(byFolder(shownSequences, $folders));
	/** A group's heading, relative to the folder being shown. */
	function heading(path: string): string {
		const shown = sequenceFolders.find((f) => f.folder.id === shownFolder)?.path;
		return shown && path.startsWith(shown) ? path.slice(shown.length).replace(/^ \/ /, '') : path;
	}
	/** Naming a new folder of sequences, inside the one being shown. */
	let namingFolder = $state(false);
	function showFolder(id: string) {
		shownFolder = id;
		try {
			localStorage.setItem(FOLDER_KEY, id);
		} catch {
			// Not remembered in a private window; still shown.
		}
	}
	let cues = $state<Record<string, Cue>>({});
	/** Naming a new sequence, or renaming one. */
	let naming = $state<{ sequence: Sequence | null } | null>(null);

	async function createSequence(name: string) {
		await data.sequences.create({
			id: uuid(),
			name,
			cue_ids: [],
			active_cue_index: null,
			went_at: null,
			master: null,
			// Made while one folder is shown, it belongs there: one not shown would
			// seem to have vanished the moment it was made.
			folder: sequenceFolders.some((f) => f.folder.id === shownFolder) ? shownFolder : null
		});
	}

	async function rename(seq: Sequence, name: string) {
		await data.sequences.byId(seq.id).name.set(name);
	}

	/**
	 * How many parameters the cue that is up is moving, for one quiet line on the
	 * executor — with the list in its tooltip for whoever wants it.
	 */
	function runningOn(seq: Sequence): string[] {
		const cueId = seq.active_cue_index !== null ? seq.cue_ids[seq.active_cue_index] : null;
		if (!cueId) return [];
		const out: string[] = [];
		for (const fixture of $fixtures) {
			for (const [key, effect] of Object.entries(fixture.live_effects ?? {})) {
				// Only what this cue put there. A programmer effect over the top is the
				// operator's, not the cue's, and saying otherwise would be a lie.
				if (effect && typeof effect.source === 'object' && effect.source.Cue === cueId) {
					out.push(`∿ ${fixture.name} · ${key}`);
				}
			}
			for (const [key, fade] of Object.entries(fixture.live_fades ?? {})) {
				if (fade && fade.cue_id === cueId) out.push(`→ ${fixture.name} · ${key}`);
			}
		}
		return out;
	}

	// The time goes with the command: every station runs it from the same arguments,
	// so a Go that carries when it happened anchors the cue's fades and effects at one
	// millisecond everywhere rather than at whenever each station got the message.
	/** Where a fader counts as down. */
	const FADER_BOTTOM = 0.01;

	async function goNext(seqId: string) {
		offer({ kind: 'Press', target: { kind: 'SequenceGo', sequence_id: seqId } });
		await data.sequences.byId(seqId).goNext({ at: actAt() });
	}

	async function goBack(seqId: string) {
		await data.sequences.byId(seqId).goBack({ at: actAt() });
	}

	// Off is a command rather than a write of `active_cue_index`, so it carries its
	// time the way Go does: every station releases what the sequence was driving from
	// the same millisecond, and a rig with a home fade time fades home together.
	async function takeOff(seqId: string) {
		offer({ kind: 'Press', target: { kind: 'SequenceOff', sequence_id: seqId } });
		await data.sequences.byId(seqId).off({ at: actAt() });
	}

	// The sequence's fader. Full is written as no level at all, which is what a show
	// opens at, so a fader pushed back to the top leaves the row as it found it.
	//
	// Up off the bottom takes a sequence that is off, and down onto it takes one that
	// is on off — the wing's faders do the same (`FADER_BOTTOM` in `wing_control`).
	function setMaster(seqId: string, level: number) {
		offer({ kind: 'Master', sequence_id: seqId });
		const seq = sequences.find((s) => s.id === seqId);
		const before = seq?.master ?? 1;
		const on = seq?.active_cue_index != null;
		void data.sequences.byId(seqId).master.set(level >= 1 ? null : level);
		if (before <= FADER_BOTTOM && level > FADER_BOTTOM && !on) void goNext(seqId);
		else if (before > FADER_BOTTOM && level <= FADER_BOTTOM && on) void takeOff(seqId);
	}

	/** Its cues, in the Cues panel: what an executor's cues say is that panel's. */
	function openCues(seqId: string) {
		sequenceInView.set(seqId);
		revealPanel('cues');
	}

	/**
	 * Deleting a sequence takes its cues with it, so it asks first — and does it as
	 * one gesture, so the one Ctrl-Z the question promises is true.
	 */
	let deleting = $state<Sequence | null>(null);
	async function deleteSequence(seqId: string) {
		const seq = sequences.find((s) => s.id === seqId);
		deleting = null;
		if (!seq) return;
		beginGesture();
		try {
			for (const cueId of seq.cue_ids) {
				await data.cues.byId(cueId).delete();
			}
			await data.sequences.byId(seqId).delete();
		} finally {
			endGesture();
		}
	}

	onMount(() => {
		// subscribeDeep auto-fetches initial value, re-fetches full collection on any change,
		// and handles reconnects — no manual fetchAll or addConnectListener needed
		const unsubSeqs = data.sequences.subscribeDeep((seqs) => {
			sequences = seqs;
		});
		const unsubCues = data.cues.subscribeDeep((cueList) => {
			const map: Record<string, Cue> = {};
			for (const c of cueList) map[c.id] = c;
			cues = map;
		});
		return () => {
			unsubSeqs();
			unsubCues();
		};
	});
</script>

<div class="runner">
	<header>
		<h2>Playback</h2>
		{#if sequenceFolders.length > 0}
			<select class="select folder" aria-label="Which folder" value={shownFolder} onchange={(e) => showFolder(e.currentTarget.value)}>
				<option value="">All sequences</option>
				{#each sequenceFolders as f (f.folder.id)}
					<option value={f.folder.id}>{'— '.repeat(f.depth)}{f.folder.name}</option>
				{/each}
			</select>
		{/if}
		<span class="spacer"></span>
		{#if $canEdit}
			<button
				class="btn btn-ghost btn-small"
				title="A folder of sequences. Right-click a sequence, Move to folder…, to put it in one."
				onclick={() => (namingFolder = true)}>+ Folder</button
			>
			<button class="btn btn-ghost btn-small" onclick={() => (naming = { sequence: null })}>+ Sequence</button>
		{/if}
	</header>

	{#if sequences.length === 0}
		<p class="empty">No sequences yet. <b>+ Sequence</b> makes one, and so does storing a cue.</p>
	{/if}

	<div class="executors">
		{#each groups as group (group.path)}
			{@const title = heading(group.path)}
			<section class="group">
				{#if groups.length > 1 || title}
					<h3 title={group.path || 'In no folder'}>{title || group.path.split(' / ').pop() || 'No folder'}</h3>
				{/if}
				<div class="row">
					{#each group.rows as seq (seq.id)}
						{@const activeCue = seq.active_cue_index !== null ? cues[seq.cue_ids[seq.active_cue_index]] : null}
						{@const running = runningOn(seq)}
						{@const level = seq.master ?? 1}
						<div class="exec" class:live={seq.active_cue_index !== null}>
							<!-- Right-click or hold for the menu every object has — Move to folder… among it. -->
							<div class="top" use:objectHandle={{ kind: 'sequence', id: seq.id }}>
								<button
									class="name"
									title="Open its cues in the Cues panel"
									onclick={() => openCues(seq.id)}
									ondblclick={() => $canEdit && (naming = { sequence: seq })}>{seq.name}</button
								>
								{#if seq.merge === 'Htp'}<span class="htp" title="HTP: its intensities show where they are higher">HTP</span>{/if}
								{#if $canEdit}
									<button class="delete" title="Delete {seq.name}" aria-label="Delete {seq.name}" onclick={() => (deleting = seq)}>
										<X size={12} />
									</button>
								{/if}
							</div>

							<div class="cue" title={activeCue ? `${activeCue.number.toFixed(1)} · ${activeCue.name}` : 'Off'}>
								{#if activeCue}
									<span class="num">{activeCue.number.toFixed(1)}</span>
									<span class="cue-name">{activeCue.name}</span>
								{:else}
									<span class="off">Off</span>
								{/if}
							</div>
							<div class="meta" title={running.join('\n')}>
								{seq.cue_ids.length} {seq.cue_ids.length === 1 ? 'cue' : 'cues'}{running.length > 0 ? ` · ${running.length} running` : ''}
							</div>

							<!-- The sequence's level, upright like a playback fader: what it dims is
							     the intensities it is playing, and nothing the programmer or a take
							     is holding. -->
							<div class="fader" title="This sequence's level: it dims the intensities it plays">
								<input
									type="range"
									min="0"
									max="1"
									step="0.005"
									value={level}
									aria-label="{seq.name} level"
									use:asOneGesture
									oninput={(e) => setMaster(seq.id, Number(e.currentTarget.value))}
								/>
								<span class="level mono">{Math.round(level * 100)}%</span>
							</div>

							<div class="keys">
								<button
									class="go"
									onclick={() => goNext(seq.id)}
									disabled={seq.cue_ids.length === 0}
									title={seq.cue_ids.length === 0 ? 'No cues in this sequence' : 'Go to the next cue'}
								>GO</button>
								<div class="pair">
									<button
										onclick={() => goBack(seq.id)}
										disabled={seq.active_cue_index === null || seq.active_cue_index === 0}
										title="Back one cue"
									>BACK</button>
									<button
										onclick={() => takeOff(seq.id)}
										disabled={seq.active_cue_index === null}
										title="Take it off: what it was driving goes back to where it rests"
									>OFF</button>
								</div>
							</div>
						</div>
					{/each}
				</div>
			</section>
		{/each}
	</div>
</div>

{#if namingFolder}
	<NameDialog
		title="New folder of sequences"
		placeholder="Folder name…"
		initial="New folder"
		confirm="Create"
		onsubmit={(name) => newFolder('sequence', sequenceFolders.some((f) => f.folder.id === shownFolder) ? shownFolder : null, name)}
		onclose={() => (namingFolder = false)}
	/>
{/if}

{#if naming}
	<NameDialog
		title={naming.sequence ? 'Rename a sequence' : 'New sequence'}
		placeholder="Sequence name…"
		initial={naming.sequence?.name ?? `Sequence ${sequences.length + 1}`}
		confirm={naming.sequence ? 'Rename' : 'Create'}
		onsubmit={(name) => (naming?.sequence ? rename(naming.sequence, name) : createSequence(name))}
		onclose={() => (naming = null)}
	/>
{/if}

{#if deleting}
	<Dialog title="Delete a sequence" onclose={() => (deleting = null)}>
		<p class="ask">
			<b>{deleting.name}</b>
			{deleting.cue_ids.length === 0
				? 'is deleted. It has no cues.'
				: `and its ${deleting.cue_ids.length} ${deleting.cue_ids.length === 1 ? 'cue are' : 'cues are'} deleted.`}
			Ctrl-Z brings {deleting.cue_ids.length === 0 ? 'it' : 'them'} back.
		</p>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (deleting = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={() => deleting && deleteSequence(deleting.id)}>Delete</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.htp {
		font-size: 9px;
		font-weight: 600;
		padding: 0 4px;
		border-radius: 3px;
		border: 1px solid var(--cat-playback, #4a8cff);
		color: var(--cat-playback, #4a8cff);
		align-self: center;
	}
	.runner {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		padding: 10px 12px;
		gap: 10px;
	}
	header {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	h2 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	.spacer {
		flex: 1;
	}
	.btn-small {
		min-height: 26px;
		padding: 0 10px;
		font-size: 12px;
	}
	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
	}

	/* A row of executors that scrolls sideways, the way a desk's executors run left to
	   right, rather than a grid of cards that reflows when the panel is resized. */
	.executors {
		flex: 1;
		min-height: 0;
		display: flex;
		gap: 8px;
		overflow-x: auto;
		overflow-y: hidden;
		padding-bottom: 4px;
	}
	/* A folder's sequences, side by side under its name; folders follow one another
	   along the same sideways row, with a rule between them. */
	.group {
		flex: none;
		display: flex;
		flex-direction: column;
		gap: 4px;
		min-height: 0;
	}
	.group + .group {
		padding-left: 8px;
		border-left: 1px solid var(--line-strong);
	}
	.group h3 {
		font-size: var(--font-xs);
		font-weight: 600;
		color: var(--text-dim);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		max-width: 100%;
	}
	.row {
		flex: 1;
		min-height: 0;
		display: flex;
		gap: 8px;
	}
	.exec {
		flex: none;
		width: 128px;
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 8px;
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		background: var(--bg-panel);
		min-height: 0;
	}
	/* A sequence with a cue up is lit, in the amber of everything reaching the rig. */
	.exec.live {
		border-color: color-mix(in srgb, var(--live) 60%, var(--line-strong));
		box-shadow: inset 0 2px 0 var(--live);
	}

	.top {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.name {
		flex: 1;
		min-width: 0;
		padding: 0;
		border: none;
		background: none;
		color: var(--text-bright);
		font: inherit;
		font-size: var(--font-sm);
		font-weight: 600;
		text-align: left;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		cursor: pointer;
	}
	.name:hover {
		color: var(--accent);
	}
	.delete {
		display: inline-flex;
		padding: 2px;
		border: none;
		background: none;
		color: var(--text-faint);
		cursor: pointer;
	}
	.delete:hover {
		color: var(--bad);
	}

	.cue {
		display: flex;
		align-items: baseline;
		gap: 6px;
		min-width: 0;
		font-size: var(--font-sm);
	}
	.num {
		color: var(--live);
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}
	.cue-name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.off {
		color: var(--text-faint);
	}
	.meta {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}

	.fader {
		flex: 1;
		min-height: 90px;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 4px;
	}
	.fader input {
		flex: 1;
		min-height: 0;
		writing-mode: vertical-lr;
		direction: rtl;
		width: 28px;
		accent-color: var(--live);
		cursor: pointer;
	}
	.level {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.mono {
		font-variant-numeric: tabular-nums;
	}

	.keys {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.keys button {
		min-height: 30px;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		background: var(--bg-raised);
		color: var(--text);
		font: inherit;
		font-size: var(--font-xs);
		font-weight: 600;
		letter-spacing: 0.06em;
		cursor: pointer;
	}
	.keys button:hover:not(:disabled) {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.keys button:disabled {
		color: var(--text-faint);
		cursor: default;
	}
	.keys .go {
		min-height: 40px;
		border-color: color-mix(in srgb, var(--good) 60%, transparent);
		color: var(--good);
		font-size: var(--font-sm);
	}
	.keys .go:hover:not(:disabled) {
		background: color-mix(in srgb, var(--good) 15%, transparent);
	}
	.pair {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 4px;
	}

	.ask {
		margin: 0;
		font-size: 0.85rem;
		line-height: 1.5;
	}
</style>
