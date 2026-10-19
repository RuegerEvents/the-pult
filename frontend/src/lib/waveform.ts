/**
 * The picture of a song, and the arithmetic for dragging things against it.
 *
 * **Nothing here decodes audio.** The station reduces a file once — min and max per
 * bin, a hundred bins a second — and stores the reduction as its own asset; this reads
 * those bytes and works out where things go on a canvas. A tablet that decoded a
 * fifty-megabyte mp3 to draw four hundred columns would be doing the work of a console
 * to produce a thumbnail, which is the whole reason `pult_audio::peaks` exists.
 *
 * The codec is `pult_audio::peaks`'s, read here rather than shared, and that is worth
 * being honest about: it is the one format in this console with two implementations.
 * It earns the exception by being **eighteen bytes of header and a pair of `i16`s** —
 * there is no arithmetic in it to drift, unlike the evaluator or the track codec, which
 * are compiled twice precisely because they have.
 *
 * Everything else in this file is pure: pixels to milliseconds and back, what a view
 * may be zoomed and scrolled to, and what a dragged position snaps to.
 */

/** `PLPK`, the magic at the top of a peaks file. */
const MAGIC = 0x50_4c_50_4b;
const VERSION = 1;

/** A reduced waveform, as the station stored it. */
export type Peaks = {
	sampleRate: number;
	samplesPerBin: number;
	/** `[min, max]` per bin, in `[-1, 1]`. */
	bins: Float32Array;
	/** How long the audio is, in milliseconds. */
	durationMs: number;
};

/**
 * Read a peaks asset.
 *
 * `null` for anything that is not one, which is what a station running a newer version
 * would send: the version is checked before the bins are, so a panel draws nothing
 * rather than drawing noise.
 */
export function decodePeaks(bytes: ArrayBuffer): Peaks | null {
	if (bytes.byteLength < 18) return null;
	const view = new DataView(bytes);
	if (view.getUint32(0, false) !== MAGIC) return null;
	if (view.getUint16(4, true) !== VERSION) return null;

	const sampleRate = view.getUint32(6, true);
	const samplesPerBin = view.getUint32(10, true);
	const count = view.getUint32(14, true);
	if (bytes.byteLength < 18 + count * 4 || sampleRate === 0) return null;

	const bins = new Float32Array(count * 2);
	for (let i = 0; i < count; i++) {
		bins[i * 2] = view.getInt16(18 + i * 4, true) / 32767;
		bins[i * 2 + 1] = view.getInt16(18 + i * 4 + 2, true) / 32767;
	}
	return {
		sampleRate,
		samplesPerBin,
		bins,
		durationMs: Math.round((count * samplesPerBin * 1000) / sampleRate)
	};
}

/** What part of a song is on screen. */
export type View = {
	/** The leftmost millisecond. */
	startMs: number;
	/** How many milliseconds the whole width covers. */
	spanMs: number;
};

/** The shortest stretch anybody needs to see: two seconds across the panel. */
export const MIN_SPAN_MS = 2_000;

/**
 * Bring a view back inside the song.
 *
 * Two rules, and the second is the one that matters: the span never exceeds the whole
 * length, and the start is clamped so the *end* of the view cannot run past the end of
 * the song. A view scrolled past the end draws a blank canvas that looks exactly like a
 * file that failed to load.
 */
export function clampView(view: View, durationMs: number): View {
	const length = Math.max(durationMs, MIN_SPAN_MS);
	const spanMs = Math.min(Math.max(view.spanMs, MIN_SPAN_MS), length);
	const startMs = Math.min(Math.max(view.startMs, 0), length - spanMs);
	return { startMs, spanMs };
}

/** Where a position sits on the canvas. */
export function xOf(ms: number, view: View, width: number): number {
	return ((ms - view.startMs) / view.spanMs) * width;
}

/** And back: what a click on the canvas means. */
export function msOf(x: number, view: View, width: number): number {
	if (width <= 0) return view.startMs;
	return Math.round(view.startMs + (x / width) * view.spanMs);
}

/**
 * Zoom about a fixed point on the canvas.
 *
 * The millisecond under the pointer stays under the pointer, which is what makes a
 * wheel over a waveform feel like a map rather than like a slider. Anchoring to the
 * centre instead is the version everybody writes first and it walks the thing you were
 * looking at off the screen.
 */
export function zoomAt(view: View, durationMs: number, at: number, factor: number): View {
	const spanMs = view.spanMs * factor;
	const fraction = view.spanMs === 0 ? 0 : (at - view.startMs) / view.spanMs;
	return clampView({ startMs: at - fraction * spanMs, spanMs }, durationMs);
}

/** Something a dragged position can land on. */
export type SnapTarget = { atMs: number; what: 'beat' | 'bar' | 'marker' };

