<script lang="ts">
	/**
	 * A clip: where it sits, what part of its file plays, how loud, and where it goes.
	 *
	 * What the timeline's "Sound" section used to be, one clip at a time. Several clips
	 * at once edit what they can share — level, loop and destination — because routing
	 * five scares to the same speakers is one act, not five.
	 *
	 * Edited in place: every field is one value and Ctrl-Z takes it back (task 79's rule).
	 */

	import type { AudioClip, AudioDestination, SpeakerZone } from '$lib/generated/index.js';
	import type { ClipRow } from '$lib/kinds/timeline.kind.js';
	import { categoryColor } from '$lib/layout/categories.js';
	import { audioNodes, bytesText, nodeOf, zoneOf } from '$lib/destination.js';
	import Dialog from '$lib/components/Dialog.svelte';
	import { canEdit } from '$lib/stores/showLock.js';
	import { openSetup } from '$lib/stores/setup.js';
	import { audioOutput, audioStatus, current, devices, nodeAudio, writeColumns } from '$lib/stores/timelines.js';
	import { audioPrefs, loadAudioPrefs, namesFor, playerOf, stations, zones } from '$lib/stores/zones.js';
	import { clock, parsePosition } from '$lib/waveform.js';
	import { channelName, mappingOn, widthName, zoneProblem } from '$lib/zones.js';
	import { getStationContext } from '$lib/ws/context.js';

	let { rows }: { rows: ClipRow[] } = $props();

	const nodes = $derived(audioNodes($devices));
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

	/** Route every selected clip to a zone, or to one node (keeping how it reaches it). */
	function pickZone(zone: SpeakerZone) {
		void setAll({ destination: { Zone: { zone: zone.id } } });
	}
	/**
	 * Sending a clip to a node's flash is asked first: an upload is megabytes written
	 * into the node's flash, which pauses its caches for every sector erased, and is
	 * not something that should start because somebody clicked a node to look at it.
	 * Streaming writes nothing and goes straight through — and is what picking a node
	 * gives a clip that was not on one, so storing is only ever the deliberate choice.
	 */
	let confirmingUpload = $state<{ serial: string; name: string } | null>(null);
	function pickNode(serial: string) {
		// Moving a clip that is already stored on one node to another is a new upload,
		// and asks; anything else streams.
		const storing = rows.some((r) => nodeOf(r.clip.destination)?.mode === 'Clip');
		if (storing) {
			confirmingUpload = { serial, name: nodes.find((n) => n.serial === serial)?.name ?? serial };
			return;
		}
		void change(() => ({ destination: { Node: { serial, mode: 'Stream' } } }));
	}
	function confirmUpload() {
		const ask = confirmingUpload;
		confirmingUpload = null;
		if (!ask) return;
		void setAll({ destination: { Node: { serial: ask.serial, mode: 'Clip' } } });
	}

	/** Where a zone comes out on the player, in its own channel names where they are known. */
	function onPlayer(zone: SpeakerZone): string {
		if (!player) return 'no station is playing this';
		const problem = zoneProblem(zone, player.id, player.hostname);
		if (problem) return problem;
		const channels = mappingOn(zone, player.id);
		const here = channels ? channels.map((c) => channelName(names, c)).join(', ') : '';
		const speakers = zone.nodes.length ? `${zone.nodes.length} node speaker${zone.nodes.length === 1 ? '' : 's'}` : '';
		return [here && `${here} on ${player.hostname}`, speakers].filter(Boolean).join(' + ');
	}

	const gainShared = $derived(rows.every((r) => r.clip.gain_db === rows[0].clip.gain_db) ? rows[0].clip.gain_db : null);
	const loopShared = $derived(rows.every((r) => r.clip.looped === rows[0].clip.looped) ? rows[0].clip.looped : null);
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
			<label title={clip.looped ? 'A clip that goes round never fades out' : ''}><span class="note">out</span>
				<input
					class="input narrow"
					type="number"
					min="0"
					disabled={!$canEdit || clip.looped}
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
		<label class="inline note">
			<input
				type="checkbox"
				disabled={!$canEdit}
				checked={loopShared === true}
				indeterminate={loopShared === null}
				onchange={(e) => void setAll({ looped: e.currentTarget.checked })}
			/>
			goes round
		</label>
	</div>

	<span class="label">Goes to</span>
	<div class="targets" role="group" aria-label="Where the clip's sound goes">
		{#if !shared}<span class="note wide-note">several places — pick one for all of them</span>{/if}
		{#each $zones as zone (zone.id)}
			{@const chosen = shared !== null && zoneOf(shared) === zone.id}
			{@const problem = player ? zoneProblem(zone, player.id, player.hostname) : null}
			<button
				class="target"
				class:on={chosen}
				class:warn={problem !== null}
				disabled={!$canEdit}
				style:--swatch={zone.color ?? categoryColor('playback')}
				title={onPlayer(zone)}
				onclick={() => pickZone(zone)}
			>
				<span class="swatch"></span>
				<span class="target-name">{zone.name}</span>
				<span class="target-note">{problem ?? widthName(zone.width)}</span>
			</button>
		{/each}
		{#each nodes as node (node.serial)}
			{@const chosen = shared !== null && nodeOf(shared)?.serial === node.serial}
			<button class="target node" class:on={chosen} disabled={!$canEdit} onclick={() => pickNode(node.serial)}>
				<span class="swatch"></span>
				<span class="target-name">{node.name}</span>
				<span class="target-note">{node.online ? node.outputs : 'offline'}</span>
			</button>
		{/each}
		{#if shared && nodeOf(shared) && !nodes.some((n) => n.serial === nodeOf(shared)?.serial)}
			<span class="says-bad">node {nodeOf(shared)?.serial} is not found</span>
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
				<span class:says-bad={!!(player && zoneProblem(zone, player.id, player.hostname))}>{onPlayer(zone)}</span>
				<button class="btn btn-ghost" onclick={() => openSetup('audio')}>Map zones…</button>
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
		{@const store = $nodeAudio?.[node.serial]}
		{@const status = $devices?.discovered[node.serial]?.audio_status}
		<span class="label">How</span>
		<div class="segmented" role="group" aria-label="How the clip reaches the node">
			<button
				class:on={node.mode === 'Clip'}
				disabled={!$canEdit}
				title="Uploaded to the node's flash ahead of time, and started there. Immune to the network once it is there."
				onclick={() =>
					node.mode === 'Clip' ||
					(confirmingUpload = { serial: node.serial, name: nodes.find((n) => n.serial === node.serial)?.name ?? node.serial })}
				>Stored on the node</button
			>
			<button
				class:on={node.mode === 'Stream'}
				disabled={!$canEdit}
				title="Sent as it plays, a playout delay ahead. No size limit, and it needs the network for as long as it sounds."
				onclick={() => void setAll({ destination: { Node: { serial: node.serial, mode: 'Stream' } } })}>Streamed</button
			>
		</div>
		{#if store || status}
			<span class="label">The node</span>
			<div class="inline note">
				{#if store}
					<span>
						{store.stored.length} clip{store.stored.length === 1 ? '' : 's'} on it, {bytesText(store.bytes_free)} free{store.uploading
							? ' · uploading…'
							: ''}{store.pending && !store.uploading ? ` · ${store.pending} waiting` : ''}{store.streaming ? ' · streaming' : ''}
					</span>
				{/if}
				{#if status}
					{#if !status.locked}
						<span class="says-bad">has not got the console's clock</span>
					{:else if status.underruns > 0}
						<span class="says-bad">{status.underruns} underruns</span>
					{:else}
						<span class="says-good"
							>{status.voices.length} voice{status.voices.length === 1 ? '' : 's'} sounding{status.stream?.active
								? `, stream ${status.stream.depth_ms} ms buffered`
								: ''}</span
						>
					{/if}
				{/if}
			</div>
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

{#if confirmingUpload}
	<Dialog title="Store on the node" onclose={() => (confirmingUpload = null)}>
		<p class="ask">
			{rows.length === 1 ? `“${rows[0].name}”` : `${rows.length} clips`} will be uploaded to
			<b>{confirmingUpload.name}</b> and kept in its flash, so it plays there with no network.
			An upload pauses the node's caches while it writes; changing a clip's trim later uploads it
			again once the clip stands still.
		</p>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (confirmingUpload = null)}>Cancel</button>
			<button class="btn btn-primary" onclick={confirmUpload}>Upload</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.ask {
		margin: 0;
		padding: 14px;
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
	.target.node .swatch {
		border-radius: 2px;
		background: var(--text-dim);
	}
	.target-name {
		font-size: var(--font-sm);
	}
	.target-note {
		font-size: var(--font-xs);
		color: var(--text-dim);
	}
	.target.warn .target-note {
		color: var(--live);
	}
	.target.on {
		border-color: var(--accent-solid);
		background: color-mix(in srgb, var(--accent-solid) 18%, var(--bg-sunken));
	}
</style>
