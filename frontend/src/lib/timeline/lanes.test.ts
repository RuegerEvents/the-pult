/**
 * The timeline's lanes: which row things go on, what is under the pointer, what a
 * rubber band picks up, and what a drag of several things writes.
 */

import { describe, expect, it } from 'vitest';

import { xOf, type View } from '$lib/waveform.js';
import {
	CLIP_ROW_H,
	FLAG_H,
	KEY_ROW_H,
	RULER_H,
	TAKE_HEAD_H,
	fitAll,
	grabbedFrom,
	hit,
	inBand,
	laneAt,
	layOut,
	moved,
	positionAt,
	rectOf,
	refKey,
	removed,
	segmentId,
	segmentIndex,
	segmentTimeline,
	type LaneInput
} from './lanes.js';

const view: View = { startMs: 0, spanMs: 10_000 };
const width = 1000; // a millisecond is a tenth of a pixel

function scare(): LaneInput {
	return {
		segments: [{ id: segmentId('t', 0), atMs: 0 }],
		markers: [{ id: 'm1', atMs: 1_000 }],
		events: [
			{ id: 'e1', atMs: 0 },
			{ id: 'e2', atMs: 7_000 }
		],
		clips: [
			// Two on Area 2's speakers that overlap, and one on a node that starts first.
			{ id: 'laugh', destination: 'station:4', destinationLabel: 'ch 4', fromMs: 300, toMs: 4_000 },
			{ id: 'bed', destination: 'station:4', destinationLabel: 'ch 4', fromMs: 2_000, toMs: 9_000 },
			{ id: 'bang', destination: 'node:4d5e6f', destinationLabel: 'Area 2 node', fromMs: 0, toMs: 1_500 }
		],
		takes: [
			{
				id: 'jaw',
				name: 'Take 3',
				source: 'programmer',
				offsetMs: 100,
				keys: [
					{ label: 'Skeleton · Jaw', fromMs: 300, toMs: 3_800 },
					{ label: 'Skeleton · Head', fromMs: 500, toMs: 3_000 }
				]
			},
			{ id: 'waiting', name: 'Take 4', source: 'Guest', offsetMs: 0, keys: null }
		]
	};
}

describe('the lanes', () => {
	it('come in a fixed order: markers, events, sound by when it starts, then takes', () => {
		const { lanes } = layOut(scare());
		expect(lanes.map((l) => l.key)).toEqual([
			'markers',
			'events',
			'audio:node:4d5e6f',
			'audio:station:4',
			'take:jaw',
			'take:waiting'
		]);
		expect(lanes[0].top).toBe(RULER_H);
		expect(lanes[1].top).toBe(RULER_H + FLAG_H);
	});

	it('give one destination one lane, with a row for each sound that overlaps another', () => {
		const { lanes, items } = layOut(scare());
		const speakers = lanes.find((l) => l.key === 'audio:station:4')!;
		expect(speakers.rows.length).toBe(2);
		expect(speakers.height).toBe(2 * CLIP_ROW_H);
		const rowOf = (id: string) => items.find((i) => i.ref.id === id)!.row;
		expect([rowOf('laugh'), rowOf('bed')].sort()).toEqual([0, 1]);
	});

	it('give a take a row per key it recorded, and one row while its outline is on the way', () => {
		const { lanes, items } = layOut(scare());
		const jaw = lanes.find((l) => l.key === 'take:jaw')!;
		expect(jaw.rows).toEqual(['Skeleton · Jaw', 'Skeleton · Head']);
		expect(jaw.height).toBe(TAKE_HEAD_H + 2 * KEY_ROW_H + 4);
		expect(lanes.find((l) => l.key === 'take:waiting')!.rows).toEqual(['']);
		// Placed by its own span moved by its offset: a take that arrived late plays early.
		const placed = items.find((i) => i.ref.id === 'jaw')!;
		expect([placed.fromMs, placed.toMs]).toEqual([400, 3_900]);
	});

	it('answer which lane is at a height, and nothing over the ruler', () => {
		const layout = layOut(scare());
		expect(laneAt(layout, 5)).toBeNull();
		expect(laneAt(layout, RULER_H + 1)?.key).toBe('markers');
		expect(laneAt(layout, layout.height + 50)).toBeNull();
	});
});

