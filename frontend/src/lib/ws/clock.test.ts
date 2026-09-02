import { describe, it, expect, beforeEach, vi, afterEach, beforeAll } from 'vitest';
import { readFileSync } from 'node:fs';
import { ClockSync, consoleNow, clockOffset, forgetOffset } from './clock.js';
import { unpack } from '../evaluator.js';

/**
 * A station whose clock is deliberately wrong relative to this browser's, answering
 * after a delay that can be different in each direction.
 *
 * Asymmetry is the honest case: a request that queues on the way out and comes back
 * promptly gives a midpoint that is late by the difference. It is also why the
 * estimate keeps the shortest round trip rather than averaging them.
 */
function aStation(skewMs: number, delays: number[]) {
	const asked: number[] = [];
	let sync: ClockSync;
	let next = 0;
	const transport = {
		ask: (sentAt: number) => {
			asked.push(sentAt);
			const rtt = delays[Math.min(next, delays.length - 1)];
			next += 1;
			// The station read its clock somewhere inside the round trip; put it in
			// the middle, then let the caller's own arithmetic find it again.
			const arrivesAt = sentAt + rtt;
			sync.answered(sentAt, sentAt + rtt / 2 + skewMs, arrivesAt);
		},
	};
	sync = new ClockSync(transport);
	return { sync, asked };
}

describe('the station clock, as a browser sees it', () => {
	beforeEach(() => {
		forgetOffset();
		vi.useFakeTimers();
	});
	afterEach(() => {
		vi.useRealTimers();
	});

	it('says nothing at all before it has been told', () => {
		expect(consoleNow()).toBeNull();
		expect(clockOffset()).toBeNull();
	});

	it('establishes an offset across a connection with a delay in it', () => {
		const { sync } = aStation(4_000, [40]);
		sync.start();
		vi.advanceTimersByTime(1);

		const offset = clockOffset();
		expect(offset).not.toBeNull();
		expect(offset!.offsetMs).toBeCloseTo(4_000, 0);
		expect(offset!.rttMs).toBe(40);
		sync.stop();
	});

	it('keeps the sample that had the shortest round trip', () => {
		// The slow ones are the ones whose midpoint is least likely to be the middle
		// of anything, so a mean would be worse than the best of five.
		const { sync } = aStation(1_500, [400, 380, 12, 350, 420]);
		sync.start();
		vi.advanceTimersByTime(10);

		expect(clockOffset()!.rttMs).toBe(12);
		expect(clockOffset()!.offsetMs).toBeCloseTo(1_500, 0);
		sync.stop();
	});

	it('places a moment on the station clock once it knows the offset', () => {
		const { sync } = aStation(-7_250, [20]);
		sync.start();
		vi.advanceTimersByTime(1);

		const now = consoleNow();
		expect(now).not.toBeNull();
		expect(now! - Date.now()).toBeCloseTo(-7_250, 0);
		sync.stop();
	});

	it('re-establishes rather than drifting when a clock steps', () => {
		const { sync } = aStation(0, [20]);
		sync.start();
		vi.advanceTimersByTime(10);
		expect(clockOffset()!.offsetMs).toBeCloseTo(0, 0);
		sync.stop();

		// The browser's clock jumps two minutes — NTP correcting a drift, or a laptop
		// waking up. The station's did not move, so the offset now has to.
		const stepped = aStation(-120_000, [20]);
		stepped.sync.start();
		vi.advanceTimersByTime(10);

		expect(clockOffset()!.offsetMs).toBeCloseTo(-120_000, 0);
		stepped.sync.stop();
	});

	it('goes on asking on its own, so an offset is maintained rather than taken once', () => {
		const { sync, asked } = aStation(0, [20]);
		sync.start();
		vi.advanceTimersByTime(10);
		const afterTheFirstEstimate = asked.length;

		vi.advanceTimersByTime(60_000);
		expect(asked.length).toBeGreaterThan(afterTheFirstEstimate);
		sync.stop();
	});

	it('forgets what it knew when the connection goes', () => {
		const { sync } = aStation(3_000, [20]);
		sync.start();
		vi.advanceTimersByTime(1);
		expect(consoleNow()).not.toBeNull();

		sync.stop();
		forgetOffset();
		expect(consoleNow()).toBeNull();
	});
});


