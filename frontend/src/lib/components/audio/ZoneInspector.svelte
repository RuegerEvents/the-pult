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

		<!-- Where it comes out — every station's channels and every node speaker — is the
		     routing matrix's to show and to set, so it is not repeated here. -->
		<span class="label">Routing</span>
		<div><button class="btn btn-ghost" onclick={() => openSetup('audio')}>Routing matrix…</button></div>

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