describe('the pointer', () => {
	const layout = layOut(scare());
	const middleOf = (id: string) => {
		const item = layout.items.find((i) => i.ref.id === id)!;
		const r = rectOf(item, layout, view, width);
		return { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
	};

	it('picks up a clip by its body and an event by its line', () => {
		const laugh = middleOf('laugh');
		expect(hit(layout, view, width, laugh.x, laugh.y)?.ref).toEqual({ kind: 'timeline-clip', id: 'laugh' });
		const go = middleOf('e2');
		expect(hit(layout, view, width, go.x + 3, go.y)?.ref).toEqual({ kind: 'timeline-event', id: 'e2' });
		expect(hit(layout, view, width, go.x + 40, go.y)).toBeNull();
	});

	it('finds a tempo segment on the ruler', () => {
		expect(hit(layout, view, width, 2, 10)?.ref.kind).toBe('timeline-segment');
	});

	it('picks up a take by any of its rows', () => {
		const lane = layout.lanes.find((l) => l.key === 'take:jaw')!;
		const x = xOf(2_000, view, width);
		expect(hit(layout, view, width, x, lane.top + lane.height - 4)?.ref.id).toBe('jaw');
	});

	it('turns an x into a position and never one before the top', () => {
		expect(positionAt(500, view, width)).toBe(5_000);
		expect(positionAt(-30, view, width)).toBe(0);
	});
});

describe('a rubber band', () => {
	it('takes everything it touches, even partly', () => {
		const layout = layOut(scare());
		const speakers = layout.lanes.find((l) => l.key === 'audio:station:4')!;
		// Across the speaker lane from 3 s to 3.5 s: the laugh and the bed both pass under.
		const band = { left: xOf(3_000, view, width), right: xOf(3_500, view, width), top: speakers.top + 1, bottom: speakers.top + speakers.height - 1 };
		expect(inBand(layout, view, width, band).map((r) => r.id).sort()).toEqual(['bed', 'laugh']);
	});

	it('works dragged in either direction', () => {
		const layout = layOut(scare());
		const events = layout.lanes.find((l) => l.key === 'events')!;
		const band = { left: xOf(8_000, view, width), right: xOf(6_000, view, width), top: events.top + events.height - 2, bottom: events.top + 2 };
		expect(inBand(layout, view, width, band)).toEqual([{ kind: 'timeline-event', id: 'e2' }]);
	});
});

describe('a drag of several things', () => {
	const timeline = {
		clips: [
			{ id: 'laugh', start_ms: 300 },
			{ id: 'bed', start_ms: 2_000 }
		],
		events: [
			{ id: 'e1', at_ms: 0 },
			{ id: 'e2', at_ms: 7_000 }
		],
		markers: [{ id: 'm1', at_ms: 1_000 }],
		grid: [{ at_ms: 0 }, { at_ms: 5_000 }],
		tracks: [{ id: 'jaw', offset_ms: 100 }]
	};

	it('moves each by the same amount and writes only the columns it touched', () => {
		const refs = [
			{ kind: 'timeline-clip' as const, id: 'bed' },
			{ kind: 'timeline-event' as const, id: 'e2' }
		];
		const grabbed = grabbedFrom(timeline, refs, segmentIndex);
		const out = moved(timeline, grabbed, 500, segmentIndex);
		expect(Object.keys(out).sort()).toEqual(['clips', 'events']);
		expect(out.clips!.map((c) => c.start_ms)).toEqual([300, 2_500]);
		expect(out.events!.map((e) => e.at_ms)).toEqual([0, 7_500]);
	});

	it('is measured from where the drag began, so frames do not add up', () => {
		const grabbed = grabbedFrom(timeline, [{ kind: 'timeline-marker', id: 'm1' }], segmentIndex);
		const once = moved(timeline, grabbed, 200, segmentIndex);
		const again = moved({ ...timeline, markers: once.markers! }, grabbed, 300, segmentIndex);
		expect(again.markers![0].at_ms).toBe(1_300);
	});

	it('stops at the top as a group, rather than squashing everything onto it', () => {
		const refs = [
			{ kind: 'timeline-clip' as const, id: 'laugh' },
			{ kind: 'timeline-clip' as const, id: 'bed' }
		];
		const out = moved(timeline, grabbedFrom(timeline, refs, segmentIndex), -1_000, segmentIndex);
		expect(out.clips!.map((c) => c.start_ms)).toEqual([0, 1_700]);
	});

	it('keeps events in time order after a move past one another', () => {
		const out = moved(timeline, grabbedFrom(timeline, [{ kind: 'timeline-event', id: 'e1' }], segmentIndex), 8_000, segmentIndex);
		expect(out.events!.map((e) => e.id)).toEqual(['e2', 'e1']);
	});

	it('moves a tempo segment by its index, and a take by its offset', () => {
		const refs = [
			{ kind: 'timeline-segment' as const, id: segmentId('t', 1) },
			{ kind: 'timeline-take' as const, id: 'jaw' }
		];
		const out = moved(timeline, grabbedFrom(timeline, refs, segmentIndex), -50, segmentIndex);
		expect(out.grid!.map((s) => s.at_ms)).toEqual([0, 4_950]);
		expect(out.tracks![0].offset_ms).toBe(50);
	});

	it('lets a take start early, but not before the top', () => {
		// The take's first point is at 300 in its own time, so -300 is as early as it goes.
		const grabbed = grabbedFrom(timeline, [{ kind: 'timeline-take', id: 'jaw' }], segmentIndex);
		const out = moved(timeline, grabbed, -10_000, segmentIndex, () => 300);
		expect(out.tracks![0].offset_ms).toBe(-300);
	});

	it('removes what was picked, a segment by its index', () => {
		const out = removed(
			timeline,
			[
				{ kind: 'timeline-segment', id: segmentId('t', 0) },
				{ kind: 'timeline-clip', id: 'laugh' }
			],
			segmentIndex
		);
		expect(out.grid).toEqual([{ at_ms: 5_000 }]);
		expect(out.clips!.map((c) => c.id)).toEqual(['bed']);
		expect(out.events).toBeUndefined();
	});
});

describe('names', () => {
	it('spells a segment as its timeline and its index, and reads both back', () => {
		const id = segmentId('a-b-c', 3);
		expect(segmentIndex(id)).toBe(3);
		expect(segmentTimeline(id)).toBe('a-b-c');
		expect(refKey({ kind: 'timeline-clip', id: 'x' })).toBe('timeline-clip:x');
	});

	it('fits the whole of a timeline, but never less than two seconds', () => {
		expect(fitAll(90_000)).toEqual({ startMs: 0, spanMs: 90_000 });
		expect(fitAll(10)).toEqual({ startMs: 0, spanMs: 2_000 });
	});
});
