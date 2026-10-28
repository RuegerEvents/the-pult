<script lang="ts">
	/**
	 * A speaker zone: how wide it is, how loud, and where it comes out — on every
	 * station of the session and on every node loudspeaker in it.
	 *
	 * The mapping is the station's to make and is made in Setup → Audio, against that
	 * station's own named channels; here it is *read*, for every station at once,
	 * because "is Area 2 mapped on the spare" is the question a failover rehearsal
	 * asks and the spare's own screen is not where anybody is standing. A node's
	 * loudspeaker is two more columns of the same matrix, so it is routed there too and
	 * only read here.
	 *
	 * Edited in place: each field is one value and Ctrl-Z takes it back.
	 */

	import type { SpeakerZone } from '$lib/generated/index.js';
	import { audioNodes } from '$lib/destination.js';
	import { clearSubject } from '$lib/objects.js';
	import { showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { openSetup } from '$lib/stores/setup.js';
	import { audioOutput, devices, drivingReport, timelines } from '$lib/stores/timelines.js';
	import { audioPrefs, loadAudioPrefs, namesFor, stationNames } from '$lib/stores/zones.js';
	import { mappingText, nodeSidesOf, sideName, widthName, zoneProblem } from '$lib/zones.js';
	import { getStationContext } from '$lib/ws/context.js';

	let { rows }: { rows: SpeakerZone[] } = $props();

	const station = getStationContext();
	loadAudioPrefs();
	const nodes = $derived(audioNodes($devices, $drivingReport));
	const here = $derived($station?.nodeId ?? null);
	const names = $derived(namesFor($audioPrefs, $audioOutput?.device));

	const zone = (id: string) => showData().speaker_zones.byId(id);

	/** How many clips, on how many timelines, go to it. */
	function usedBy(z: SpeakerZone): string {
		let clips = 0;
		let places = 0;
		for (const t of $timelines) {
			const n = t.clips.filter((c) => 'Zone' in c.destination && c.destination.Zone.zone === z.id).length;
			clips += n;
			if (n > 0) places++;
		}
		return clips === 0 ? 'no clip goes here yet' : `${clips} clip${clips === 1 ? '' : 's'} on ${places} timeline${places === 1 ? '' : 's'}`;
	}

	/** The zone a Delete is being asked about: one press asks, the second deletes. Ctrl-Z puts it back. */
	let deleting = $state<string | null>(null);
	async function remove(z: SpeakerZone) {
		deleting = null;
		clearSubject();
		await zone(z.id).delete();
	}

	function setWidth(z: SpeakerZone, width: number) {
		void zone(z.id).width.set(Math.max(1, Math.min(64, Math.round(width))));
	}

	/** Where a zone comes out of one node's two outputs, in words. */
	function onNode(z: SpeakerZone, serial: string): string {
		const output = (o: number) => (o === 1 ? 'left' : o === 2 ? 'right' : `output ${o}`);
		const sides = nodeSidesOf(z, serial);
		const said = (outputs: number[] | undefined) => (outputs?.length ? outputs.map(output).join(' and ') : 'nowhere');
		const how = z.nodes.find((n) => n.serial === serial)?.mode === 'Clip' ? ', stored' : ', streamed';
		if (z.width === 1) return said(sides[0]) + how;
		return sides.map((o, i) => `${sideName(z.width, i)}: ${said(o)}`).join(' · ') + how;
	}

	/** One station's mapping, in that station's words where this is that station. */
	function mapped(z: SpeakerZone, stationId: string): string {
		const text = mappingText(z, stationId, stationId === here ? names : undefined);
		// Not mapped on a station is the ordinary case, not a fault: said as a dash.
		if (!text) return '—';
		return zoneProblem(z, stationId, 'it') ? `${text} — ${widthName(z.width)}, a side goes nowhere` : text;
	}
</script>

{#each rows as z (z.id)}
	<div class="fields">
		<span class="label">Width</span>
		<div class="inline">
			<div class="segmented" role="group" aria-label="How many channels">
				{#each [1, 2] as w (w)}
					<button class:on={z.width === w} disabled={!$canEdit} onclick={() => setWidth(z, w)}>{widthName(w)}</button>
				{/each}
			</div>
			<input
				class="input narrow"
				type="number"
				min="1"
				max="64"
				disabled={!$canEdit}
				value={z.width}
				aria-label="Channels"
				onchange={(e) => setWidth(z, Number(e.currentTarget.value) || 1)}
			/>
		</div>

		<!-- Over every clip that goes here, on the station's channels and the node
		     speakers alike: a room that is too loud is turned down once, not per scare. -->
		<span class="label">Level</span>
		<div class="inline">
			<input
				class="input narrow"
				type="number"
				step="1"
				max="12"
				disabled={!$canEdit}
				value={z.level_db ?? 0}
				aria-label="Zone level in dB"
				onchange={(e) => {
					const db = Number(e.currentTarget.value);
					if (Number.isFinite(db) && e.currentTarget.value !== '') void zone(z.id).level_db!.set(db === 0 ? null : Math.min(12, db));
				}}
			/>
			<span class="note">dB{z.nodes.length && (z.level_db ?? 0) > 0 ? ' — a node speaker stops at 0 dB' : ''}</span>
		</div>

		<span class="label">Used by</span>
		<span class="note">{usedBy(z)}</span>

		<span class="label">On stations</span>
		<div class="stations">
			{#each $stationNames as s (s.id)}
				{@const text = mapped(z, s.id)}
				<span class="station" class:here={s.id === here} class:gone={!s.online}>{s.name}{s.id === here ? ' (this one)' : s.online ? '' : ' (offline)'}</span>
				<span class:says-bad={text.includes('goes nowhere')} class="map">{text}</span>
			{/each}
			{#each Object.keys(z.channels).filter((id) => !$stationNames.some((s) => s.id === id)) as gone (gone)}
				<span class="station gone">a station not here</span>
				<span class="map">{mappingText(z, gone, undefined)}</span>
			{/each}
		</div>
		<span></span>
		<div><button class="btn btn-ghost" onclick={() => openSetup('audio')}>Map on this station…</button></div>

		{#if z.nodes.length}
			<span class="label">Node speakers</span>
			<div class="stations">
				{#each z.nodes as member (member.serial)}
					{@const node = nodes.find((n) => n.serial === member.serial)}
					<span class="station" class:gone={!node?.online}>{node?.name ?? member.serial}{node ? (node.online ? '' : ' (offline)') : ' (not found)'}</span>
					<span class="map">{onNode(z, member.serial)}</span>
				{/each}
			</div>
		{/if}

		<span class="label">Notes</span>
		<input
			class="input"
			disabled={!$canEdit}
			placeholder="Where the speakers are"
			value={z.notes}
			onchange={(e) => void zone(z.id).notes.set(e.currentTarget.value)}
		/>

		{#if $canEdit}
			<span></span>
			<div class="inline">
				{#if deleting === z.id}
					<span class="says-bad">{usedBy(z) === 'no clip goes here yet' ? 'Delete it?' : `${usedBy(z)} — they will play nowhere. Delete it?`}</span>
					<button class="btn btn-danger" onclick={() => void remove(z)}>Delete</button>
					<button class="btn btn-ghost" onclick={() => (deleting = null)}>Cancel</button>
				{:else}
					<button class="btn btn-ghost" onclick={() => (deleting = z.id)}>Delete zone…</button>
				{/if}
			</div>
		{/if}
	</div>
{/each}

<style>
	.stations,
	.station {
		color: var(--text);
	}
	.station.here {
		font-weight: 600;
	}
	.station.gone {
		color: var(--text-faint);
	}
	.map {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.map.says-bad {
		color: var(--live);
	}
</style>
