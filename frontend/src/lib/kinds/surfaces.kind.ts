/**
 * The Surfaces panel's kind: a page a phone runs the show from. A tap opens it on
 * this screen, full-size, the way the phone will draw it.
 */

import { get } from 'svelte/store';

import Smartphone from '@lucide/svelte/icons/smartphone';

import type { Surface } from '$lib/generated/index.js';
import { copyName, registerKind } from '$lib/objects.js';
import { collection, showData } from '$lib/stores/show.js';
import { surfaceUrl } from '$lib/surface.js';
import { copyText, uuid } from '$lib/uuid.js';

const surfaces = collection('surfaces');

registerKind<Surface>({
	kind: 'surface',
	table: 'surfaces',
	label: 'Surface',
	category: 'playback',
	icon: Smartphone,
	rows: surfaces,
	panel: 'surface',
	actLabel: 'Open full-screen',
	act: (surface) => {
		window.open(surfaceUrl(window.location, surface.id), '_blank');
	},
	rename: (surface, name) => showData().surfaces.byId(surface.id).name.set(name),
	recolor: (surface, color) => showData().surfaces.byId(surface.id).color.set(color),
	duplicate: async (surface) => {
		await showData().surfaces.create({
			...surface,
			id: uuid(),
			name: copyName(surface.name, get(surfaces).map((s) => s.name)),
			// Fresh tile ids: two surfaces sharing a tile id would edit each other's.
			tiles: surface.tiles.map((t) => ({ ...t, id: uuid() }))
		});
	},
	remove: (surface) => showData().surfaces.byId(surface.id).delete(),
	verbs: (surface) => [
		{
			id: 'copy-link',
			label: 'Copy the phone link',
			structural: false,
			run: () => copyText(surfaceUrl(window.location, surface.id))
		}
	]
});
