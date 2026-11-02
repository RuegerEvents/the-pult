/**
 * A timeline's sound for listening while programming, as arithmetic: what a chunk is,
 * and where in the page's audio clock it plays.
 *
 * The station renders a lead ahead of the show clock and says in each chunk which show
 * millisecond its first frame belongs to (`pult_audio::monitor`, the other reader of
 * this format). The page plays it at that moment by its own estimate of the station
 * clock, so what it hears is where the playhead it draws is. **A monitor and nothing
 * else**: nothing here reaches the show, and the show's sound stays the station's
 * device.
 */

/** One chunk, its channels apart, the way an `AudioBuffer` wants them. */
export type MonitorChunk = {
	atMs: number;
	sampleRate: number;
	frames: number;
	/** A file the chunk covers is still loading on the station, and is not in it. */
	loading: boolean;
	channels: Float32Array<ArrayBuffer>[];
};

const MAGIC = 'PLMN';
const VERSION = 1;
const HEADER = 26;
const LOADING = 1;

/** A chunk off the wire, or `null` for anything that is not one this page can read. */
export function decodeChunk(bytes: ArrayBuffer): MonitorChunk | null {
	if (bytes.byteLength < HEADER) return null;
	const view = new DataView(bytes);
	const magic = String.fromCharCode(view.getUint8(0), view.getUint8(1), view.getUint8(2), view.getUint8(3));
	if (magic !== MAGIC || view.getUint16(4, true) !== VERSION) return null;
	const count = view.getUint16(6, true);
	const sampleRate = view.getUint32(8, true);
	const atMs = view.getFloat64(12, true);
	const frames = view.getUint32(20, true);
	const loading = (view.getUint16(24, true) & LOADING) !== 0;
	if (count === 0 || bytes.byteLength < HEADER + frames * count * 2) return null;
	const channels = Array.from({ length: count }, () => new Float32Array(frames));
	let at = HEADER;
	for (let frame = 0; frame < frames; frame++) {
		for (let c = 0; c < count; c++) {
			channels[c][frame] = view.getInt16(at, true) / 32767;
			at += 2;
		}
	}
	return { atMs, sampleRate, frames, loading, channels };
}

/** How close to where the last chunk ended a chunk may be told to start and still be
 *  laid end to end with it, in seconds: clock-estimate jitter, not a jump. */
export const SEAM_S = 0.02;
/** How soon after now a chunk can still be started, in seconds. */
export const TOO_LATE_S = 0.01;

/**
 * Where on the page's audio clock a chunk starts, or `null` when it is too late to.
 *
 * `wanted` is the show millisecond mapped onto the audio clock; `lastEnd` is where the
 * previous chunk ends. Within a seam of it the chunk is laid end to end, so the sound
 * is continuous however the clock estimate wobbles; further off, it is a locate or a
 * correction and the chunk goes where it belongs.
 */
export function placeChunk(wanted: number, lastEnd: number | null, now: number): number | null {
	const at = lastEnd !== null && Math.abs(wanted - lastEnd) < SEAM_S ? lastEnd : wanted;
	return at < now + TOO_LATE_S ? null : at;
}
