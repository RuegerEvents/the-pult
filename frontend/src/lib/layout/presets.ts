/**
 * The layouts that are always there.
 *
 * A console has to open onto something the first time it is used, and an operator
 * who has rearranged everything into a corner needs a way back. Presets are built in
 * rather than seeded into the showfile: a show carried to another building should
 * not also carry a broken arrangement that nobody can reset.
 *
 * Saving over one is not possible — *Save as…* makes a layout in the show, and that
 * is what a house rig's own arrangement should be.
 */

import type { LayoutNode } from '$lib/generated/index.js';
import { split, tabs } from '$lib/layout.js';

export type Preset = { key: string; name: string; tree: LayoutNode };

export const PRESETS: Preset[] = [
	{
		key: 'programming',
		name: 'Programming',
		// The rig to point things in, the values to set them with, and the cue list
		// underneath to store them into: the spec's programmer, in one screen.
		tree: split(
			'Column',
			[
				split('Row', [tabs(['rig']), split('Column', [tabs(['values']), tabs(['selection'])], [0.62, 0.38])], [0.6, 0.4]),
				tabs(['playback'])
			],
			[0.75, 0.25]
		)
	},
	{
		key: 'playback',
		name: 'Playback',
		tree: split('Row', [tabs(['playback']), tabs(['plan'])], [0.65, 0.35])
	},
	{
		key: 'plan-rig',
		name: 'Plan & Rig',
		tree: split(
			'Column',
			[split('Row', [tabs(['plan']), tabs(['rig'])], [0.5, 0.5]), tabs(['values', 'objects'])],
			[0.68, 0.32]
		)
	},
	{
		key: 'scene',
		name: 'Scene',
		// Building a rig by hand, which is what everything in this column is for: the
		// catalogue to drag from and the verbs to act with on the left, the drawing in
		// the middle, and what is in it — as a list, and as the numbers for the one
		// thing picked — on the right. The rig gets the width because it is the thing
		// being aimed at.
		tree: split(
			'Row',
			[
				split('Column', [tabs(['pieces']), tabs(['tools'])], [0.55, 0.45]),
				tabs(['rig', 'plan']),
				split('Column', [tabs(['objects', 'layers']), tabs(['object'])], [0.55, 0.45])
			],
			[0.2, 0.55, 0.25]
		)
	},
	{
		key: 'paperwork',
		name: 'Paperwork',
		// The sheets take the width, because what is being looked at is a piece of A3
		// and everything else on the screen is there to say what is on it. Layers and
		// objects beside it, since deciding what a sheet shows is deciding which parts
		// of the drawing belong on paper.
		tree: split(
			'Row',
			[tabs(['paperwork']), split('Column', [tabs(['layers']), tabs(['objects'])], [0.5, 0.5])],
			[0.78, 0.22]
		)
	},
	{
		key: 'patch',
		name: 'Patch',
		tree: split('Row', [tabs(['patch']), tabs(['devices', 'outputs'])], [0.66, 0.34])
	},
	{
		key: 'effects',
		name: 'Effects',
		// Everything a chase is built from, in reach at once: the rig to pick heads
		// in, the effect editor beside it, the tempo they follow, and the values
		// panel to see what is actually being held.
		tree: split(
			'Column',
			[
				split('Row', [tabs(['rig', 'plan']), tabs(['effects'])], [0.5, 0.5]),
				split('Row', [tabs(['speedmasters']), tabs(['values'])], [0.42, 0.58])
			],
			[0.55, 0.45]
		)
	},
	{
		key: 'flows',
		name: 'Flows',
		tree: split('Row', [tabs(['flows']), tabs(['devices'])], [0.72, 0.28])
	}
];

export const DEFAULT_PRESET = PRESETS[0];

export const presetByKey = (key: string) => PRESETS.find((p) => p.key === key) ?? null;