/**
 * What a position snaps to, or itself.
 *
 * `withinPx` rather than a number of milliseconds, because what an operator means by
 * "near" is a distance on the screen and not a distance in the song: at eight bars
 * across the panel a beat is forty pixels away and at a whole act it is one.
 *
 * A bar beats a beat and a marker beats both, so dragging near a chorus mark lands on
 * the chorus mark and not on whichever beat happens to be a pixel closer.
 */
export function snap(
	ms: number,
	targets: SnapTarget[],
	view: View,
	width: number,
	withinPx: number
): number {
	if (targets.length === 0 || width <= 0) return ms;
	const perMs = width / view.spanMs;
	const rank = { marker: 3, bar: 2, beat: 1 };
	let best: SnapTarget | null = null;
	let bestDistance = Infinity;
	for (const target of targets) {
		const distance = Math.abs(target.atMs - ms) * perMs;
		if (distance > withinPx) continue;
		if (
			best === null ||
			rank[target.what] > rank[best.what] ||
			(rank[target.what] === rank[best.what] && distance < bestDistance)
		) {
			best = target;
			bestDistance = distance;
		}
	}
	return best ? best.atMs : ms;
}

/**
 * The beat and bar lines a grid puts across a stretch of song.
 *
 * Worked out from the segments rather than stored, because a grid is four numbers and a
 * song is ten thousand beats — and because a beat's position has to move when somebody
 * drags the segment it belongs to.
 *
 * **Bounded by `limit`**, and that is not a nicety: at a whole act across the panel a
 * four-minute song has 480 beats and a festival set has 9 000, which is more lines than
 * pixels. The caller passes what it can draw, and a view too wide for beats is drawn
 * with bars alone.
 */
export function gridLines(
	grid: { at_ms: number; bpm: number; beats_per_bar: number }[],
	fromMs: number,
	toMs: number,
	limit = 4_000
): SnapTarget[] {
	const out: SnapTarget[] = [];
	const sorted = [...grid].sort((a, b) => a.at_ms - b.at_ms);
	for (let i = 0; i < sorted.length; i++) {
		const segment = sorted[i];
		if (segment.bpm <= 0) continue;
		const until = Math.min(sorted[i + 1]?.at_ms ?? Infinity, toMs);
		if (until <= fromMs) continue;
		const interval = 60_000 / segment.bpm;
		const perBar = Math.max(1, Math.round(segment.beats_per_bar));
		// Start at the first beat at or after the visible left edge rather than at the
		// segment's own start, so scrolling into the middle of a song is not a walk
		// through every beat before it.
		const first = Math.max(0, Math.ceil((fromMs - segment.at_ms) / interval));
		for (let beat = first; ; beat++) {
			const at = segment.at_ms + beat * interval;
			if (at >= until) break;
			out.push({ atMs: Math.round(at), what: beat % perBar === 0 ? 'bar' : 'beat' });
			if (out.length >= limit) return out;
		}
	}
	return out;
}

/** `1:04.20`, which is how a position is read off a stopwatch. */
export function clock(ms: number): string {
	const whole = Math.max(0, Math.floor(ms));
	const minutes = Math.floor(whole / 60_000);
	const seconds = Math.floor((whole % 60_000) / 1000);
	const hundredths = Math.floor((whole % 1000) / 10);
	return `${minutes}:${String(seconds).padStart(2, '0')}.${String(hundredths).padStart(2, '0')}`;
}

/** `1:04.2` back to milliseconds, and a bare number is already milliseconds. */
export function parsePosition(text: string): number | null {
	const trimmed = text.trim();
	if (!trimmed) return null;
	if (trimmed.includes(':')) {
		const [minutes, rest] = trimmed.split(':');
		const m = Number(minutes);
		const s = Number(rest);
		if (!Number.isFinite(m) || !Number.isFinite(s)) return null;
		return Math.round(m * 60_000 + s * 1000);
	}
	const ms = Number(trimmed);
	return Number.isFinite(ms) ? Math.round(ms) : null;
}

// ── Clips ─────────────────────────────────────────────────────────────────────

/** What of a clip is needed to place it: the schema's fields, by their own names. */
export type ClipPlace = {
	start_ms: number;
	offset_ms: number;
	length_ms: number | null;
	looped: boolean;
};

/**
 * Where a clip sits on the timeline, from its start to where it stops sounding.
 *
 * `fileMs` is how long the file is, which a browser only knows from the peaks — so a
 * clip whose waveform has not arrived yet is drawn as long as its own length says, or
 * as a short stub, rather than not drawn. A looped clip runs to `untilMs`, because it
 * goes round for as long as the timeline does.
 */
