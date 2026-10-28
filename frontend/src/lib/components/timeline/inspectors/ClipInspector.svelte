<script lang="ts">
	/**
	 * A clip: where it sits, what part of its file plays, how loud, and where it goes.
	 *
	 * What the timeline's "Sound" section used to be, one clip at a time. Several clips
	 * at once edit what they can share — level, repetitions and destination — because routing
	 * five scares to the same speakers is one act, not five.
	 *
	 * Edited in place: every field is one value and Ctrl-Z takes it back (task 79's rule).
	 *
	 * A clip goes to a **zone**, never to a node: a node's loudspeaker is a member of a
	 * zone (its Inspector), which is where how it is reached and the room's level are
	 * said once rather than per clip (HHG, 2026-10-28). A clip from before that, still
	 * sent straight to a node, plays as it did and says so here, with a way out.
	 */

	import type { AudioClip, AudioDestination, SpeakerZone } from '$lib/generated/index.js';
	import type { ClipRow } from '$lib/kinds/timeline.kind.js';
	import { categoryColor } from '$lib/layout/categories.js';
	import { audioNodes, bytesText, nodeOf, zoneOf } from '$lib/destination.js';
	import Dialog from '$lib/components/Dialog.svelte';
	import { canEdit } from '$lib/stores/showLock.js';
	import { openSetup } from '$lib/stores/setup.js';
	import { audioOutput, audioStatus, current, devices, drivingReport, nodeAudio, writeColumns } from '$lib/stores/timelines.js';
	import { audioPrefs, loadAudioPrefs, namesFor, playerOf, stationNames, stations, zones } from '$lib/stores/zones.js';
	import { clock, parsePosition, timesOf } from '$lib/waveform.js';
	import { mappingText, widthName } from '$lib/zones.js';
	import { getStationContext } from '$lib/ws/context.js';

	let { rows }: { rows: ClipRow[] } = $props();

	const nodes = $derived(audioNodes($devices, $drivingReport));
	const one = $derived(rows.length === 1 ? rows[0] : null);
	const station = getStationContext();
	loadAudioPrefs();

	/**
	 * The station that plays these clips — the timeline's own, or the leader — since
	 * that is the one whose map decides where a zone comes out. The first row's
	 * timeline stands for all of them: several clips at once are nearly always one
	 * scare's.
	 */
	const player = $derived(playerOf(rows[0] ? current(rows[0].timelineId) : undefined, $stations));
	/** This browser's station plays them, so its own channel names apply. */
	const playedHere = $derived(!!player && player.id === $station?.nodeId);
	const names = $derived(playedHere ? namesFor($audioPrefs, $audioOutput?.device) : undefined);

	/** One write per timeline touched, so editing five clips on one timeline is one row. */
	async function change(edit: (clip: AudioClip) => Partial<AudioClip>) {
		const byTimeline = new Map<string, Set<string>>();
		for (const row of rows) {
			const ids = byTimeline.get(row.timelineId) ?? new Set();
			ids.add(row.id);
			byTimeline.set(row.timelineId, ids);
		}
		for (const [timelineId, ids] of byTimeline) {
			const timeline = current(timelineId);
			if (!timeline) continue;
			await writeColumns(timelineId, {
				clips: timeline.clips.map((c) => (ids.has(c.id) ? { ...c, ...edit(c) } : c))
			});
		}
	}

	const setAll = (patch: Partial<AudioClip>) => change(() => patch);

	/** A destination every selected clip shares, or `null` when they differ. */
	const shared = $derived.by((): AudioDestination | null => {
		const first = JSON.stringify(rows[0]?.clip.destination);
		return rows.every((r) => JSON.stringify(r.clip.destination) === first) ? rows[0].clip.destination : null;
	});

	/** Route every selected clip to a zone. */
	function pickZone(zone: SpeakerZone) {
		void setAll({ destination: { Zone: { zone: zone.id } } });
	}

	/** Whether every selected clip is stored on its nodes, or `null` when they differ. */
	const storedShared = $derived(rows.every((r) => !!r.clip.stored === !!rows[0]?.clip.stored) ? !!rows[0]?.clip.stored : null);
	/**
	 * Storing is asked first: an upload is megabytes written into each node's flash,
	 * which pauses its caches for every sector erased — not something that should start
	 * because somebody clicked to look. Streaming writes nothing and goes straight through.
	 */
	let confirmingStore = $state(false);

	/** The zones a node is a member of, for a clip still sent straight to that node. */
	const zonesWith = (serial: string) => $zones.filter((z) => z.nodes.some((n) => n.serial === serial));

	/**
	 * Where a zone comes out, on every station that maps it — in this station's own
	 * channel names here. A station that does not map it is not listed rather than
	 * warned about: a zone wired on some machines and not others is the ordinary case,
	 * and a clip that cannot sound where it plays says so on its lane.
	 */
	function comesOut(zone: SpeakerZone): string {
		const mapped = $stationNames
			.map((s) => {
				const text = mappingText(zone, s.id, s.id === $station?.nodeId ? names : undefined);
				return text ? `${text} on ${s.name}` : null;
			})
			.filter(Boolean);
		const speakers = zone.nodes.length ? `${zone.nodes.length} node speaker${zone.nodes.length === 1 ? '' : 's'}` : '';
		return [...mapped, speakers].filter(Boolean).join(' + ') || 'nowhere yet';
	}

	const gainShared = $derived(rows.every((r) => r.clip.gain_db === rows[0].clip.gain_db) ? rows[0].clip.gain_db : null);
	const timesShared = $derived(rows.every((r) => timesOf(r.clip) === timesOf(rows[0].clip)) ? timesOf(rows[0].clip) : null);
	const faults = $derived(
		rows.map((r) => ({ name: r.name, fault: $audioStatus?.[r.timelineId]?.clip_faults?.[r.id] })).filter((f) => f.fault)
	);
	/// Sounding, but not everywhere it goes: what failed, by member.
	const partials = $derived(
		rows
			.map((r) => ({ name: r.name, failed: $audioStatus?.[r.timelineId]?.clip_partial?.[r.id] ?? [] }))
			.filter((p) => p.failed.length)
	);
