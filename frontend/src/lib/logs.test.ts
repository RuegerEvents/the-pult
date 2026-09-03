import { describe, it, expect } from 'vitest';

import type { LogLevel, LogLine } from './generated/index.js';
import { LogBuffer, Throttle, passes } from './logs.js';

const ROOF = '11111111-1111-1111-1111-111111111111';
const BOOTH = '22222222-2222-2222-2222-222222222222';

function line(nodeId: string, seq: number, atMs = 1000 + seq, level: LogLevel = 'info'): LogLine {
	return {
		seq,
		node_id: nodeId,
		at_ms: atMs,
		level,
		target: 'test',
		source: { kind: 'station' },
		message: `line ${seq}`
	};
}

describe('levels', () => {
	it('keeps everything quieter than the threshold', () => {
		expect(passes('error', 'warn')).toBe(true);
		expect(passes('warn', 'warn')).toBe(true);
		expect(passes('info', 'warn')).toBe(false);
		expect(passes('trace', 'debug')).toBe(false);
	});
});

describe('merging the backlog with the live stream', () => {
	it('takes a line once however many times it arrives', () => {
		const buffer = new LogBuffer();
		// The backlog, then a live batch overlapping it — which is what happens
		// whenever a line is written between the RPC going out and its answer.
		expect(buffer.add([line(ROOF, 1), line(ROOF, 2)])).toBe(2);
		expect(buffer.add([line(ROOF, 2), line(ROOF, 3)])).toBe(1);
		expect(buffer.size).toBe(3);
	});

	it('tells two stations apart by more than their sequence numbers', () => {
		const buffer = new LogBuffer();
		buffer.add([line(ROOF, 1), line(BOOTH, 1)]);
		expect(buffer.size).toBe(2);
		expect(buffer.stations().sort()).toEqual([ROOF, BOOTH].sort());
	});

	it('orders by the clock across stations and by seq within one', () => {
		const buffer = new LogBuffer();
		// Two lines in the same millisecond from one station: only seq separates
		// them, and without the tie-break they would shuffle on every insert.
		buffer.add([line(ROOF, 5, 100), line(ROOF, 4, 100), line(BOOTH, 9, 50)]);
		const seqs = buffer
			.entries()
			.filter((e) => e.kind === 'line')
			.map((e) => (e.kind === 'line' ? e.line.seq : 0));
		expect(seqs).toEqual([9, 4, 5]);
	});
});

describe('gaps', () => {
	it('says how many lines went missing when a station skips', () => {
		const buffer = new LogBuffer();
		buffer.add([line(ROOF, 1), line(ROOF, 1206)]);
		const gaps = buffer.entries().filter((e) => e.kind === 'gap');
		expect(gaps).toHaveLength(1);
		expect(gaps[0]).toMatchObject({ nodeId: ROOF, missing: 1204 });
	});

	it('does not call the start of what it holds a gap', () => {
		// Every panel opened mid-show starts partway through a station's numbering.
		// That is a beginning, not a hole, and claiming otherwise would put a
		// "4,000 lines missing" marker at the top of every fresh panel.
		const buffer = new LogBuffer();
		buffer.add([line(ROOF, 4000), line(ROOF, 4001)]);
		expect(buffer.entries().filter((e) => e.kind === 'gap')).toHaveLength(0);
	});

	it('does not invent a gap out of lines the level filter hid', () => {
		const buffer = new LogBuffer();
		buffer.add([
			line(ROOF, 1, 1001, 'error'),
			line(ROOF, 2, 1002, 'debug'),
			line(ROOF, 3, 1003, 'error')
		]);
		// Turning the panel down to errors hides seq 2 — which is not it going
		// missing, and a marker there would send somebody looking for nothing.
		const shown = buffer.entries('error');
		expect(shown.filter((e) => e.kind === 'gap')).toHaveLength(0);
		expect(shown.filter((e) => e.kind === 'line')).toHaveLength(2);
	});

	it('counts a gap per station rather than across the merged view', () => {
		const buffer = new LogBuffer();
		buffer.add([line(ROOF, 1, 10), line(BOOTH, 500, 11), line(ROOF, 2, 12)]);
		// The roof went 1, 2 with the booth's line in between. Nothing is missing.
		expect(buffer.entries().filter((e) => e.kind === 'gap')).toHaveLength(0);
	});
});

describe('filtering', () => {
	it('picks out one plugin by its field rather than by its message', () => {
		const buffer = new LogBuffer();
		buffer.add([
			{ ...line(ROOF, 1), source: { kind: 'plugin', id: 'command-line' } },
			{ ...line(ROOF, 2), source: { kind: 'plugin', id: 'natural-language-control' } },
			// A station line whose text merely *looks* like a plugin's, which is
			// what a prefix-matching filter would have been fooled by.
			{ ...line(ROOF, 3), message: '[plugin:command-line] not really' }
		]);
		const shown = buffer.entries(undefined, (s) => s.kind === 'plugin' && s.id === 'command-line');
		expect(shown).toHaveLength(1);
		expect(shown[0].kind === 'line' && shown[0].line.seq).toBe(1);
	});
});

describe('bounds', () => {
	it('drops the oldest and does not let a backlog fetch resurrect them', () => {
		const buffer = new LogBuffer(3);
		buffer.add([line(ROOF, 1), line(ROOF, 2), line(ROOF, 3), line(ROOF, 4)]);
		expect(buffer.size).toBe(3);

		// Scrolling back asks again and gets seq 1 in the answer. It must not come
		// back, or the panel would grow without bound while somebody scrolls.
		expect(buffer.add([line(ROOF, 1)])).toBe(0);
		expect(buffer.size).toBe(3);
	});
});

describe('what a browser reports about itself', () => {
	it('reports the first of a burst and counts the rest into the next one', () => {
		let now = 0;
		const throttle = new Throttle(1000, () => now);

		expect(throttle.admit('RigPanel: mesh is null')).toEqual({ count: 1 });
		// A render loop throwing every frame: seen, counted, not sent.
		for (let i = 0; i < 42; i++) expect(throttle.admit('RigPanel: mesh is null')).toBeNull();

		now = 1500;
		expect(throttle.admit('RigPanel: mesh is null')).toEqual({ count: 43 });
	});

	it('counts each distinct fault separately', () => {
		let now = 0;
		const throttle = new Throttle(1000, () => now);
		expect(throttle.admit('one')).toEqual({ count: 1 });
		expect(throttle.admit('two')).toEqual({ count: 1 });
		expect(throttle.admit('one')).toBeNull();
	});

	it('starts counting again after a report', () => {
		let now = 0;
		const throttle = new Throttle(1000, () => now);
		throttle.admit('x');
		throttle.admit('x');
		now = 1001;
		expect(throttle.admit('x')).toEqual({ count: 2 });
		now = 2002;
		expect(throttle.admit('x')).toEqual({ count: 1 });
	});
});
