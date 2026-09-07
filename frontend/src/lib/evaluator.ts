/**
 * The console's evaluator, in the page.
 *
 * `crates/pult-render-wasm` compiled to `wasm32-unknown-unknown`: the same crate the
 * station links natively, so the browser and the lamps cannot disagree about what a
 * fade is worth at a moment. There is no TypeScript translation of the arithmetic
 * anywhere, and the reason is that the surface is large enough — easings, curves, step
 * lists, spread, phase, direction, width, master rates, priority, home fallback, split
 * fades — that two of it would drift, and the visible form of that drift is the screen
 * showing something the rig is not doing.
 *
 * Loaded on demand rather than imported: a page that never shows a light should not
 * fetch a hundred kilobytes of arithmetic, and the module cannot be loaded at all
 * during server-side rendering.
 */

import type { ParameterValue } from './generated/index.js';
import type { DrivenBy } from './driving.js';

/** How a packed answer says what kind of value it is. See `pult-render-wasm`. */
const NONE = 0;
const FLOAT = 1;
const INT = 2;
const BOOL = 3;
const COLOR = 4;
const TEXT = 5;
/** Four floats per parameter: a tag and up to three components. */
const STRIDE = 4;

/**
 * What the generated module gives us.
 *
 * Described here rather than imported: the module is built by
 * `scripts/build-evaluator.sh` and is not in the tree until it has been, so a type
 * that reached into it would make `svelte-check` depend on a build step.
 */
type Instance = {
	set_driving(driving: unknown): void;
	set_one(key: string, drivenBy: unknown): void;
	forget_fixture(fixtureId: string): void;
	watch(keys: unknown): void;
	evaluate(nowMs: number): Float32Array;
	text(key: string, nowMs: number): string | undefined;
	load_track(sha: string, bytes: Uint8Array): void;
	play_track(sha: string, anchorMs: number, positionAtAnchorMs: number, rate: number): void;
	stop_track(sha: string): void;
	forget_track(sha: string): void;
};

type Wasm = {
	default: () => Promise<unknown>;
	Evaluator: new () => Instance;
};

let instance: Instance | null = null;
let loading: Promise<Instance | null> | null = null;

/**
 * Load the evaluator, once.
 *
 * Answers `null` where it cannot be loaded at all — during server rendering, or in a
 * test runner with no wasm — and every caller has to cope with that, because the
 * alternative is a page that shows plausible numbers it did not compute.
 */
export function loadEvaluator(): Promise<Instance | null> {
	if (loading) return loading;
	loading = (async () => {
		try {
			const wasm = (await import('./evaluator/pult_render_wasm.js')) as unknown as Wasm;
			await wasm.default();
			instance = new wasm.Evaluator();
			return instance;
		} catch (e) {
			console.warn('[pult] the evaluator could not be loaded', e);
			return null;
		}
	})();
	return loading;
}

/** The loaded evaluator, or `null` while it is not. */
export const evaluator = (): Instance | null => instance;

/** Hand over everything driving the rig. Called when the show changes, not per frame. */
export function setDriving(driving: Record<string, DrivenBy>): void {
	instance?.set_driving(driving);
}

/** Replace what is driving one parameter, leaving the rest alone. */
export function setOneDriving(key: string, drivenBy: DrivenBy | null): void {
	instance?.set_one(key, drivenBy);
}

/**
 * Hand over a recording's bytes, decoded once.
 *
 * The same bytes the station reads, through the same parser — there is no JSON form of
 * a track anywhere, deliberately, for the reason there is no TypeScript form of the
 * evaluator. A recording is the largest thing that has ever driven a parameter and two
 * readers of it would disagree exactly where it is longest.
 */
export function loadTrack(sha: string, bytes: Uint8Array): void {
	instance?.load_track(sha, bytes);
}

/**
 * This recording is running, anchored here.
 *
 * The transport is the timeline's own row, so a page and a connector work out the same
 * playhead from the same four numbers with nothing ticking on either side.
 */
export function playTrack(
	sha: string,
	anchorMs: number,
	positionAtAnchorMs: number,
	rate: number
): void {
	instance?.play_track(sha, anchorMs, positionAtAnchorMs, rate);
}

/** This recording is no longer running. The bytes stay loaded. */
export function stopTrack(sha: string): void {
	instance?.stop_track(sha);
}

/** The show no longer carries this recording at all. */
export function forgetTrack(sha: string): void {
	instance?.forget_track(sha);
}

/** Say which parameters are being shown, and in what order the answers come back. */
export function watch(keys: string[]): void {
	instance?.watch(keys);
}

/**
 * Unpack one answer.
 *
 * `undefined` means the evaluator was given nothing that could place this parameter —
 * which a caller shows as a gap rather than as a zero, because for a dimmer a zero is
 * a decision to turn the light off.
 */
export function unpack(packed: Float32Array, at: number): ParameterValue | undefined {
	const base = at * STRIDE;
	switch (packed[base]) {
		case FLOAT:
			return { type: 'Float', value: packed[base + 1] };
		case INT:
			return { type: 'Int', value: Math.round(packed[base + 1]) };
		case BOOL:
			return { type: 'Bool', value: packed[base + 1] !== 0 };
		case COLOR:
			return {
				type: 'Color',
				// The packed frame is four floats per parameter and an override map does
				// not fit in one; `color_overrides` is the export that answers for those.
				value: { r: packed[base + 1], g: packed[base + 2], b: packed[base + 3], overrides: {} }
			};
		case TEXT:
			// A line of text does not fit in four floats; ask for it by name.
			return undefined;
		case NONE:
		default:
			return undefined;
	}
}

export { STRIDE, TEXT };