</script>

<div class="fields">
	{#if one}
		{@const clip = one.clip}
		<span class="label">Starts</span>
		<input
			class="input narrow"
			disabled={!$canEdit}
			value={clock(clip.start_ms)}
			onchange={(e) => {
				const ms = parsePosition(e.currentTarget.value);
				if (ms !== null) void setAll({ start_ms: Math.max(0, ms) });
			}}
		/>

		<span class="label">Plays</span>
		<div class="pairs">
			<label><span class="note">from</span>
				<input
					class="input narrow"
					disabled={!$canEdit}
					value={clock(clip.offset_ms)}
					onchange={(e) => {
						const ms = parsePosition(e.currentTarget.value);
						if (ms !== null) void setAll({ offset_ms: Math.max(0, ms) });
					}}
				/></label
			>
			<label><span class="note">for</span>
				<input
					class="input narrow"
					disabled={!$canEdit}
					placeholder="the rest"
					value={clip.length_ms === null ? '' : clock(clip.length_ms)}
					onchange={(e) => {
						const text = e.currentTarget.value.trim();
						const ms = text ? parsePosition(text) : null;
						void setAll({ length_ms: ms && ms > 0 ? ms : null });
					}}
				/></label
			>
		</div>

		<span class="label">Fades, ms</span>
		<div class="pairs">
			<label><span class="note">in</span>
				<input
					class="input narrow"
					type="number"
					min="0"
					disabled={!$canEdit}
					value={clip.fade_in_ms}
					onchange={(e) => void setAll({ fade_in_ms: Math.max(0, Math.round(Number(e.currentTarget.value) || 0)) })}
				/></label
			>
			<label title="Out of the last time round"><span class="note">out</span>
				<input
					class="input narrow"
					type="number"
					min="0"
					disabled={!$canEdit}
					value={clip.fade_out_ms}
					onchange={(e) => void setAll({ fade_out_ms: Math.max(0, Math.round(Number(e.currentTarget.value) || 0)) })}
				/></label
			>
		</div>
	{/if}

	<span class="label">Level</span>
	<div class="inline">
		<input
			class="input narrow"
			type="number"
			step="0.5"
			disabled={!$canEdit}
			placeholder={gainShared === null ? 'mixed' : ''}
			value={gainShared ?? ''}
			onchange={(e) => {
				const db = Number(e.currentTarget.value);
				if (Number.isFinite(db) && e.currentTarget.value !== '') void setAll({ gain_db: Math.min(12, db) });
			}}
		/>
		<span class="note">dB</span>
	</div>

	<!-- How many times it plays, never "for ever": a bed that has to go on is the
	     timeline going round, which takes its cues round with it. -->
	<span class="label">Repetitions</span>
	<div class="inline">
		<input
			class="input narrow"
			type="number"
			min="1"
			step="1"
			disabled={!$canEdit}
			placeholder={timesShared === null ? 'mixed' : ''}
			value={timesShared ?? ''}
			onchange={(e) => {
				const n = Math.floor(Number(e.currentTarget.value));
				if (Number.isFinite(n) && n >= 1) void setAll({ repeats: n });
				else e.currentTarget.value = timesShared === null ? '' : String(timesShared);
			}}
		/>
		<span class="note">{timesShared === 1 ? 'plays once' : timesShared === null ? '' : `plays ${timesShared} times, back to back`}</span>
	</div>

	<span class="label">Goes to</span>
	<div class="targets" role="group" aria-label="Where the clip's sound goes">
		{#if !shared}<span class="note wide-note">several places — pick one for all of them</span>{/if}
		{#each $zones as zone (zone.id)}
			{@const chosen = shared !== null && zoneOf(shared) === zone.id}
			<button
				class="target"
				class:on={chosen}
				disabled={!$canEdit}
				style:--swatch={zone.color ?? categoryColor('playback')}
				title={comesOut(zone)}
				onclick={() => pickZone(zone)}
			>
				<span class="swatch"></span>
				<span class="target-name">{zone.name}</span>
				<span class="target-note">{widthName(zone.width)}</span>
			</button>
		{/each}
		{#if $zones.length === 0}
			<span class="note wide-note">No speaker zones yet. A node's loudspeaker is a member of a zone.</span>
		{/if}
		{#if shared && zoneOf(shared) && !$zones.some((z) => z.id === zoneOf(shared))}
			<span class="says-bad">the zone this went to has been deleted</span>
		{/if}
	</div>

	{#if shared && zoneOf(shared)}
		{@const zone = $zones.find((z) => z.id === zoneOf(shared))}
		{#if zone}
			<span class="label">Comes out of</span>
			<div class="inline note">
				<span>{comesOut(zone)}</span>
				<button class="btn btn-ghost" onclick={() => openSetup('audio')}>Map zones…</button>
			</div>
		{/if}
		{#if zone && zone.nodes.length}
			<span class="label">On nodes</span>
			<div class="segmented" role="group" aria-label="How the clip reaches the node speakers in its zone">
				<button
					class:on={storedShared === false}
					disabled={!$canEdit}
					title="Sent as it plays, a playout delay ahead. No size limit, and it needs the network for as long as it sounds."
					onclick={() => void setAll({ stored: false })}>Streamed</button
				>
				<button
					class:on={storedShared === true}
					disabled={!$canEdit}
					title="Uploaded to each node's flash ahead of time, and started there. Immune to the network once it is there."
					onclick={() => storedShared === true || (confirmingStore = true)}>Stored</button
				>
			</div>
		{/if}
		<span class="label">Output</span>
		{#if $audioOutput?.open}
			<span class="note"><span class="says-good">{names?.alias || $audioOutput.device}</span> · {$audioOutput.channels} channels at {$audioOutput.sample_rate} Hz, on this station</span>
		{:else if $audioOutput?.fault}
			<span class="says-bad">{$audioOutput.fault}</span>
		{:else}
			<span class="note">closed: nothing this station plays has a clip</span>
		{/if}
	{:else if shared && nodeOf(shared)}
		{@const node = nodeOf(shared)!}
		{@const name = nodes.find((n) => n.serial === node.serial)?.name ?? node.serial}
		{@const store = $nodeAudio?.[node.serial]}
		{@const homes = zonesWith(node.serial)}
		<span class="label">Straight to</span>
		<div class="inline note">
			<span class="says-warn"
				>{name}{node.mode === 'Stream' ? ', streamed' : ', stored on the node'} — the way clips went before zones held node speakers</span
			>
		</div>
		<span></span>
		<div class="inline note">
			{#if homes.length}
				<span>{name} is in {homes.map((z) => z.name).join(', ')}: pick {homes.length === 1 ? 'it' : 'one'} above.</span>
			{:else}
				<span>Put {name} in a zone (its Inspector, under Node speakers), then pick the zone above.</span>
			{/if}
		</div>
		{#if store}
			<span class="label">The node</span>
			<span class="note">
				{store.stored.length} clip{store.stored.length === 1 ? '' : 's'} on it, {bytesText(store.bytes_free)} free{store.uploading
					? ' · uploading…'
					: ''}{store.streaming ? ' · streaming' : ''}
			</span>
		{/if}
	{/if}

	{#each faults as f (f.name)}
		<span class="label">Not playing</span>
		<span class="says-bad">{rows.length > 1 ? `${f.name}: ` : ''}{f.fault}</span>
	{/each}
	{#each partials as p (p.name)}
		<span class="label">Partly</span>
		<span class="says-warn">{rows.length > 1 ? `${p.name}: ` : ''}playing, except {p.failed.join('; ')}</span>
	{/each}
</div>

{#if confirmingStore}
	<Dialog title="Store on the nodes" onclose={() => (confirmingStore = false)} onaccept={() => { confirmingStore = false; void setAll({ stored: true }); }}>
		<p class="ask">
			{rows.length === 1 ? `“${rows[0].name}”` : `${rows.length} clips`} will be uploaded to every node speaker in
			{rows.length === 1 ? 'its zone' : 'their zones'} and kept in flash, so it plays there with no network. An upload
			pauses a node's caches while it writes: do it before the show, not during a scare. Changing the trim later uploads
			it again once the clip stands still.
		</p>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (confirmingStore = false)}>Cancel</button>
			<button class="btn btn-primary" onclick={() => { confirmingStore = false; void setAll({ stored: true }); }}>Upload</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.ask {
		margin: 0;
		font-size: 0.85rem;
		line-height: 1.5;
	}
	.pairs {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 6px;
	}
	.pairs label {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}
	.pairs .input {
		width: 100%;
	}
	.targets {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		min-width: 0;
	}
	.wide-note {
		flex-basis: 100%;
	}
	.target {
		display: grid;
		grid-template-columns: 10px auto;
		grid-template-rows: auto auto;
		column-gap: 6px;
		align-items: center;
		min-height: var(--control-h);
		padding: 3px 8px;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.target .swatch {
		grid-row: 1 / 3;
		width: 10px;
		height: 10px;
		border-radius: 50%;
		background: var(--swatch, var(--cat-setup));
	}
	.target-name {
		font-size: var(--font-sm);
	}
	.target-note {
		font-size: var(--font-xs);
		color: var(--text-dim);
	}
	.target.on {
		border-color: var(--accent-solid);
		background: color-mix(in srgb, var(--accent-solid) 18%, var(--bg-sunken));
	}
</style>
