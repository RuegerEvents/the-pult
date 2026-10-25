<script lang="ts">
	/**
	 * A timeline's own settings: how it ends, who plays it, where its position comes
	 * from, the tempo it drives and its beat grid.
	 *
	 * Everything that used to be stacked under the waveform as sections, and none of it
	 * about one thing on a lane — so it is what the Inspector shows when the timeline
	 * itself is the subject, and what the timeline's docked inspector shows when nothing
	 * on it is selected.
	 *
	 * **The detector proposes; this confirms.** *Detect beats* draws its answer over the
	 * lanes in its own color and changes nothing; *Use this grid* is the write. A console
	 * that silently rewrote the grid of a show somebody had already programmed against
	 * would be worse than one that could not detect anything.
	 *
	 * What it records is not here: that is the transport's *Recording* menu, where it is
	 * shown while it happens, and so is a grab. And what a setting means is behind its
	 * `?` rather than written under it, the rule every panel follows now.
	 */

	import type { AtEnd, LtcRate, Timeline } from '$lib/generated/index.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { collection, showClient, showData } from '$lib/stores/show.js';
	import { audioStatus, now, positionOf, proposals } from '$lib/stores/timelines.js';
	import { gridFrom } from '$lib/timeline/grid.js';
	import { subject } from '$lib/objects.js';
	import { clock, parsePosition } from '$lib/waveform.js';
	import Help from '../../Help.svelte';

	let { rows }: { rows: Timeline[] } = $props();

	const stations = collection('stations');
	const masters = collection('speed_masters');

	let problem = $state<string | null>(null);
	let busy = $state<string | null>(null);
	let modelState = $state<{ present: boolean; fetching: boolean; bytes: number } | null>(null);

	const proxy = (id: string) => showData().timelines.byId(id);

	async function askAboutTheModel(fetchIt: boolean) {
		try {
			modelState = (await showClient().call('timeline.model', { full: fetchIt })) as typeof modelState;
		} catch (e) {
			problem = String(e);
		}
	}
	$effect(() => {
		void askAboutTheModel(false);
	});

	/** The clip to listen to: the one selected on this timeline, or its first. */
	function clipFor(timeline: Timeline): string | null {
		const picked = $subject.find((r) => r.kind === 'timeline-clip' && timeline.clips.some((c) => c.id === r.id));
		return picked?.id ?? timeline.clips[0]?.id ?? null;
	}

	/** Find the beats. The station does the work; this waits for the answer. */
	async function detect(timeline: Timeline) {
		problem = null;
		busy = 'Listening to the whole song…';
		try {
			const found = (await showClient().call('timeline.detect', {
				timelineId: timeline.id,
				clipId: clipFor(timeline)
			})) as import('$lib/generated/index.js').Detected;
			if (found.beats_ms.length === 0) problem = 'Nothing in that file reads as a pulse.';
			else proposals.update((all) => ({ ...all, [timeline.id]: found }));
		} catch (e) {
			problem = String(e);
		} finally {
			busy = null;
		}
	}

	async function useProposal(timeline: Timeline) {
		const found = $proposals[timeline.id];
		if (!found) return;
		const segments = gridFrom(found);
		if (segments.length === 0) {
			problem = 'There is not enough of a pulse there to make a grid from.';
			return;
		}
		await proxy(timeline.id).grid.set(segments);
		discard(timeline);
	}

	function discard(timeline: Timeline) {
		proposals.update((all) => {
			const next = { ...all };
			delete next[timeline.id];
			return next;
		});
	}
</script>