/**
 * The property the offset exists for: a browser whose own clock is wrong still draws
 * the rig where the station has it.
 *
 * A fade is anchored in console milliseconds, and nothing stores what it is worth. So
 * a page that evaluated against an unadjusted `Date.now()` would run every fade out by
 * however wrong its clock is — silently, because each value on its own is plausible.
 * This is what makes that a test rather than an argument.
 */
describe('a browser whose clock is wrong', () => {
	type Built = {
		initSync: (opts: { module: BufferSource }) => unknown;
		Evaluator: new () => {
			set_driving(driving: unknown): void;
			watch(keys: unknown): void;
			evaluate(nowMs: number): Float32Array;
		};
	};
	let built: Built | null = null;

	beforeAll(async () => {
		try {
			const module = (await import('../evaluator/pult_render_wasm.js')) as unknown as Built;
			const bytes = readFileSync(
				new URL('../evaluator/pult_render_wasm_bg.wasm', import.meta.url)
			);
			module.initSync({ module: bytes });
			built = module;
		} catch {
			built = null;
		}
	});

	/** A four-second fade from dark to full, anchored on the station's clock. */
	const STATION_T0 = 1_700_000_000_000;
	const driving = {
		'spot/Intensity': {
			fade: {
				from: { type: 'Float', value: 0 },
				to: { type: 'Float', value: 1 },
				t0: STATION_T0,
				duration_ms: 4_000,
				easing: 'Linear',
				cue_id: '00000000-0000-0000-0000-000000000000'
			}
		}
	};

	const levelAt = (evaluator: InstanceType<Built['Evaluator']>, at: number) => {
		const value = unpack(evaluator.evaluate(at), 0);
		return value?.type === 'Float' ? value.value : null;
	};

	beforeEach(() => forgetOffset());

	it('still draws the fade where the station has it', () => {
		expect(built).not.toBeNull();
		if (!built) return;
		const evaluator = new built.Evaluator();
		evaluator.set_driving(driving);
		evaluator.watch(['spot/Intensity']);

		// This browser is three and a half minutes fast. Its own clock says one thing
		// and the station's says another; only one of them anchors the fade.
		const SKEW = 210_000;
		const local = STATION_T0 + SKEW;
		vi.spyOn(Date, 'now').mockImplementation(() => local);

		const sync = new ClockSync({
			ask: (sentAt) => sync.answered(sentAt, STATION_T0 + 20, sentAt + 40)
		});
		vi.useFakeTimers();
		sync.start();
		vi.advanceTimersByTime(10);
		vi.useRealTimers();
		sync.stop();

		// Every one of these is a moment on *this browser's* clock, placed on the
		// station's before it is asked about.
		for (const after of [0, 1_000, 2_000, 3_000, 4_000]) {
			vi.spyOn(Date, 'now').mockImplementation(() => local + after);
			const at = consoleNow();
			expect(at).not.toBeNull();
			// What the station would compute for the same wall-clock instant.
			const want = Math.min(1, after / 4_000);
			expect(levelAt(evaluator, at!)).toBeCloseTo(want, 3);
			// And what it would have drawn without the offset, which is the bug. Only
			// while the fade is still running: once it has landed both clocks are past
			// the end of it and agree by accident, which is exactly why this failure
			// mode is silent.
			if (after < 4_000) {
				expect(levelAt(evaluator, local + after)).not.toBeCloseTo(want, 3);
			}
		}
		vi.restoreAllMocks();
	});

	it('draws nothing at all before it knows the offset', () => {
		expect(consoleNow()).toBeNull();
	});
});
