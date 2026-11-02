import { describe, expect, it } from 'vitest';

import { decodeChunk, placeChunk } from './monitor.js';

/** A chunk as `pult_audio::monitor::encode` writes one. */
function chunk(atMs: number, samples: number[], channels = 2, loading = false): ArrayBuffer {
	const frames = samples.length / channels;
	const buf = new ArrayBuffer(26 + samples.length * 2);
	const view = new DataView(buf);
	'PLMN'.split('').forEach((c, i) => view.setUint8(i, c.charCodeAt(0)));
	view.setUint16(4, 1, true);
	view.setUint16(6, channels, true);
	view.setUint32(8, 48_000, true);
	view.setFloat64(12, atMs, true);
	view.setUint32(20, frames, true);
	view.setUint16(24, loading ? 1 : 0, true);
	samples.forEach((s, i) => view.setInt16(26 + i * 2, Math.round(s * 32767), true));
	return buf;
}

describe('a monitor chunk', () => {
	it('reads back the moment it belongs to and its channels apart', () => {
		const read = decodeChunk(chunk(1234.5, [0.5, -0.5, 1, 0]))!;
		expect(read.atMs).toBe(1234.5);
		expect(read.loading).toBe(false);
		expect(decodeChunk(chunk(0, [0, 0], 2, true))!.loading).toBe(true);
		expect(read.frames).toBe(2);
		expect(read.channels[0][1]).toBeCloseTo(1, 3);
		expect(read.channels[1][0]).toBeCloseTo(-0.5, 3);
	});

	it('is refused when it is not one', () => {
		expect(decodeChunk(new ArrayBuffer(10))).toBeNull();
		const bad = chunk(0, [0, 0]);
		new DataView(bad).setUint8(0, 88);
		expect(decodeChunk(bad)).toBeNull();
		expect(decodeChunk(chunk(0, [0, 0]).slice(0, 27))).toBeNull();
	});
});

describe('where a chunk plays', () => {
	it('is laid end to end through clock wobble, and goes where it belongs after a jump', () => {
		expect(placeChunk(10.005, 10, 9)).toBe(10);
		expect(placeChunk(12, 10, 9)).toBe(12);
		expect(placeChunk(9.5, null, 9)).toBe(9.5);
	});

	it('is dropped when its moment has passed', () => {
		expect(placeChunk(9, 8, 9.5)).toBeNull();
	});
});
