/**
 * Speaker zones: where a clip's sound goes, named for the room.
 *
 * Nothing to *do* with one at a tap — a zone is a place, not an act — so a tap makes
 * it the subject and the Inspector says where it comes out on every station. Deleting
 * one cascades nothing: a clip going there says so on the playing station's status row,
 * and Ctrl-Z of the delete puts every route back, because no route was rewritten.
 */

import { get } from 'svelte/store';

import Speaker from '@lucide/svelte/icons/speaker';

import type { SpeakerZone } from '$lib/generated/index.js';
import { copyName, registerKind, selectSubject } from '$lib/objects.js';
import { showData } from '$lib/stores/show.js';
import { zones } from '$lib/stores/zones.js';
import ZoneInspector from '$lib/components/audio/ZoneInspector.svelte';

registerKind<SpeakerZone>({
	kind: 'speaker-zone',
	label: 'Speaker zone',
	category: 'setup',
	icon: Speaker,
	rows: zones,
	panel: 'audio',
	actLabel: 'Inspect',
	act: (zone) => selectSubject({ kind: 'speaker-zone', id: zone.id }),
	rename: (zone, name) => showData().speaker_zones.byId(zone.id).name.set(name),
	recolor: (zone, color) => showData().speaker_zones.byId(zone.id).color.set(color),
	duplicate: async (zone) => {
		await showData().speaker_zones.create({
			...zone,
			id: crypto.randomUUID(),
			name: copyName(zone.name, get(zones).map((z) => z.name))
		});
	},
	remove: (zone) => showData().speaker_zones.byId(zone.id).delete(),
	inspector: ZoneInspector
});