{#each rows as timeline (timeline.id)}
	{@const sound = $audioStatus?.[timeline.id] ?? null}
	{@const proposal = $proposals[timeline.id]}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{timeline.name}</span>{/if}

		<span class="label">Ends</span>
		<div class="inline">
			<input
				class="input narrow"
				placeholder="never"
				disabled={!$canEdit}
				value={timeline.end ? clock(timeline.end.at_ms) : ''}
				onchange={(e) => {
					const text = e.currentTarget.value.trim();
					const ms = text ? parsePosition(text) : null;
					void proxy(timeline.id).end.set(ms ? { at_ms: ms, then: timeline.end?.then ?? 'Stop' } : null);
				}}
			/>
			{#if timeline.end}
				{@const end = timeline.end}
				<div class="segmented" role="group" aria-label="At the end">
					{#each [['Stop', 'and stops'], ['Loop', 'and goes round']] as [then, label] (then)}
						<button
							class:on={end.then === then}
							disabled={!$canEdit}
							onclick={() => void proxy(timeline.id).end.set({ ...end, then: then as AtEnd })}>{label}</button
						>
					{/each}
				</div>
			{:else}
				<span class="note">runs until somebody stops it</span>
			{/if}
		</div>

		<span class="label">Played by</span>
		<div class="inline">
			<select
				class="select"
				disabled={!$canEdit}
				value={timeline.node_id ?? ''}
				onchange={(e) => void proxy(timeline.id).node_id.set(e.currentTarget.value || null)}
			>
				<option value="">the leader</option>
				{#each $stations as station (station.id)}
					<option value={station.id}>{station.hostname}</option>
				{/each}
			</select>
			{#if sound}
				{#if sound.playing}
					<span class="says-good">playing through {sound.device ?? 'a device'}</span>
					<span class="note">{sound.drift_ms} ms from the anchor</span>
				{:else if sound.fault}
					<span class="says-bad">{sound.fault}</span>
				{/if}
			{:else if timeline.clips.length > 0}
				<span class="note">nothing on this station is playing it</span>
			{/if}
		</div>

		<span class="label">Position from</span>
		<div class="inline">
			<select
				class="select"
				disabled={!$canEdit}
				value={typeof timeline.source === 'string' ? 'Internal' : 'Ltc'}
				onchange={(e) =>
					void proxy(timeline.id).source.set(e.currentTarget.value === 'Ltc' ? { Ltc: { fps: 'F25', offset_frames: 0 } } : 'Internal')}
			>
				<option value="Internal">this console</option>
				<option value="Ltc">timecode (LTC)</option>
			</select>
			{#if typeof timeline.source !== 'string'}
				{@const ltc = timeline.source.Ltc}
				<select
					class="select"
					disabled={!$canEdit}
					value={ltc.fps}
					onchange={(e) => void proxy(timeline.id).source.set({ Ltc: { ...ltc, fps: e.currentTarget.value as LtcRate } })}
				>
					<option value="F24">24 fps</option>
					<option value="F25">25 fps</option>
					<option value="F30">30 fps</option>
					<option value="F2997">29.97 non-drop</option>
					<option value="F2997Df">29.97 drop-frame</option>
				</select>
				<input
					class="input narrow"
					disabled={!$canEdit}
					title="Offset in frames, subtracted from what arrives"
					value={ltc.offset_frames}
					onchange={(e) => {
						const frames = Number(e.currentTarget.value);
						if (Number.isFinite(frames)) void proxy(timeline.id).source.set({ Ltc: { ...ltc, offset_frames: Math.round(frames) } });
					}}
				/>
				<span class="note">frames offset</span>
			{/if}
		</div>
		{#if sound?.ltc}
			<span class="label">Timecode</span>
			<div class="inline">
				{#if sound.ltc.lock === 'Locked'}
					<span class="says-good">locked</span>
				{:else if sound.ltc.lock === 'Lost'}
					<span class="says-bad">LTC lost</span>
				{:else}
					<span class="note">waiting for timecode</span>
				{/if}
				{#if sound.ltc.position_ms !== null && sound.ltc.position_ms !== undefined}<span class="note">{clock(sound.ltc.position_ms)}</span>{/if}
				{#if sound.ltc.device}<span class="note">{sound.ltc.device}</span>{/if}
				{#if sound.ltc.fault}<span class="says-bad">{sound.ltc.fault}</span>{/if}
			</div>
		{/if}

		<span class="label">Drives <Help label="What a timeline drives">While it runs, the speed master is set to the tempo of each grid segment it crosses, with the downbeat put where the segment's "one" fell.</Help></span>
		<div class="inline">
			<select
				class="select"
				disabled={!$canEdit}
				value={timeline.speed_master ?? ''}
				onchange={(e) => void proxy(timeline.id).speed_master.set(e.currentTarget.value || null)}
			>
				<option value="">no speed master</option>
				{#each $masters as master (master.id)}
					<option value={master.id}>{master.name}</option>
				{/each}
			</select>
		</div>

		<span class="label"
			>Beat grid <Help label="How beats are found">
				<i>Detect beats</i> listens to the selected clip, or the first, and draws what it hears over the
				lanes. Nothing changes until you press <i>Use this grid</i>.
				{#if modelState?.present}It is using the full-accuracy model on this station.
				{:else if modelState?.fetching}The full-accuracy model is downloading; until it lands, the small one is used.
				{:else}It is using the small model this console ships with.
					<button class="link" onclick={() => askAboutTheModel(true)}>Fetch the full one (83 MB)</button>
				{/if}
			</Help></span
		>
		<div class="inline">
			{#if proposal}
				<span class="note">{proposal.beats_ms.length} beats, {proposal.downbeats_ms.length} bars proposed</span>
				{#if $canEdit}<button class="btn btn-primary" onclick={() => useProposal(timeline)}>Use this grid</button>{/if}
				<button class="btn" onclick={() => discard(timeline)}>Discard</button>
			{:else}
				<span class="note">{timeline.grid.length === 0 ? 'none' : `${timeline.grid.length} segment${timeline.grid.length === 1 ? '' : 's'}`}</span>
				<button class="btn" onclick={() => detect(timeline)} disabled={timeline.clips.length === 0 || busy !== null}>Detect beats</button>
				{#if $canEdit}
					<button
						class="btn btn-ghost"
						onclick={() =>
							void proxy(timeline.id).grid.set(
								[...timeline.grid, { at_ms: Math.round(positionOf(timeline, $now) ?? 0), bpm: 120, beats_per_bar: 4 }].sort(
									(a, b) => a.at_ms - b.at_ms
								)
							)}>+ Tempo at the playhead</button
					>
				{/if}
			{/if}
		</div>
		{#if busy}<p class="wide note">{busy}</p>{/if}

		{#if problem}<p class="wide says-bad">{problem}</p>{/if}
	</div>
{/each}

<style>
	.link {
		background: none;
		border: none;
		color: var(--accent);
		font: inherit;
		padding: 0;
		cursor: pointer;
		text-decoration: underline;
	}
</style>
