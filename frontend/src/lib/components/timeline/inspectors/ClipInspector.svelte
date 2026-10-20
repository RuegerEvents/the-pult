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

	import type { AudioClip, AudioDestination } from '$lib/generated/index.js';
	import type { ClipRow } from '$lib/kinds/timeline.kind.js';
	import { audioNodes, bytesText, nodeOf, stationChannels } from '$lib/destination.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { audioOutput, audioStatus, current, devices, nodeAudio, writeColumns } from '$lib/stores/timelines.js';
	import { channelsText, clock, parseChannels, parsePosition } from '$lib/waveform.js';

	let { rows }: { rows: ClipRow[] } = $props();

	const nodes = $derived(audioNodes($devices));
	const one = $derived(rows.length === 1 ? rows[0] : null);
	let problem = $state<string | null>(null);

	/**
	 * How many channels there are to offer: the device this station has open, or —
	 * where it has none, or this browser is on a station that is not the one playing —
	 * sixteen, which is a number to type past rather than a limit.
	 */
	const channelCount = $derived($audioOutput?.open && $audioOutput.channels > 0 ? $audioOutput.channels : 16);

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

	function toggleChannel(channel: number) {
		void change((clip) => {
			const now = stationChannels(clip.destination) ?? [];
			const next = now.includes(channel) ? now.filter((c) => c !== channel) : [...now, channel].sort((a, b) => a - b);
			return { destination: { Station: { channels: next } } };
		});
	}

	function pickTarget(serial: string) {
		void change((clip) => ({
			destination: serial
				? { Node: { serial, mode: nodeOf(clip.destination)?.mode ?? 'Clip' } }
				: { Station: { channels: stationChannels(clip.destination) ?? [1, 2] } }
		}));
	}

	const gainShared = $derived(rows.every((r) => r.clip.gain_db === rows[0].clip.gain_db) ? rows[0].clip.gain_db : null);
	const loopShared = $derived(rows.every((r) => r.clip.looped === rows[0].clip.looped) ? rows[0].clip.looped : null);
	const faults = $derived(
		rows.map((r) => ({ name: r.name, fault: $audioStatus?.[r.timelineId]?.clip_faults?.[r.id] })).filter((f) => f.fault)
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
	<select
		class="select"
		disabled={!$canEdit}
		value={shared ? (nodeOf(shared)?.serial ?? '') : '?'}
		onchange={(e) => pickTarget(e.currentTarget.value)}
	>
		{#if !shared}<option value="?" disabled>several places</option>{/if}
		<option value="">this station's channels</option>
		{#each nodes as node (node.serial)}
			<option value={node.serial}>{node.name} — {node.outputs}{node.online ? '' : ' (offline)'}</option>
		{/each}
		{#if shared && nodeOf(shared) && !nodes.some((n) => n.serial === nodeOf(shared)?.serial)}
			<option value={nodeOf(shared)?.serial}>{nodeOf(shared)?.serial} (not found)</option>
		{/if}
	</select>

	{#if shared && stationChannels(shared)}
		{@const channels = stationChannels(shared) ?? []}
		<span class="label">Channels</span>
		<div class="inline">
			<input
				class="input narrow"
				disabled={!$canEdit}
				title="Output channels, counting from 1: 5, 6 or 1-4"
				value={channelsText(channels)}
				onchange={(e) => {
					const parsed = parseChannels(e.currentTarget.value);
					if (parsed) {
						problem = null;
						void setAll({ destination: { Station: { channels: parsed } } });
					} else problem = 'Channels count from 1, written like 5, 6 or 1-4.';
				}}
			/>
			<div class="channels">
				{#each Array.from({ length: Math.min(64, channelCount) }, (_, i) => i + 1) as channel (channel)}
					<button
						class="channel"
						class:on={channels.includes(channel)}
						disabled={!$canEdit}
						title="Channel {channel}"
						onclick={() => toggleChannel(channel)}>{channel}</button
					>
				{/each}
			</div>
		</div>
		<span class="label">Output</span>
		{#if $audioOutput?.open}
			<span class="note"><span class="says-good">{$audioOutput.device}</span> · {$audioOutput.channels} channels at {$audioOutput.sample_rate} Hz, on this station</span>
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
				onclick={() => void setAll({ destination: { Node: { serial: node.serial, mode: 'Clip' } } })}>Stored on the node</button
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
	{#if problem}<p class="wide says-bad">{problem}</p>{/if}
</div>

<style>
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
	.channels {
		display: flex;
		flex-wrap: wrap;
		gap: 2px;
	}
	.channel {
		width: 28px;
		height: 24px;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: 3px;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 0;
		cursor: pointer;
	}
	.channel.on {
		background: var(--accent-solid);
		border-color: var(--accent-solid);
		color: var(--text-bright);
	}
</style>