export function clipSpan(
	clip: ClipPlace,
	fileMs: number | null,
	untilMs: number
): { fromMs: number; toMs: number; regionMs: number } {
	const rest = fileMs === null ? null : Math.max(0, fileMs - clip.offset_ms);
	const regionMs =
		clip.length_ms !== null
			? rest === null
				? clip.length_ms
				: Math.min(clip.length_ms, rest)
			: (rest ?? 2_000);
	const toMs = clip.looped ? Math.max(untilMs, clip.start_ms + regionMs) : clip.start_ms + regionMs;
	return { fromMs: clip.start_ms, toMs, regionMs };
}

/**
 * A lane per clip that would otherwise overlap the one before it, in start order.
 *
 * Answered by index into the list handed in. Two clips that overlap in time are a
 * scream over a bed, which is the ordinary case, and drawn on one lane one of them
 * would be under the other and could not be grabbed.
 */
export function clipLanes(spans: { fromMs: number; toMs: number }[]): number[] {
	const order = spans.map((_, i) => i).sort((a, b) => spans[a].fromMs - spans[b].fromMs);
	const laneEnds: number[] = [];
	const lanes = new Array<number>(spans.length).fill(0);
	for (const i of order) {
		let lane = laneEnds.findIndex((end) => end <= spans[i].fromMs);
		if (lane === -1) {
			lane = laneEnds.length;
			laneEnds.push(spans[i].toMs);
		} else {
			laneEnds[lane] = spans[i].toMs;
		}
		lanes[i] = lane;
	}
	return lanes;
}

/** Which part of a clip a drag took hold of. */
export type ClipGrip = 'body' | 'front' | 'back';

/**
 * A clip after a drag, as the schema's fields.
 *
 * The **body** moves the clip and changes nothing about which part of the file plays.
 * The **front** trims: the clip starts later on the timeline *and* later in the file by
 * the same amount, so the sound under the edge stays where it was — which is what every
 * audio editor does and the only version that does not slide the song under the cursor.
 * The **back** sets how much plays. Neither trim may turn a clip inside out or reach
 * before the start of the file; a clip trimmed to nothing could never be grabbed again.
 */
export function dragClip(
	grip: ClipGrip,
	fromMs: number,
	toMs: number,
	fileMs: number | null,
	grabbed: { start_ms: number; offset_ms: number; length_ms: number | null }
): Pick<ClipPlace, 'start_ms' | 'offset_ms' | 'length_ms'> {
	const MIN = 50;
	const delta = toMs - fromMs;
	const rest = fileMs === null ? null : Math.max(0, fileMs - grabbed.offset_ms);
	const region = grabbed.length_ms ?? rest ?? 2_000;
	if (grip === 'body') {
		return {
			start_ms: Math.max(0, Math.round(grabbed.start_ms + delta)),
			offset_ms: grabbed.offset_ms,
			length_ms: grabbed.length_ms
		};
	}
	if (grip === 'front') {
		// No earlier than the file's own start, no earlier than the timeline's, and no
		// later than leaves MIN of the clip.
		const earliest = Math.max(-grabbed.offset_ms, -grabbed.start_ms);
		const moved = Math.min(Math.max(delta, earliest), region - MIN);
		return {
			start_ms: Math.round(grabbed.start_ms + moved),
			offset_ms: Math.round(grabbed.offset_ms + moved),
			length_ms: Math.round(region - moved)
		};
	}
	const longest = rest ?? Infinity;
	const length = Math.min(Math.max(region + delta, MIN), longest);
	return {
		start_ms: grabbed.start_ms,
		offset_ms: grabbed.offset_ms,
		// Dragged back out to the end of the file is "the rest of the file" again.
		length_ms: rest !== null && length >= rest ? null : Math.round(length)
	};
}

/** `"5, 6"` or `"5-8"` to channel numbers counting from one. `null` for anything else. */
export function parseChannels(text: string): number[] | null {
	const out: number[] = [];
	for (const part of text.split(/[,\s]+/).filter(Boolean)) {
		const range = /^(\d+)-(\d+)$/.exec(part);
		if (range) {
			const [from, to] = [Number(range[1]), Number(range[2])];
			if (from < 1 || to < from || to - from > 256) return null;
			for (let c = from; c <= to; c++) out.push(c);
		} else if (/^\d+$/.test(part) && Number(part) >= 1) {
			out.push(Number(part));
		} else {
			return null;
		}
	}
	return out.length > 0 ? [...new Set(out)] : null;
}

/** And back: `[5, 6, 7, 8]` reads `5-8`, `[1, 3]` reads `1, 3`. */
export function channelsText(channels: number[]): string {
	const sorted = [...channels].sort((a, b) => a - b);
	const runs: string[] = [];
	for (let i = 0; i < sorted.length; ) {
		let j = i;
		while (j + 1 < sorted.length && sorted[j + 1] === sorted[j] + 1) j++;
		runs.push(j - i >= 2 ? `${sorted[i]}-${sorted[j]}` : sorted.slice(i, j + 1).join(', '));
		i = j + 1;
	}
	return runs.join(', ');
}
