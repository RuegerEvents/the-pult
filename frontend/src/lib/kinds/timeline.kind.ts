/**
 * What is written on a timeline, as objects: its clips, events, markers, tempo
 * segments and takes.
 *
 * Each is something a person picks up on a lane, so each is a kind — and that is what
 * puts its editor in the Inspector rather than in a form under the canvas, the way a
 * preset's is. The rows are read off the `timelines` collection, so a clip is still a
 * field of its timeline in the schema; registering it here changes nothing about where
 * it is stored, only how it is reached.
 *
 * A tap on one **goes to it** (a take's tap turns it on or off, since a take has no one
 * place to go to), which is what tapping a thing on a timeline does in every editor.
 */

import { derived, get, type Readable } from 'svelte/store';

import AudioLines from '@lucide/svelte/icons/audio-lines';
import Bookmark from '@lucide/svelte/icons/bookmark';
import Zap from '@lucide/svelte/icons/zap';
import Metronome from '@lucide/svelte/icons/metronome';
import Activity from '@lucide/svelte/icons/activity';

import type { AudioClip, GridSegment, Marker, Timeline, TimelineEvent, TimelineTrack } from '$lib/generated/index.js';
import { copyName, registerKind, type ObjectRow } from '$lib/objects.js';
import { collection } from '$lib/stores/show.js';
import { current, locate, setClip, timelines, writeColumns } from '$lib/stores/timelines.js';
import { describeAction } from '$lib/timeline/describe.js';
import { removed, segmentId, segmentIndex, type ItemRef } from '$lib/timeline/lanes.js';
import { clock } from '$lib/waveform.js';
import ClipInspector from '$lib/components/timeline/inspectors/ClipInspector.svelte';
import EventInspector from '$lib/components/timeline/inspectors/EventInspector.svelte';
import MarkerInspector from '$lib/components/timeline/inspectors/MarkerInspector.svelte';
import SegmentInspector from '$lib/components/timeline/inspectors/SegmentInspector.svelte';
import TakeInspector from '$lib/components/timeline/inspectors/TakeInspector.svelte';
import { uuid } from '$lib/uuid.js';

export type ClipRow = ObjectRow & { timelineId: string; clip: AudioClip };
export type EventRow = ObjectRow & { timelineId: string; event: TimelineEvent };
export type MarkerRow = ObjectRow & { timelineId: string; marker: Marker };
export type SegmentRow = ObjectRow & { timelineId: string; index: number; segment: GridSegment };
export type TakeRow = ObjectRow & { timelineId: string; track: TimelineTrack };

const sequences = collection('sequences');
const cues = collection('cues');
const variables = collection('variables');

const all = timelines as Readable<Timeline[]>;

const clipRows: Readable<ClipRow[]> = derived(all, ($t) =>
	$t.flatMap((t) => t.clips.map((clip) => ({ id: clip.id, name: clip.name || 'Clip', timelineId: t.id, clip })))
);
const eventRows: Readable<EventRow[]> = derived([all, sequences, cues, variables], ([$t, $s, $c, $v]) =>
	$t.flatMap((t) =>
		t.events.map((event) => ({
			id: event.id,
			name: `${clock(event.at_ms)} · ${describeAction(event.action, $s, $c, $t, $v)}`,
			timelineId: t.id,
			event
		}))
	)
);
const markerRows: Readable<MarkerRow[]> = derived(all, ($t) =>
	$t.flatMap((t) => t.markers.map((marker) => ({ id: marker.id, name: marker.name, timelineId: t.id, marker })))
);
const segmentRows: Readable<SegmentRow[]> = derived(all, ($t) =>
	$t.flatMap((t) =>
		t.grid.map((segment, index) => ({
			id: segmentId(t.id, index),
			name: `${Math.round(segment.bpm * 10) / 10} bpm from ${clock(segment.at_ms)}`,
			timelineId: t.id,
			index,
			segment
		}))
	)
);
const takeRows: Readable<TakeRow[]> = derived(all, ($t) =>
	$t.flatMap((t) => t.tracks.map((track) => ({ id: track.id, name: track.name, timelineId: t.id, track })))
);

/** Take one item off its timeline. */
async function drop(timelineId: string, ref: ItemRef) {
	const timeline = current(timelineId);
	if (timeline) await writeColumns(timelineId, removed(timeline, [ref], segmentIndex));
}

