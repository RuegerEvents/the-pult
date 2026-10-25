/**
 * The pools' kinds: saved groups and presets.
 *
 * A group's tap recalls its question; a preset's applies its look to the selection,
 * as much of it as the selection takes.
 */

import { get } from 'svelte/store';

import LayoutGrid from '@lucide/svelte/icons/layout-grid';
import SquareMousePointer from '@lucide/svelte/icons/square-mouse-pointer';

import type { Group, Preset } from '$lib/generated/index.js';
import { copyName, registerKind } from '$lib/objects.js';
import { viewPreset } from '$lib/stores/cues.js';
import { beginGesture, endGesture } from '$lib/stores/gesture.js';
import { entries, recallPreset, updatePreset } from '$lib/stores/programmer.js';
import { recall, selection } from '$lib/stores/selection.js';
import { collection, showData } from '$lib/stores/show.js';
import { addToast } from '$lib/toasts.js';
import GroupInspector from '$lib/components/pools/GroupInspector.svelte';
import PresetInspector from '$lib/components/pools/PresetInspector.svelte';
import { uuid } from '$lib/uuid.js';

const groups = collection('groups');
const presets = collection('presets');

registerKind<Group>({
	kind: 'group',
	label: 'Group',
	category: 'program',
	icon: SquareMousePointer,
	rows: groups,
	panel: 'pools',
	actLabel: 'Recall',
	act: (group) => recall(group.query),
	rename: (group, name) => showData().groups.byId(group.id).name.set(name),
	recolor: (group, color) => showData().groups.byId(group.id).color.set(color),
	duplicate: async (group) => {
		await showData().groups.create({
			...group,
			id: uuid(),
			name: copyName(group.name, get(groups).map((g) => g.name))
		});
	},
	remove: (group) => showData().groups.byId(group.id).delete(),
	inspector: GroupInspector
});

registerKind<Preset>({
	kind: 'preset',
	label: 'Preset',
	category: 'program',
	icon: LayoutGrid,
	rows: presets,
	panel: 'pools',
	actLabel: 'Apply',
	act: async (preset) => {
		const count = await recallPreset(preset, get(selection));
		if (count === 0) addToast(`“${preset.name}” says nothing about what is selected`);
	},
	rename: (preset, name) => showData().presets.byId(preset.id).name.set(name),
	recolor: (preset, color) => showData().presets.byId(preset.id).color.set(color),
	duplicate: async (preset) => {
		await showData().presets.create({
			...preset,
			id: uuid(),
			name: copyName(preset.name, get(presets).map((p) => p.name))
		});
	},
	remove: (preset) => showData().presets.byId(preset.id).delete(),
	verbs: (preset) => [
		{
			id: 'update',
			label: 'Update from programmer',
			structural: true,
			disabled: get(entries).length === 0 ? 'The programmer is empty' : false,
			run: async () => {
				beginGesture();
				try {
					const n = await updatePreset(preset);
					addToast(
						n > 0 ? `“${preset.name}” updated — every cue using it followed` : 'the programmer is empty'
					);
				} finally {
					endGesture();
				}
			}
		},
		{ id: 'sheet', label: 'Show in the sheet', structural: false, run: () => viewPreset(preset.id) }
	],
	inspector: PresetInspector
});
