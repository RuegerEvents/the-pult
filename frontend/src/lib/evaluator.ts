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

import type { ParameterValue, RunningEffect, Shape, TimelineFlash } from './generated/index.js';
import type { DrivenBy } from './driving.js';
import type { NodeEffect } from './stage.js';

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
	compose_track(id: string, parts: unknown, flashes: unknown): boolean;
	play_track(sha: string, anchorMs: number, positionAtAnchorMs: number, rate: number, levels: boolean): void;
	stop_track(sha: string): void;
	forget_track(sha: string): void;
	recording(nowMs: number): string[];
};

type Wasm = {
	default: () => Promise<unknown>;
	Evaluator: new () => Instance;
	effect_values(effects: unknown, nowMs: number): Float32Array;
	shape_level(shape: unknown, width: number, traveled: number, salt: number): number;
	track_outline(bytes: Uint8Array, budget: number): KeyOutline[];
	track_points(bytes: Uint8Array, fixtureId: string, key: string): TrackPoint[];
	encode_key(fixtureId: string, key: string, points: TrackPoint[]): Uint8Array;
	pixel_effect(
		effect: string,
		color: Uint8Array,
		color2: Uint8Array,
		brightness: number,
		speed: number,
		size: number,
		reverse: boolean,
		mirror: boolean,
		segments: number,
		nowMs: number,
		count: number
	): Uint8Array;
};

/** One key of a recording at a glance — `pult_render::KeyOutline`. */
export type KeyOutline = {
	fixture_id: string;
	key: string;
	from_ms: number;
	to_ms: number;
	points: number;
	/** `[ms, level]`, ascending, thinned to the budget asked for. */
	levels: [number, number][];
};

let instance: Instance | null = null;
/** The module itself, for the two free functions the Effects panel previews with. */
let module: Wasm | null = null;
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
			module = wasm;
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

/**
 * What each of these effects is asserting at `nowMs`, packed like `evaluate`.
 *
 * For a preview: nothing here is driving the rig, so it never enters the evaluator's
 * own map. `null` while the evaluator is not loaded, and the preview draws nothing.
 */
export function effectValues(effects: RunningEffect[], nowMs: number): Float32Array | null {
	if (!module || effects.length === 0) return null;
	try {
		return module.effect_values(effects, nowMs);
	} catch (e) {
		console.warn('[pult] effect preview', e);
		return null;
	}
}

/**
 * One frame of a strip a node draws itself, as the node draws it: three bytes a pixel.
 *
 * The node's own effect (OpenHaunt doc 14), not an approximation of it: the wasm copy
 * of `oh_pixfx.c`, held to the C by `testdata/pixel-effects.json`. `null` while the
 * evaluator is not loaded, and the strip draws dark.
 */
export function pixelEffect(effect: NodeEffect, nowMs: number, count: number): Uint8Array | null {
	if (!module) return null;
	return module.pixel_effect(
		effect.effect,
		new Uint8Array(effect.color),
		new Uint8Array(effect.color2),
		effect.brightness,
		effect.speed,
		effect.size,
		effect.reverse,
		effect.mirror,
		effect.segments,
		nowMs,
		count
	);
}

/**
 * A shape's level after `traveled` cycles, for drawing its waveform, including the
 * two shapes that never repeat. `null` while the evaluator is not loaded.
 */
export function shapeLevel(shape: Shape, width: number, traveled: number, salt = 0): number | null {
	if (!module) return null;
	return module.shape_level(shape, width, traveled, salt >>> 0);
}

/**
 * A recording outlined for a timeline lane: per key, where it starts and stops and its
 * level along the way. The same parse of the same bytes the station plays from, so a
 * lane cannot draw a take the lamps would not do. `null` while the evaluator is not
 * loaded or the bytes are not a recording.
 */
export function trackOutline(bytes: Uint8Array, budget = 400): KeyOutline[] | null {
	if (!module) return null;
	try {
		return module.track_outline(bytes, budget);
	} catch (e) {
		console.warn('[pult] a recording could not be outlined', e);
		return null;
	}
}

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

/** One row of a timeline's recordings, for composing: a loaded sha and the row's fields. */
export type TrackPart = {
	sha: string;
	fixture_id: string | null;
	key: string | null;
	from_ms: number | null;
	to_ms: number | null;
	offset_ms: number;
};

/**
 * A timeline's rows composed into one recording under `id`, and its flashes over them,
 * by the station's own `pult_render::compose_with`. Whether every sha the rows name
 * was loaded.
 */
export function composeTrack(id: string, parts: TrackPart[], flashes: TimelineFlash[] = []): boolean {
	return instance?.compose_track(id, parts, flashes) ?? false;
}

/** One change point of a take, as the codec has it. A release says nothing from `ms` on. */
export type TrackPoint = { ms: number; value: ParameterValue; release?: boolean };

/** Every change point one recording holds for one key, read through the one codec. */
export async function trackPoints(bytes: Uint8Array, fixtureId: string, key: string): Promise<TrackPoint[] | null> {
	await loadEvaluator();
	return module ? module.track_points(bytes, fixtureId, key) : null;
}

/** A recording of one key with these points, as the bytes a take is stored in. */
export async function encodeKey(fixtureId: string, key: string, points: TrackPoint[]): Promise<Uint8Array | null> {
	await loadEvaluator();
	return module ? module.encode_key(fixtureId, key, points) : null;
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
	rate: number,
	/** Whether the sequence levels it recorded play: not while its timeline records. */
	levels = true
): void {
	instance?.play_track(sha, anchorMs, positionAtAnchorMs, rate, levels);
}

/** This recording is no longer running. The bytes stay loaded. */
export function stopTrack(sha: string): void {
	instance?.stop_track(sha);
}

/** The show no longer carries this recording at all. */
export function forgetTrack(sha: string): void {
	instance?.forget_track(sha);
}

/**
 * Which of the watched parameters a recording is asserting right now.
 *
 * The fixture sheet's one layer that cannot be read off a fixture row: a fade and an
 * effect are `live_fades` and `live_effects`, and a take is bytes that only ever cross
 * this boundary. Asked for rather than derived, and asked only while a sheet is open
 * and a timeline is running — a settled console never calls it.
 */
export function recordingKeys(nowMs: number): string[] {
	return instance?.recording(nowMs) ?? [];
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

/** A text parameter's value at a moment, which the packed frame has no room for. */
export function textOf(key: string, nowMs: number): string | undefined {
	return instance?.text(key, nowMs);
}
