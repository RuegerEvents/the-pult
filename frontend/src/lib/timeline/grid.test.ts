import { describe, expect, it } from 'vitest';

import { gridFrom } from './grid.js';

describe("the detector's proposal as a tempo", () => {
	it('reads 128 bpm through 20 ms quantization, and a missed beat as two', () => {
		// 128 bpm is a beat every 468.75 ms; the model rounds each to 20 ms.
		const beats = Array.from({ length: 64 }, (_, i) => Math.round((i * 468.75) / 20) * 20).filter((_, i) => i !== 10);
		const [segment] = gridFrom({ beats_ms: beats, downbeats_ms: [beats[0], beats[4]] });
		expect(Math.abs(segment.bpm - 128)).toBeLessThan(0.3);
		expect(segment.beats_per_bar).toBe(4);
		expect(segment.at_ms).toBe(0);
	});

	it('has nothing to say about one beat', () => {
		expect(gridFrom({ beats_ms: [500], downbeats_ms: [] })).toEqual([]);
	});
});
