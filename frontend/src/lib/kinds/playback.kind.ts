/**
 * What runs the show, as objects the palette can find and act on: sequences, their
 * cues, and timelines. Their own panels keep their own rows for now; registering them
 * here is what lets ⌘K say "Go Area 2 scare" and the Inspector rename one.
 */

import { get } from 'svelte/store';

import AudioWaveform from '@lucide/svelte/icons/audio-waveform';
import ListOrdered from '@lucide/svelte/icons/list-ordered';
import Play from '@lucide/svelte/icons/play';

import type { Cue, Sequence, Timeline } from '$lib/generated/index.js';
import { registerKind } from '$lib/objects.js';
import { collection, showData } from '$lib/stores/show.js';
import { addToast } from '$lib/toasts.js';
import { consoleNow } from '$lib/ws/clock.js';

const sequences = collection('sequences');
const cues = collection('cues');
const timelines = collection('timelines');

/** A Go carries the moment it was taken, like every transport act. */
const stamp = () => consoleNow() ?? Date.now();

registerKind<Sequence>({
	kind: 'sequence',
	label: 'Sequence',
	category: 'playback',
	icon: Play,
	rows: sequences,
	panel: 'playback',
	actLabel: 'Go',
	act: (sequence) => showData().sequences.byId(sequence.id).goNext({ at: stamp() }),
	rename: (sequence, name) => showData().sequences.byId(sequence.id).name.set(name),
	verbs: (sequence) => [
		{
			id: 'off',
			label: 'Off',
			structural: false,
			run: () => showData().sequences.byId(sequence.id).off({ at: stamp() })
		}
	]
});

registerKind<Cue>({
	kind: 'cue',
	label: 'Cue',
	category: 'playback',
	icon: ListOrdered,
	rows: cues,
	panel: 'cues',
	actLabel: 'Go to',
	act: async (cue) => {
		const sequence = get(sequences).find((s) => s.cue_ids.includes(cue.id));
		if (!sequence) return addToast(`Cue ${cue.number} is in no sequence`);
		await showData().sequences.byId(sequence.id).goToCue({ cueId: cue.id, at: stamp() });
	},
	rename: (cue, name) => showData().cues.byId(cue.id).name.set(name)
});

registerKind<Timeline>({
	kind: 'timeline',
	label: 'Timeline',
	category: 'playback',
	icon: AudioWaveform,
	rows: timelines,
	panel: 'timeline',
	actLabel: 'Play or stop',
	act: async (timeline) => {
		const at = consoleNow();
		if (at === null) return addToast('This browser has not worked out the station clock yet');
		const proxy = showData().timelines.byId(timeline.id);
		await (timeline.running ? proxy.stop({ at }) : proxy.play({ at }));
	},
	rename: (timeline, name) => showData().timelines.byId(timeline.id).name.set(name)
});
