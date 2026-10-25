import { describe, it, expect } from 'vitest';

import { rolling } from './tracks.js';
import type { Timeline, TimelineTrack } from '../generated/index.js';

/**
 * Which recordings are playing, and where each playhead is.
 *
 * The arithmetic is one line and it is guarded because it exists twice: here, and in
 * `playing_tracks` on the station. A page that applied a take's nudge the other way
 * round would run the screen ahead of the lamps by exactly that many milliseconds,
 * which is the kind of disagreement nobody notices until a show.
 */

const aTrack = (over: Partial<TimelineTrack> = {}): TimelineTrack => ({
	id: 'track',
	name: 'Take 1',
	asset: 'abc',
	input_id: null,
	offset_ms: 0,
	enabled: true,
	fixture_id: null,
	key: null,
	from_ms: null,
	to_ms: null,
	...over
});

const aTimeline = (over: Partial<Timeline> = {}): Timeline => ({
	id: 'timeline',
	name: 'Song',
	clips: [],
	detected: null,
	source: 'Internal',
	grid: [],
	markers: [],
	events: [],
	tracks: [aTrack()],
	speed_master: null,
	node_id: null,
	end: null,
	running: true,
	anchor_ms: 1000,
	position_at_anchor_ms: 500,
	rate: 1,
	recording: null,
	...over
});

describe('which recordings are rolling', () => {
	it('is nothing at all while the timeline is stopped', () => {
		expect(rolling([aTimeline({ running: false })])).toEqual([]);
	});

	it('leaves out a take somebody switched off', () => {
		expect(rolling([aTimeline({ tracks: [aTrack({ enabled: false })] })])).toEqual([]);
	});

	it('is one composition per timeline, on the timeline’s own transport', () => {
		const [one] = rolling([aTimeline({ rate: 2, tracks: [aTrack(), aTrack({ id: 'b', asset: 'def', offset_ms: 40 })] })]);
		expect(one).toMatchObject({ id: 'timeline:timeline', anchorMs: 1000, positionMs: 500, rate: 2, levels: true });
		// The nudge is the composition's, row by row, not the transport's.
		expect(one.parts.map((p) => [p.sha, p.offset_ms])).toEqual([
			['abc', 0],
			['def', 40]
		]);
	});

	it('leaves the sequence levels to their faders while the timeline records a pass', () => {
		const recording = aTimeline({ recording: { Programmer: { release: true } } });
		expect(rolling([recording])[0].levels).toBe(false);
	});
});
