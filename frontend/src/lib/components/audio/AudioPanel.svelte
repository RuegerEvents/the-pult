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
	 * An errand rather than a panel somebody keeps open: it is done at the venue, once
	 * per machine, and then a clip is routed to "Area 2" and never to a number.
	 */

	import { onMount } from 'svelte';

	import Card from '$lib/components/objects/Card.svelte';
	import { focusOnMount } from '$lib/actions.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { showData } from '$lib/stores/show.js';
	import { audioOutput } from '$lib/stores/timelines.js';
	import { audioPrefs, createZone, loadAudioPrefs, nameDevice, namesFor, stationNames, zones } from '$lib/stores/zones.js';
	import { writePreferences } from '$lib/preferences.js';
	import { getClientContext, getStationContext } from '$lib/ws/context.js';
	import { channelName, freshZoneName, mappingOn, needsMapping, toggleChannel, unmappedOn, widthName, zoneProblem } from '$lib/zones.js';
	import { selectSubject } from '$lib/objects.js';

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
	const hereName = $derived($stationNames.find((s) => s.id === here)?.name ?? 'this station');

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
		const mapped = here ? $zones.flatMap((z) => z.channels[here] ?? []) : [];
		const last = Math.min(64, Math.max(channelCount, ...mapped));
		return Array.from({ length: last }, (_, i) => i + 1);
	});

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

	function toggle(zoneId: string, channel: number) {
		const zone = $zones.find((z) => z.id === zoneId);
		if (!zone || !here) return;
		void showData().speaker_zones.byId(zone.id).channels.set(toggleChannel(zone, here, channel));
	}

	const groups = (list: AudioDevice[]) =>
		[
			{ label: 'This machine uses this by default', devices: list.filter((d) => d.is_default) },
			{ label: 'Everything else', devices: list.filter((d) => !d.is_default) }
		].filter((g) => g.devices.length > 0);
</script>

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
			<div class="cards">
				{#each $zones as zone (zone.id)}
					{@const missing = unmappedOn(zone, $stationNames)}
					<Card
						ref={{ kind: 'speaker-zone', id: zone.id }}
						name={zone.name}
						color={zone.color}
						state={missing.length ? `not mapped on ${missing.join(', ')}` : `${widthName(zone.width)}${zone.nodes.length ? ` + ${zone.nodes.length} node` : ''}`}
						dim={missing.length > 0}
					/>
				{/each}
			</div>

			<h4>On {hereName}{names?.alias ? ` · ${names.alias}` : cardName ? ` · ${cardName}` : ''}</h4>
			<div class="matrix-scroll">
				<table class="matrix">
					<thead>
						<tr>
							<th class="corner"></th>
							{#each channels as channel (channel)}
								<th
									class:missing={channel > channelCount}
									title={channel > channelCount ? `${channelName(names, channel)} — this card has not got it` : channelName(names, channel)}
								>
									<span class="number">{channel}</span>
									{#if names?.channels?.[channel - 1]}<span class="cname">{names.channels[channel - 1]}</span>{/if}
								</th>
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each $zones as zone (zone.id)}
							{@const mapped = here ? (mappingOn(zone, here) ?? []) : []}
							{@const problem = here ? zoneProblem(zone, here, hereName) : null}
							<tr>
								<th class="zone" style:--zone={zone.color ?? 'var(--cat-setup)'}>
									<span class="zname">{zone.name}</span>
									<span class="zwidth" class:warn={problem !== null && needsMapping(zone)}>{problem ?? widthName(zone.width)}</span>
								</th>
								{#each channels as channel (channel)}
									<td>
										<button
											class="cell"
											class:on={mapped.includes(channel)}
											disabled={!$canEdit || !here}
											aria-label="{zone.name} on {channelName(names, channel)}"
											aria-pressed={mapped.includes(channel)}
											onclick={() => toggle(zone.id, channel)}
										></button>
									</td>
								{/each}
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="note">
				A mono zone moves to the channel clicked; a stereo one takes two, and a third click
				drops the oldest. A zone this station has not mapped is not played here, and says so on
				the timeline rather than coming out of some other speaker.
			</p>
		{/if}
	</section>
</div>

<style>
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
	h4 {
		margin: 6px 0 0;
		font-size: var(--font-sm);
		color: var(--text-dim);
		font-weight: 600;
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
	.cards {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
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
	.zwidth.warn {
		color: var(--live);
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
	}
	.cell:disabled {
		cursor: default;
	}
</style>