registerKind<ClipRow>({
	kind: 'timeline-clip',
	label: 'Clip',
	category: 'playback',
	icon: AudioLines,
	rows: clipRows,
	panel: 'timeline',
	actLabel: 'Go to',
	act: (row) => locate(row.timelineId, row.clip.start_ms),
	rename: (row, name) => setClip(row.timelineId, row.id, { name }),
	duplicate: async (row) => {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const copy: AudioClip = {
			...row.clip,
			id: uuid(),
			name: copyName(row.clip.name, timeline.clips.map((c) => c.name)),
			// Right after the original, where a second one of the same sound usually goes.
			start_ms: row.clip.start_ms + 1_000
		};
		await writeColumns(row.timelineId, { clips: [...timeline.clips, copy] });
	},
	remove: (row) => drop(row.timelineId, { kind: 'timeline-clip', id: row.id }),
	inspector: ClipInspector
});

registerKind<EventRow>({
	kind: 'timeline-event',
	label: 'Event',
	category: 'playback',
	icon: Zap,
	rows: eventRows,
	panel: 'timeline',
	actLabel: 'Go to',
	act: (row) => locate(row.timelineId, row.event.at_ms),
	duplicate: async (row) => {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const events = [...timeline.events, { ...row.event, id: uuid(), at_ms: row.event.at_ms + 1_000 }];
		await writeColumns(row.timelineId, { events: events.sort((a, b) => a.at_ms - b.at_ms) });
	},
	remove: (row) => drop(row.timelineId, { kind: 'timeline-event', id: row.id }),
	inspector: EventInspector
});

registerKind<MarkerRow>({
	kind: 'timeline-marker',
	label: 'Marker',
	category: 'playback',
	icon: Bookmark,
	rows: markerRows,
	panel: 'timeline',
	actLabel: 'Go to',
	act: (row) => locate(row.timelineId, row.marker.at_ms),
	rename: async (row, name) => {
		const timeline = current(row.timelineId);
		if (timeline) await writeColumns(row.timelineId, { markers: timeline.markers.map((m) => (m.id === row.id ? { ...m, name } : m)) });
	},
	duplicate: async (row) => {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const copy = { ...row.marker, id: uuid(), at_ms: row.marker.at_ms + 1_000, name: copyName(row.marker.name, timeline.markers.map((m) => m.name)) };
		await writeColumns(row.timelineId, { markers: [...timeline.markers, copy] });
	},
	remove: (row) => drop(row.timelineId, { kind: 'timeline-marker', id: row.id }),
	inspector: MarkerInspector
});

registerKind<SegmentRow>({
	kind: 'timeline-segment',
	label: 'Tempo',
	category: 'playback',
	icon: Metronome,
	rows: segmentRows,
	panel: 'timeline',
	actLabel: 'Go to',
	act: (row) => locate(row.timelineId, row.segment.at_ms),
	remove: (row) => drop(row.timelineId, { kind: 'timeline-segment', id: row.id }),
	inspector: SegmentInspector
});

registerKind<TakeRow>({
	kind: 'timeline-take',
	label: 'Take',
	category: 'playback',
	icon: Activity,
	rows: takeRows,
	panel: 'timeline',
	actLabel: 'Turn on or off',
	act: async (row) => {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		await writeColumns(row.timelineId, {
			tracks: timeline.tracks.map((t) => (t.id === row.id ? { ...t, enabled: !t.enabled } : t))
		});
	},
	rename: async (row, name) => {
		const timeline = current(row.timelineId);
		if (timeline) await writeColumns(row.timelineId, { tracks: timeline.tracks.map((t) => (t.id === row.id ? { ...t, name } : t)) });
	},
	remove: (row) => drop(row.timelineId, { kind: 'timeline-take', id: row.id }),
	inspector: TakeInspector
});

/** Which timeline an item on one belongs to, for the panel that has to open it. */
export function timelineOfItem(ref: { kind: string; id: string }): string | null {
	const find = <R extends { id: string; timelineId: string }>(rows: Readable<R[]>) =>
		get(rows).find((r) => r.id === ref.id)?.timelineId ?? null;
	switch (ref.kind) {
		case 'timeline':
			return ref.id;
		case 'timeline-clip':
			return find(clipRows);
		case 'timeline-event':
			return find(eventRows);
		case 'timeline-marker':
			return find(markerRows);
		case 'timeline-segment':
			return find(segmentRows);
		case 'timeline-take':
			return find(takeRows);
		default:
			return null;
	}
}
