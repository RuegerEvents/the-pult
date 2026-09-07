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
	...over
});

const aTimeline = (over: Partial<Timeline> = {}): Timeline => ({
	id: 'timeline',
	name: 'Song',
	audio: null,
	peaks: null,
	detected: null,
	source: 'Internal',
	grid: [],
	markers: [],
	events: [],
	tracks: [aTrack()],
	speed_master: null,
	node_id: null,
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

	it('carries the timeline’s own transport', () => {
		expect(rolling([aTimeline({ rate: 2 })])).toEqual([
			{ sha: 'abc', anchorMs: 1000, positionMs: 500, rate: 2 }
		]);
	});

	it('moves the playhead the other way from a take’s nudge', () => {
		// A take that arrived 40 ms late is played 40 ms earlier.
		expect(rolling([aTimeline({ tracks: [aTrack({ offset_ms: 40 })] })])[0].positionMs).toBe(460);
		expect(rolling([aTimeline({ tracks: [aTrack({ offset_ms: -40 })] })])[0].positionMs).toBe(540);
	});

	it('never runs the playhead negative, which samples nothing at all', () => {
		expect(rolling([aTimeline({ tracks: [aTrack({ offset_ms: 9999 })] })])[0].positionMs).toBe(0);
	});
});
