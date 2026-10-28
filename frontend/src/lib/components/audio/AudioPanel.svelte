<script lang="ts">
	/**
	 * Audio: this station's sound card, what its channels are called, and the show's
	 * speaker zones mapped onto them.
	 *
	 * Two kinds of fact on one page, and the page says which is which. The **card** and
	 * its **names** are this machine's — `[audio]` in its preferences — because which
	 * speaker channel 4 is depends on how this Mac is wired, and the spare beside it is
	 * wired differently. The **zones** are the show's and travel with it; the **map**
	 * from a zone to channels is written on the zone, per station, so each station
	 * maps the same zones to its own card and every station can see who has not.
	 *
	 * An OpenHaunt node's loudspeaker is two more columns of the same matrix, after the
	 * stations: a zone is routed onto a node's left and right the way it is onto a card's
	 * channels, and a node is not the station's, so it is not under one.
	 *
	 * An errand rather than a panel somebody keeps open: it is done at the venue, once
	 * per machine, and then a clip is routed to "Area 2" and never to a number.
	 */

	import { onMount } from 'svelte';

	import Card from '$lib/components/objects/Card.svelte';
	import Foldered from '$lib/components/objects/Foldered.svelte';
	import { focusOnMount } from '$lib/actions.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { showData } from '$lib/stores/show.js';
	import { audioOutput, devices as deviceList, drivingReport } from '$lib/stores/timelines.js';
	import { audioNodes } from '$lib/destination.js';
	import { audioPrefs, createZone, forgetStation, loadAudioPrefs, nameDevice, namesFor, stationAudio, stationNames, zones } from '$lib/stores/zones.js';
	import { writePreferences } from '$lib/preferences.js';
	import { getClientContext, getStationContext } from '$lib/ws/context.js';
	import {
		NODE_CHANNELS,
		channelName,
		freshZoneName,
		nodeSidesAt,
		sideName,
		sidesAt,
		sidesOf,
		toggleChannel,
		toggleNodeChannel,
		widthName,
		type DeviceNames
	} from '$lib/zones.js';
	import { isSubject, selectSubject, subject, subjectRows } from '$lib/objects.js';
	import InspectorPanel from '$lib/components/inspector/InspectorPanel.svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';

	const client = getClientContext();
	const station = getStationContext();
	loadAudioPrefs();

	type AudioDevice = { name: string; id: string; is_default: boolean; channels: number; sample_rate: number };
	let devices = $state<{ outputs: AudioDevice[]; inputs: AudioDevice[] } | null>(null);
	let trouble = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let newWidth = $state(1);

	const here = $derived($station?.nodeId ?? null);

	/** The card this station plays through: the one it names, or the system default. */
	const card = $derived.by((): AudioDevice | null => {
		const outputs = devices?.outputs ?? [];
		const named = $audioPrefs?.output;
		return (named ? outputs.find((d) => d.name === named) : outputs.find((d) => d.is_default)) ?? null;
	});
	/** Its name as the preferences key it: the device's own. */
	const cardName = $derived(card?.name ?? $audioOutput?.device ?? null);
	const names = $derived(namesFor($audioPrefs, cardName));
	/** How many channels the card has, capped where a matrix stops being readable. */
	const channelCount = $derived(Math.min(64, card?.channels ?? ($audioOutput?.channels || 2)));
	/**
	 * The columns: the card's channels, and past them any channel a zone is mapped to
	 * here that the card has not got — drawn, and marked, rather than left off, because a
	 * mapping nobody can see is a mapping nobody can take back.
	 */
	const channels = $derived.by(() => {
		const mapped = here ? $zones.flatMap((z) => sidesOf(z, here).flat()) : [];
		const last = Math.min(64, Math.max(channelCount, ...mapped));
		return Array.from({ length: last }, (_, i) => i + 1);
	});

	// ── The matrix: every station's channels side by side ─────────────────────────

	/**
	 * One block of columns per station in the session, the way Dante Controller lays a
	 * network out: a zone is a row, and where it comes out on each machine is read along
	 * it, so "is Area 2 mapped on the spare" is answered without choosing whose card to
	 * look at. A station's channel names are its own preference and are shown, not
	 * edited, here except for this station's: one console rewriting another machine's
	 * settings file is what `LogRaise` refuses too, and the mapping is all that has to be
	 * done from a distance.
	 */
	type Block = {
		id: string;
		label: string;
		count: number;
		names: DeviceNames | undefined;
		channels: number[];
		told: boolean;
		online: boolean;
	};
	const blocks = $derived.by((): Block[] => {
		const ids = $stationNames.length > 0 ? $stationNames : here ? [{ id: here, name: 'this station', online: true }] : [];
		const list = ids.map((s): Block => {
			const home = s.id === here;
			const row = $stationAudio.find((r) => r.id === s.id) ?? null;
			const count = home ? channelCount : Math.min(64, row?.channels || 2);
			const mapped = $zones.flatMap((z) => sidesOf(z, s.id).flat());
			const last = Math.min(64, Math.max(count, ...mapped));
			return {
				id: s.id,
				label: `${stationLabel(s)}${home ? ' (here)' : ''}`,
				count,
				names: home ? names : row ? { alias: row.alias, channels: row.channel_names } : undefined,
				channels: Array.from({ length: last }, (_, i) => i + 1),
				told: home || row !== null,
				online: s.online
			};
		});
		// This station first: it is the one whose names can be edited from here.
		return list.sort((x, y) => (x.id === here ? -1 : y.id === here ? 1 : 0));
	});
	/**
	 * A station as the matrix names it: its hostname, and its card's alias beside it
	 * where two machines share a hostname — two MacBooks both called "MacBook-Pro" are
	 * the ordinary case, and a header of two identical words is no header.
	 */
	const stationLabel = (s: { id: string; name: string }) => {
		const shared = $stationNames.filter((o) => o.name === s.name).length > 1;
		const alias = $stationAudio.find((r) => r.id === s.id)?.alias;
		if (!shared) return alias ? `${s.name} · ${alias}` : s.name;
		return `${s.name} · ${alias || s.id.slice(0, 8)}`;
	};

	// ── Node loudspeakers: two columns each, after the stations ───────────────────

	/**
	 * Every adopted node with a loudspeaker, from the station driving them, and any node
	 * a zone still names that is not found — drawn, for the reason a channel past the
	 * card is: a routing nobody can see is one nobody can take back.
	 */
	const nodeBlocks = $derived.by(() => {
		const found = audioNodes($deviceList, $drivingReport);
		const named = [...new Set($zones.flatMap((z) => z.nodes.map((n) => n.serial)))].filter(
			(serial) => !found.some((n) => n.serial === serial)
		);
		return [
			...found.map((n) => ({ serial: n.serial, label: n.name, online: n.online, found: true })),
			...named.map((serial) => ({ serial, label: serial, online: false, found: false }))
		];
	});
	const nodeChannels = Array.from({ length: NODE_CHANNELS }, (_, i) => i + 1);
	const nodeOutputName = (channel: number) => (channel === 1 ? 'L' : channel === 2 ? 'R' : String(channel));

	function toggleNode(zoneId: string, serial: string, channel: number) {
		const zone = $zones.find((z) => z.id === zoneId);
		if (!zone) return;
		void showData().speaker_zones.byId(zone.id).nodes.set(toggleNodeChannel(zone, serial, channel, 'Stream'));
	}


	/**
	 * Forgetting a station the show remembers asks first, and says what goes: every
	 * zone's mapping on it. One gesture, so one Ctrl-Z brings all of it back.
	 */
	let forgetting = $state<Block | null>(null);
	const mappedOn = (id: string) => $zones.filter((z) => id in z.channels);
	async function forget(block: Block) {
		forgetting = null;
		beginGesture();
		try {
			await forgetStation(block.id);
		} finally {
			endGesture();
		}
	}

	/**
	 * The dock beside the matrix shows zones and nothing else. A zone picked up here by
	 * its card's dot or a ⌘-click is *added* to the subject, which still held the clip
	 * somebody was editing on the timeline — so on this page anything that is not a
	 * zone is let go of as soon as a zone is picked up.
	 */
	$effect(() => {
		const refs = $subject;
		if (refs.some((r) => r.kind === 'speaker-zone') && refs.some((r) => r.kind !== 'speaker-zone')) {
			subject.set(refs.filter((r) => r.kind === 'speaker-zone'));
		}
	});

	/** The zones that are the Inspector's subject, docked beside the matrix. */
	const inspecting = $derived($subjectRows.some((g) => g.kind.kind === 'speaker-zone'));

	onMount(() => {
		// Asked once rather than watched: enumerating a machine's devices blocks.
		client
			.call('audio.devices', {})
			.then((found) => (devices = found as { outputs: AudioDevice[]; inputs: AudioDevice[] }))
			.catch(() => {});
	});

	async function setDevice(which: 'output' | 'input', name: string) {
		trouble = null;
		const stored = await writePreferences({ audio: { ...($audioPrefs ?? {}), [which]: name || null } });
		if (!stored) trouble = 'This console could not write its settings down.';
		else audioPrefs.set(stored.audio);
	}

	async function rename(change: { alias?: string | null; channel?: number; name?: string }) {
		if (!cardName) return;
		trouble = null;
		const now = names ?? {};
		const list = [...(now.channels ?? [])];
		if (change.channel !== undefined) {
			while (list.length < change.channel) list.push('');
			list[change.channel - 1] = (change.name ?? '').trim();
			while (list.length > 0 && list[list.length - 1] === '') list.pop();
		}
		const alias = change.alias !== undefined ? change.alias?.trim() || null : (now.alias ?? null);
		if (!(await nameDevice(cardName, { alias, channels: list }))) {
			trouble = 'This console could not write its settings down.';
		}
	}

	async function addZone() {
		const name = newName.trim() || freshZoneName($zones);
		const id = await createZone(name, Math.max(1, newWidth));
		creating = false;
		newName = '';
		newWidth = 1;
		selectSubject({ kind: 'speaker-zone', id });
	}

	function toggle(zoneId: string, station: string, channel: number) {
		const zone = $zones.find((z) => z.id === zoneId);
		if (!zone) return;
		void showData().speaker_zones.byId(zone.id).channels.set(toggleChannel(zone, station, channel));
	}

	const groups = (list: AudioDevice[]) =>
		[
			{ label: 'This machine uses this by default', devices: list.filter((d) => d.is_default) },
			{ label: 'Everything else', devices: list.filter((d) => !d.is_default) }
		].filter((g) => g.devices.length > 0);
