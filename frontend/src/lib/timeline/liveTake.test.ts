import { describe, expect, it } from 'vitest';

import type { ProgrammerValue } from '$lib/generated/index.js';
import { LivePass } from './liveTake.js';

const held = (level: number): ProgrammerValue =>
	({ id: 'e', fixture_id: 'f', parameter_kind: 'Intensity', value: { type: 'Float', value: level }, effect: null, preset: null, locked: false }) as ProgrammerValue;

describe('a take as it is recorded', () => {
	it('records from the first change, not the baseline', () => {
		const pass = new LivePass();
		pass.observe(0, [held(0.5)]);
		expect(pass.keys.size).toBe(0);
		pass.observe(100, [held(0.7)]);
		expect([...pass.keys.values()][0].points).toEqual([[100, 0.7]]);
	});

	it('lets a key go when it leaves the programmer, as a flash key does', () => {
		const pass = new LivePass();
		pass.observe(0, []);
		pass.observe(1000, [held(1)]);
		pass.observe(1400, []);
		expect([...pass.keys.values()][0].points).toEqual([
			[1000, 1],
			[1400, null]
		]);
		expect(pass.span()).toEqual({ from: 1000, to: 1400 });
	});
});
