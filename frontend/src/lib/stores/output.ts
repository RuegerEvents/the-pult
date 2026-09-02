/**
 * What the rig is putting out, worked out in the page.
 *
 * Nothing stores this. The console keeps what is *driving* each parameter — the fade
 * or shape anchored in console milliseconds, the programmer over it, the home value
 * beneath — and every consumer evaluates for the moment and at the rate it needs. Here
 * that is animation frame rate, over the parameters some panel has said it is showing,
 * through the one evaluator the station also runs.
 *
 * Two things this must not do.
 *
 * It must not present a value before the browser knows the station's clock. What is
 * driving a parameter is anchored in *console* time, so a page evaluating against an
 * unadjusted `Date.now()` runs every fade out by however wrong its own clock is, and
 * does it silently. {@link Showing.at} is `null` until the offset exists, and every
 * lookup answers `null` with it — a visible gap rather than a plausible lie.
 *
 * And it must not pay for the rig it is not showing. A panel says what it is looking
 * at; the union of those is what a frame costs, so a rig of two thousand with forty on
 * screen evaluates forty.
 */

import { derived, readable, type Readable } from 'svelte/store';

import { drivingTheRig } from '../driving.js';
import { loadEvaluator, setDriving, unpack, watch } from '../evaluator.js';
import type { ParameterValue } from '../generated/index.js';
import { consoleNow } from '../ws/clock.js';
import { collection } from './show.js';

/** What the rig was doing at one moment. */
export type Showing = {
	/**
	 * The console millisecond these values are for, or `null` when this browser
	 * cannot yet place itself on the station's clock.
	 */
	readonly at: number | null;
	/** What one parameter is putting out, or `null` if it cannot be said. */
	value(fixtureId: string, key: string): ParameterValue | null;
};

/** What a page shows before it knows what time it is. */
export const NOTHING_YET: Showing = { at: null, value: () => null };

/**
 * A reading made from values that are already in hand.
 *
 * For anything holding a snapshot rather than driving the evaluator itself — a test,
 * or a panel showing a moment that has been captured rather than the moment it is.
 * Keyed the way everything else is: `"<fixture id>/<parameter key>"`.
 */
export function readingOf(values: Record<string, ParameterValue>, at = 0): Showing {
	return { at, value: (fixtureId, key) => values[`${fixtureId}/${key}`] ?? null };
}

/** Build a reading from a packed answer and the order it was asked in. */
export function reading(at: number, order: string[], packed: Float32Array): Showing {
	const values = new Map<string, ParameterValue>();
	order.forEach((key, index) => {
		const value = unpack(packed, index);
		if (value !== undefined) values.set(key, value);
	});
	return {
		at,
		value: (fixtureId, key) => values.get(`${fixtureId}/${key}`) ?? null
	};
}

// ── What is being looked at ───────────────────────────────────────────────────

const wanted = new Map<symbol, string[]>();
let order: string[] = [];
let orderIsStale = true;

/**
 * Say that these parameters are on screen, and keep saying it as that changes.
 *
 * A panel registers the whole of what it lists rather than what is precisely visible.
 * Under-reporting is the failure that matters — a row that stops being evaluated shows
 * a value from whenever it last was — so a cheap superset beats exact bookkeeping.
 */
export function watching(keys: string[]): { update(keys: string[]): void; stop(): void } {
	const id = Symbol('watching');
	wanted.set(id, keys);
	orderIsStale = true;
	return {
		update(next: string[]) {
			wanted.set(id, next);
			orderIsStale = true;
		},
		stop() {
			wanted.delete(id);
			orderIsStale = true;
		}
	};
}

function currentOrder(): string[] {
	if (!orderIsStale) return order;
	order = [...new Set([...wanted.values()].flat())];
	orderIsStale = false;
	watch(order);
	return order;
}

// ── The frame loop ────────────────────────────────────────────────────────────

/**
 * What is driving the rig, pushed into the evaluator whenever the show changes.
 *
 * Derived from the three collections that decide it and nothing else, so a fade in
 * progress — which changes none of them — costs this nothing at all.
 */
const driving = derived(
	[collection('fixtures'), collection('fixture_types'), collection('programmer_values')],
	([$fixtures, $types, $entries]) => drivingTheRig($fixtures, $types, $entries)
);

/**
 * The rig, evaluated once per animation frame.
 *
 * Frame rate rather than the console's old forty a second, which is the improvement:
 * motion is drawn as often as it can be seen, and the socket says nothing at all
 * while it happens.
 */
export const output: Readable<Showing> = readable<Showing>(NOTHING_YET, (set) => {
	let frame: number | null = null;
	let stopDriving: (() => void) | undefined;
	let live = true;

	loadEvaluator().then((instance) => {
		if (!live || !instance) return;
		stopDriving = driving.subscribe((rig) => {
			setDriving(rig);
			// A row appearing or going means the order may have grown; rebuild it on
			// the next frame rather than here, where several stores land at once.
			orderIsStale = true;
		});

		const draw = () => {
			frame = requestAnimationFrame(draw);
			const at = consoleNow();
			if (at === null) {
				set(NOTHING_YET);
				return;
			}
			const keys = currentOrder();
			if (keys.length === 0) {
				set({ at, value: () => null });
				return;
			}
			set(reading(at, keys, instance.evaluate(at)));
		};
		frame = requestAnimationFrame(draw);
	});

	return () => {
		live = false;
		if (frame !== null) cancelAnimationFrame(frame);
		stopDriving?.();
	};
});