</script>

<div class="audio-page">
<div class="audio">
	<section>
		<h3>This station's sound card <span class="whose">this machine</span></h3>
		<div class="fields">
			<label class="label" for="audio-out">Plays through</label>
			<select
				id="audio-out"
				class="select"
				disabled={!$canEdit}
				value={$audioPrefs?.output ?? ''}
				onchange={(e) => setDevice('output', e.currentTarget.value)}
			>
				<option value="">The system default</option>
				{#if $audioPrefs?.output && !(devices?.outputs ?? []).some((d) => d.name === $audioPrefs?.output)}
					<!-- Named, and not on this machine: shown as itself, never as the default. -->
					<option value={$audioPrefs.output}>{$audioPrefs.output} — not on this machine</option>
				{/if}
				<!-- Grouped and never filtered, the rule the interface picker follows. -->
				{#each groups(devices?.outputs ?? []) as group (group.label)}
					<optgroup label={group.label}>
						{#each group.devices as device (device.id)}
							<option value={device.name}>{namesFor($audioPrefs, device.name)?.alias || device.name} — {device.channels} ch, {device.sample_rate} Hz</option>
						{/each}
					</optgroup>
				{/each}
			</select>

			<label class="label" for="audio-in">Hears timecode on</label>
			<select
				id="audio-in"
				class="select"
				disabled={!$canEdit}
				value={$audioPrefs?.input ?? ''}
				onchange={(e) => setDevice('input', e.currentTarget.value)}
			>
				<option value="">The system default</option>
				{#if $audioPrefs?.input && !(devices?.inputs ?? []).some((d) => d.name === $audioPrefs?.input)}
					<option value={$audioPrefs.input}>{$audioPrefs.input} — not on this machine</option>
				{/if}
				{#each groups(devices?.inputs ?? []) as group (group.label)}
					<optgroup label={group.label}>
						{#each group.devices as device (device.id)}
							<option value={device.name}>{device.name} — {device.channels} ch, {device.sample_rate} Hz</option>
						{/each}
					</optgroup>
				{/each}
			</select>

			{#if cardName}
				<label class="label" for="audio-alias">Called here</label>
				<input
					id="audio-alias"
					class="input"
					disabled={!$canEdit}
					placeholder={cardName}
					value={names?.alias ?? ''}
					onchange={(e) => rename({ alias: e.currentTarget.value })}
				/>
			{/if}
		</div>

		{#if cardName}
			<p class="note">
				Name the channels for what is plugged into them. A zone mapped to "FOH L, FOH R" is a
				mapping somebody can check against the patch sheet; one mapped to "1, 2" is not.
			</p>
			<div class="channel-names">
				{#each channels.filter((c) => c <= channelCount) as channel (channel)}
					<label class="channel-name">
						<span class="number">{channel}</span>
						<input
							class="input"
							disabled={!$canEdit}
							placeholder="ch {channel}"
							value={names?.channels?.[channel - 1] ?? ''}
							onchange={(e) => rename({ channel, name: e.currentTarget.value })}
						/>
					</label>
				{/each}
			</div>
		{/if}
		{#if trouble}<p class="says-bad">{trouble}</p>{/if}
	</section>

	<section>
		<header>
			<h3>Speaker zones <span class="whose">the show</span></h3>
			<span class="spacer"></span>
			{#if $canEdit}
				<button class="btn btn-ghost" onclick={() => (creating = !creating)}>{creating ? 'Cancel' : '+ Zone'}</button>
			{/if}
		</header>
		{#if creating && $canEdit}
			<form
				class="inline"
				onsubmit={(e) => {
					e.preventDefault();
					void addZone();
				}}
			>
				<input class="input" placeholder="Name, e.g. Area 2" bind:value={newName} use:focusOnMount />
				<div class="segmented" role="group" aria-label="Width">
					{#each [1, 2] as w (w)}
						<button type="button" class:on={newWidth === w} onclick={() => (newWidth = w)}>{widthName(w)}</button>
					{/each}
				</div>
				<button class="btn btn-primary" type="submit">Create</button>
			</form>
		{/if}

		{#if $zones.length === 0}
			<p class="empty">
				No zones yet. A zone is a place sound goes — "Area 2", "the tanks", "the entrance" —
				and every station maps it to its own channels, so a spare that takes over plays each
				scare out of the same speakers.
			</p>
		{:else}
			<Foldered kind="speaker-zone" rows={$zones}>
				{#snippet item(zone)}
					<Card
						ref={{ kind: 'speaker-zone', id: zone.id }}
						name={zone.name}
						color={zone.color}
						state={`${widthName(zone.width)}${zone.nodes.length ? ` + ${zone.nodes.length} node` : ''}`}
					/>
				{/snippet}
			</Foldered>

			<div class="matrix-scroll">
				<table class="matrix">
					<thead>
						<tr>
							<th class="corner" rowspan="2"></th>
							{#each blocks as block (block.id)}
								<th class="station" class:offline={!block.online} colspan={block.channels.length} title={block.told ? '' : 'It has not said what card it plays through yet'}>
									{block.label}{block.online ? '' : ' · offline'}{block.told ? '' : ' — card unknown'}
									{#if !block.online && $canEdit}
										<button class="forget" title="Forget this station: its row and its mappings" onclick={() => (forgetting = block)}>Forget…</button>
									{/if}
								</th>
							{/each}
							{#each nodeBlocks as node (node.serial)}
								<th class="station node" class:offline={!node.online} colspan={NODE_CHANNELS} title="An OpenHaunt node's loudspeaker">
									<span class="kind">node</span>
									{node.label}{node.found ? (node.online ? '' : ' · offline') : ' · not found'}
								</th>
							{/each}
						</tr>
						<tr>
							{#each blocks as block (block.id)}
								{#each block.channels as channel (channel)}
									<th
										class:missing={channel > block.count}
										class:first={channel === 1}
										title={channel > block.count ? `${channelName(block.names, channel)} — this card has not got it` : channelName(block.names, channel)}
									>
										<span class="number">{channel}</span>
										{#if block.names?.channels?.[channel - 1]}<span class="cname">{block.names.channels[channel - 1]}</span>{/if}
									</th>
								{/each}
							{/each}
							{#each nodeBlocks as node (node.serial)}
								{#each nodeChannels as channel (channel)}
									<th class:first={channel === 1} title="{node.label}, {channel === 1 ? 'left' : 'right'}">
										<span class="number">{channel}</span>
										<span class="cname">{nodeOutputName(channel)}</span>
									</th>
								{/each}
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each $zones as zone (zone.id)}
							<tr class:picked={isSubject($subject, { kind: 'speaker-zone', id: zone.id })}>
								<th class="zone" style:--zone={zone.color ?? 'var(--cat-setup)'}>
									<button class="zbutton" onclick={(e) => selectSubject({ kind: 'speaker-zone', id: zone.id }, e.metaKey || e.ctrlKey || e.shiftKey)}>
										<span class="zname">{zone.name}</span>
										<!-- A zone mapped on some stations and not others is the ordinary case — a
										     scare plays where it is wired — so the row says what the zone is, not
										     where it is missing. -->
										<span class="zwidth">{widthName(zone.width)}</span>
									</button>
								</th>
								{#each blocks as block (block.id)}
									{#each block.channels as channel (channel)}
										{@const carried = sidesAt(zone, block.id, channel)}
										{@const said = carried.map((side) => sideName(zone.width, side)).join('+')}
										<td class:first={channel === 1}>
											<button
												class="cell"
												class:on={carried.length > 0}
												disabled={!$canEdit}
												aria-label="{zone.name} on {block.label} {channelName(block.names, channel)}"
												aria-pressed={carried.length > 0}
												title={said ? `${zone.name} ${said} on ${channelName(block.names, channel)}` : ''}
												onclick={() => toggle(zone.id, block.id, channel)}
											>{zone.width > 1 ? said : ''}</button>
										</td>
									{/each}
								{/each}
								{#each nodeBlocks as node (node.serial)}
									{#each nodeChannels as channel (channel)}
										{@const carried = nodeSidesAt(zone, node.serial, channel)}
										{@const said = carried.map((side) => sideName(zone.width, side)).join('+')}
										<td class:first={channel === 1}>
											<button
												class="cell"
												class:on={carried.length > 0}
												disabled={!$canEdit}
												aria-label="{zone.name} on {node.label} {channel === 1 ? 'left' : 'right'}"
												aria-pressed={carried.length > 0}
												title={said ? `${zone.name} ${said} on ${node.label} ${channel === 1 ? 'left' : 'right'}` : ''}
												onclick={() => toggleNode(zone.id, node.serial, channel)}
											>{zone.width > 1 ? said : ''}</button>
										</td>
									{/each}
								{/each}
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="note">
				A click puts a mono zone on that output or takes it off; on a stereo zone it goes
				round left, right and off, so each side is set on its own. Any number of outputs can
				carry the same side — a bed on all eight Dante channels is eight clicks. A zone a
				station has not mapped is not played there, and says so on the timeline rather than
				coming out of some other speaker. A node is the two columns after the stations — its
				left and right. Whether a clip is streamed to a node or stored on it is the clip's own
				setting, in its Inspector.
			</p>
		{/if}
	</section>
</div>
	<!-- A zone picked up is edited beside the matrix: Setup covers the workspace, so an
	     Inspector panel somewhere behind it is no help. The timeline docks one the same way. -->
	{#if inspecting}
		<aside class="dock"><InspectorPanel /></aside>
	{/if}
</div>

{#if forgetting}
	{@const block = forgetting}
	{@const lose = mappedOn(block.id)}
	<Dialog title="Forget {block.label}?" onclose={() => (forgetting = null)} onaccept={() => void forget(block)}>
		<div class="prompt">
		<p>
			The show stops remembering this station.
			{#if lose.length > 0}
				{lose.length === 1 ? 'One zone loses' : `${lose.length} zones lose`} where it comes out there:
				{lose.map((z) => z.name).join(', ')}.
			{:else}
				No zone is mapped on it.
			{/if}
		</p>
		<p class="note">If it comes back, it is listed again and has to be mapped again. Ctrl-Z takes it back as one act.</p>
		</div>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (forgetting = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={() => void forget(block)}>Forget</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.audio-page {
		display: flex;
		align-items: flex-start;
		min-width: 0;
	}
	.audio-page > .audio {
		flex: 1;
		min-width: 0;
	}
	.dock {
		position: sticky;
		top: 0;
		width: 320px;
		flex: none;
		border-left: 1px solid var(--line);
		max-height: 100%;
		overflow-y: auto;
	}
	.station {
		text-align: left !important;
		font-size: var(--font-xs);
		font-weight: 600 !important;
		color: var(--text);
		border-left: 2px solid var(--line-strong);
		padding: 4px 6px !important;
		white-space: nowrap;
		max-width: none !important;
	}
	.matrix th.first,
	.matrix td.first {
		border-left: 2px solid var(--line-strong);
	}
	.prompt {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.prompt p {
		margin: 0;
		line-height: 1.5;
	}
	.station.node .kind {
		color: var(--text-faint);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-weight: 500;
		margin-right: 4px;
	}
	.station.offline {
		color: var(--text-dim);
		font-style: italic;
	}
	.forget {
		margin-left: 6px;
		font: inherit;
		font-style: normal;
		font-size: var(--font-xs);
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: 4px;
		color: var(--text-dim);
		cursor: pointer;
		padding: 0 6px;
	}
	.zbutton {
		all: unset;
		cursor: pointer;
		display: block;
	}
	tr.picked th.zone {
		background: var(--bg-hover, #2a2a2a);
	}
	.audio {
		display: flex;
		flex-direction: column;
		gap: 20px;
		padding: var(--pad);
	}
	section {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	header {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.spacer {
		flex: 1;
	}
	h3 {
		margin: 0;
		font-size: var(--font-md, 14px);
		display: flex;
		align-items: baseline;
		gap: 8px;
	}
	.whose {
		font-size: var(--font-xs);
		font-weight: 500;
		color: var(--text-faint);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.channel-names {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
		gap: 4px 8px;
	}
	.channel-name {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.channel-name .input {
		flex: 1;
		min-width: 0;
	}
	.number {
		display: inline-block;
		min-width: 1.6em;
		text-align: right;
		color: var(--text-faint);
		font-size: var(--font-xs);
		font-variant-numeric: tabular-nums;
	}
	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.matrix-scroll {
		overflow-x: auto;
		border: 1px solid var(--line);
		border-radius: var(--radius);
	}
	.matrix {
		border-collapse: collapse;
		font-size: var(--font-xs);
	}
	.matrix th,
	.matrix td {
		border-bottom: 1px solid var(--line);
		padding: 2px;
	}
	.matrix thead th {
		vertical-align: bottom;
		text-align: center;
		font-weight: 500;
		min-width: 30px;
		max-width: 64px;
	}
	.matrix thead th.missing,
	.matrix thead th.missing .number {
		color: var(--bad);
	}
	.cname {
		display: block;
		max-width: 64px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--text-dim);
	}
	.matrix th.zone {
		position: sticky;
		left: 0;
		background: var(--bg-chrome);
		text-align: left;
		padding: 4px 10px 4px 8px;
		border-left: 3px solid var(--zone);
		min-width: 120px;
	}
	.matrix th.corner {
		position: sticky;
		left: 0;
		background: var(--bg-chrome);
	}
	.zname {
		display: block;
		font-size: var(--font-sm);
		color: var(--text);
		font-weight: 500;
	}
	.zwidth {
		color: var(--text-faint);
	}
	.cell {
		width: 26px;
		height: 26px;
		border: 1px solid var(--line-strong);
		border-radius: 3px;
		background: var(--bg-sunken);
		cursor: pointer;
		padding: 0;
	}
	.cell.on {
		background: var(--accent-solid);
		border-color: var(--accent-solid);
		color: var(--text-bright, #fff);
		font-size: 10px;
		font-weight: 700;
	}
	.cell:disabled {
		cursor: default;
	}
</style>
