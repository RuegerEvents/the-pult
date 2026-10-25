/**
 * A timeline as lanes: what goes on which row, where each thing is drawn, what is under
 * the pointer, and what a drag does to it.
 *
 * Pure, so every rule here is tested without a canvas. The panel draws what this lays
 * out and hands back what the pointer did; nothing in here knows what a pixel looks
 * like beyond the `View` (`$lib/waveform.ts`) that turns a position into an x.
 *
 * **The lanes are worked out from what is on the timeline, never stored**, in a fixed
 * order: markers, events, then one lane per place sound goes, then one lane per take.
 * Nobody reorders them. A lane order somebody kept would be one more thing that stops
 * matching the content the moment a clip is re-routed, and the order here is the order
 * a scare is read in: where you are, what fires, what is heard, what moves.
 *
 * **A sound lane is a destination**, not a clip: everything going to the same speakers
 * shares a lane, so "what plays out of Area 2" is one row to read. Clips on one
 * destination that overlap in time — a scream over a bed — take extra rows inside that
 * lane (`clipLanes`), or one would be drawn under the other and could not be grabbed.
 *
 * **Recordings are one lane with a row per parameter**, labeled by fixture and
 * parameter — "Skeleton · Jaw" — and each take is a segment on its parameter's row:
 * a `TimelineTrack` per parameter, with a window. So a parameter recorded three times
 * over, sliced and cropped, reads as one row with its pieces in it, the way it plays.
 * Drawn from `pult_render::outline`, the same parse of the same bytes the station
 * plays, so a lane cannot show a take the lamps would not do.
 */

import { clipLanes, msOf, xOf, type View } from '$lib/waveform.js';

/** Everything on a timeline a person can pick up. Each is also an object kind. */
export type ItemKind =
	| 'timeline-clip'
	| 'timeline-event'
	| 'timeline-marker'
	| 'timeline-segment'
	| 'timeline-take';

export type ItemRef = { kind: ItemKind; id: string };

export const refKey = (ref: { kind: string; id: string }) => `${ref.kind}:${ref.id}`;

/** Heights, in CSS pixels. */
export const RULER_H = 24;
export const FLAG_H = 22;
export const CLIP_ROW_H = 40;
export const KEY_ROW_H = 18;
/** The row a take whose recording has not been read yet is drawn on. */
const UNREAD_ROW = 'unread';

export type LaneKind = 'markers' | 'events' | 'audio' | 'take';

export type Lane = {
	key: string;
	kind: LaneKind;
	label: string;
	/** A second line under the label: a take's source, a destination's fault. */
	detail?: string;
	/** A sound lane's zone color, drawn on its label and its clips' edges. */
	color?: string;
	/** Labels of the rows inside the lane, top to bottom — a parameter each, on the recordings lane. */
	rows: string[];
	/** What each row is, where that is more than its label: the recordings lane's `fixture/key`. */
	rowKeys?: string[];
	top: number;
	height: number;
	rowHeight: number;
	/** Above the first row: a take's name line. Zero for every other lane. */
	headHeight: number;
};

/** Where one thing is drawn. A point (event, marker, segment) has `fromMs === toMs`. */
export type Placed = {
	ref: ItemRef;
	lane: string;
	row: number;
	fromMs: number;
	toMs: number;
};

// ── What the panel hands in ────────────────────────────────────────────────────

export type ClipInput = {
	id: string;
	/** Which lane: everything going to the same place shares one. */
	destination: string;
	destinationLabel: string;
	/** The zone's own color, where the destination is a zone that has one. */
	color?: string | null;
	fromMs: number;
	toMs: number;
};

/** One parameter a take plays, over the stretch it plays it. */
export type KeyInput = { row: string; label: string; fromMs: number; toMs: number };

export type TakeInput = {
	id: string;
	/** The parameters it plays — one, or every one for an old whole take — or `null`
	 *  until its recording has been read. */
	keys: KeyInput[] | null;
};

export type LaneInput = {
	clips: ClipInput[];
	events: { id: string; atMs: number }[];
	markers: { id: string; atMs: number }[];
	segments: { id: string; atMs: number }[];
	takes: TakeInput[];
};

export type Layout = { lanes: Lane[]; items: Placed[]; height: number };

/** The lanes, top to bottom, and where every item on them goes. */
export function layOut(input: LaneInput): Layout {
	const lanes: Lane[] = [];
	const items: Placed[] = [];
	let top = RULER_H;

	const push = (lane: Omit<Lane, 'top'>) => {
		lanes.push({ ...lane, top });
		top += lane.height;
	};

	// The segments live on the ruler, which is where a tempo is read.
	for (const segment of input.segments) {
		items.push({
			ref: { kind: 'timeline-segment', id: segment.id },
			lane: 'ruler',
			row: 0,
			fromMs: segment.atMs,
			toMs: segment.atMs
		});
	}

	push({ key: 'markers', kind: 'markers', label: 'Markers', rows: [''], height: FLAG_H, rowHeight: FLAG_H, headHeight: 0 });
	for (const marker of input.markers) {
		items.push({ ref: { kind: 'timeline-marker', id: marker.id }, lane: 'markers', row: 0, fromMs: marker.atMs, toMs: marker.atMs });
	}

	push({ key: 'events', kind: 'events', label: 'Events', rows: [''], height: FLAG_H, rowHeight: FLAG_H, headHeight: 0 });
	for (const event of input.events) {
		items.push({ ref: { kind: 'timeline-event', id: event.id }, lane: 'events', row: 0, fromMs: event.atMs, toMs: event.atMs });
	}

	// One lane per destination, in the order their first sound starts.
	const groups = new Map<string, ClipInput[]>();
	for (const clip of input.clips) {
		const group = groups.get(clip.destination);
		if (group) group.push(clip);
		else groups.set(clip.destination, [clip]);
	}
	const ordered = [...groups.entries()].sort(
		(a, b) => Math.min(...a[1].map((c) => c.fromMs)) - Math.min(...b[1].map((c) => c.fromMs)) || a[0].localeCompare(b[0])
	);
	for (const [destination, clips] of ordered) {
		const rows = clipLanes(clips);
		const count = Math.max(1, ...rows.map((r) => r + 1));
		const key = `audio:${destination}`;
		push({
			key,
			kind: 'audio',
			label: clips[0].destinationLabel,
			color: clips[0].color ?? undefined,
			rows: Array.from({ length: count }, () => ''),
			height: count * CLIP_ROW_H,
			rowHeight: CLIP_ROW_H,
			headHeight: 0
		});
		clips.forEach((clip, i) =>
			items.push({ ref: { kind: 'timeline-clip', id: clip.id }, lane: key, row: rows[i], fromMs: clip.fromMs, toMs: clip.toMs })
		);
	}

	// One lane for every recording, a row per parameter, in the order a person reads a
	// rig: by fixture, then by parameter.
	if (input.takes.length) {
		const labels = new Map<string, string>();
		for (const take of input.takes) {
			for (const k of take.keys ?? [{ row: UNREAD_ROW, label: 'reading…', fromMs: 0, toMs: 0 }]) labels.set(k.row, k.label);
		}
		const rowKeys = [...labels.keys()].sort((a, b) =>
			a === UNREAD_ROW ? 1 : b === UNREAD_ROW ? -1 : labels.get(a)!.localeCompare(labels.get(b)!)
		);
		push({
			key: 'takes',
			kind: 'take',
			label: 'Recordings',
			rows: rowKeys.map((k) => labels.get(k)!),
			rowKeys,
			height: rowKeys.length * KEY_ROW_H + 4,
			rowHeight: KEY_ROW_H,
			headHeight: 0
		});
		for (const take of input.takes) {
			for (const k of take.keys ?? [{ row: UNREAD_ROW, label: '', fromMs: 0, toMs: 0 }]) {
				items.push({
					ref: { kind: 'timeline-take', id: take.id },
					lane: 'takes',
					row: rowKeys.indexOf(k.row),
					fromMs: k.fromMs,
					toMs: Math.max(k.fromMs, k.toMs)
				});
			}
		}
	}

	return { lanes, items, height: top + 6 };
}

// ── On the screen ─────────────────────────────────────────────────────────────

export type Rect = { left: number; right: number; top: number; bottom: number };

/** A point is this wide on the screen, for picking it up. */
export const POINT_PX = 5;

/** Where an item is drawn, in canvas pixels. A point is a thin band around its x. */
export function rectOf(item: Placed, layout: Layout, view: View, width: number): Rect {
	const x0 = xOf(item.fromMs, view, width);
	const x1 = xOf(item.toMs, view, width);
	if (item.lane === 'ruler') {
		return { left: x0 - POINT_PX, right: x0 + POINT_PX, top: 0, bottom: RULER_H };
	}
	const lane = layout.lanes.find((l) => l.key === item.lane);
	if (!lane) return { left: 0, right: 0, top: 0, bottom: 0 };
	const top = lane.top + item.row * lane.rowHeight;
	if (item.fromMs === item.toMs) {
		return { left: x0 - POINT_PX, right: x0 + POINT_PX, top, bottom: top + lane.rowHeight };
	}
	return { left: x0, right: Math.max(x1, x0 + 2), top: top + 2, bottom: top + lane.rowHeight - 2 };
}

/**
 * The item under a point, or `null`.
 *
 * Points beat spans where both are under the pointer, because a Go drawn through a
 * clip must stay grabbable — and on a lane of its own one never is under the other
 * anyway. Among spans, the one drawn last (on top) wins.
 */
export function hit(layout: Layout, view: View, width: number, x: number, y: number): Placed | null {
	let span: Placed | null = null;
	for (const item of layout.items) {
		const r = rectOf(item, layout, view, width);
		const pad = item.fromMs === item.toMs ? 0 : 2;
		if (x < r.left - pad || x > r.right + pad || y < r.top || y > r.bottom) continue;
		if (item.fromMs === item.toMs) return item;
		span = item;
	}
	return span;
}

/** The lane at a height, or `null` over the ruler and below the last lane. */
export function laneAt(layout: Layout, y: number): Lane | null {
	return layout.lanes.find((l) => y >= l.top && y < l.top + l.height) ?? null;
}

/**
 * Everything a rubber band touches. Partly inside counts: dragging a box across the
 * middle of a long bed should pick it up, the way it does in every audio editor.
 */
export function inBand(layout: Layout, view: View, width: number, band: Rect): ItemRef[] {
	const box = {
		left: Math.min(band.left, band.right),
		right: Math.max(band.left, band.right),
		top: Math.min(band.top, band.bottom),
		bottom: Math.max(band.top, band.bottom)
	};
	return layout.items
		.filter((item) => {
			const r = rectOf(item, layout, view, width);
			return r.right >= box.left && r.left <= box.right && r.bottom >= box.top && r.top <= box.bottom;
		})
		.map((item) => item.ref);
}

/** The position under an x, never before the top of the timeline. */
export const positionAt = (x: number, view: View, width: number) => Math.max(0, msOf(x, view, width));

// ── What a drag does ──────────────────────────────────────────────────────────

/** The timeline's columns a move rewrites, as the schema spells them. */
export type Movable = {
	clips: { id: string; start_ms: number }[];
	events: { id: string; at_ms: number }[];
	markers: { id: string; at_ms: number }[];
	grid: { at_ms: number }[];
	tracks: { id: string; offset_ms: number; from_ms: number | null; to_ms: number | null }[];
};

/** Where each grabbed item was when the drag began, which every frame is measured from. */
export type Grabbed = Map<string, number>;

/** What the grabbed items start from, keyed by `refKey`. Segments are keyed by index. */
export function grabbedFrom(timeline: Movable, refs: ItemRef[], segmentIndex: (id: string) => number): Grabbed {
	const out: Grabbed = new Map();
	for (const ref of refs) {
		let at: number | undefined;
		switch (ref.kind) {
			case 'timeline-clip':
				at = timeline.clips.find((c) => c.id === ref.id)?.start_ms;
				break;
			case 'timeline-event':
				at = timeline.events.find((e) => e.id === ref.id)?.at_ms;
				break;
			case 'timeline-marker':
				at = timeline.markers.find((m) => m.id === ref.id)?.at_ms;
				break;
			case 'timeline-segment':
				at = timeline.grid[segmentIndex(ref.id)]?.at_ms;
				break;
			case 'timeline-take':
				at = timeline.tracks.find((t) => t.id === ref.id)?.offset_ms;
				break;
		}
		if (at !== undefined) out.set(refKey(ref), at);
	}
	return out;
}

/**
 * Everything grabbed, moved by `deltaMs` from where it started — the columns that
 * changed and nothing else, so a drag of three events writes `events` once a frame and
 * leaves `clips` alone.
 *
 * The delta is clamped so nothing goes before the top: a group dragged left stops when
 * its earliest member reaches zero, rather than squashing everything onto it. A take's
 * offset is the exception and may go negative — a take that arrived late is played
 * early — but not so far that it would start before the top either.
 */
export function moved<T extends Movable>(
	timeline: T,
	grabbed: Grabbed,
	deltaMs: number,
	segmentIndex: (id: string) => number,
	takeStart: (id: string) => number = () => 0
): Partial<Pick<T, keyof Movable>> {
	let delta = Math.round(deltaMs);
	for (const [key, from] of grabbed) {
		const [kind, id] = splitKey(key);
		const earliest = kind === 'timeline-take' ? -(from + takeStart(id)) : -from;
		delta = Math.max(delta, earliest);
	}
	const has = (kind: ItemKind, id: string) => grabbed.get(`${kind}:${id}`);
	const out: Partial<Pick<T, keyof Movable>> = {};
	const touched = (kind: ItemKind) => [...grabbed.keys()].some((k) => k.startsWith(`${kind}:`));

	if (touched('timeline-clip')) {
		out.clips = timeline.clips.map((c) => {
			const from = has('timeline-clip', c.id);
			return from === undefined ? c : { ...c, start_ms: from + delta };
		}) as T['clips'];
	}
	if (touched('timeline-event')) {
		out.events = timeline.events
			.map((e) => {
				const from = has('timeline-event', e.id);
				return from === undefined ? e : { ...e, at_ms: from + delta };
			})
			.sort((a, b) => a.at_ms - b.at_ms) as T['events'];
	}
	if (touched('timeline-marker')) {
		out.markers = timeline.markers.map((m) => {
			const from = has('timeline-marker', m.id);
			return from === undefined ? m : { ...m, at_ms: from + delta };
		}) as T['markers'];
	}
	if (touched('timeline-segment')) {
		out.grid = timeline.grid.map((s, i) => {
			const key = [...grabbed.keys()].find((k) => k.startsWith('timeline-segment:') && segmentIndex(splitKey(k)[1]) === i);
			return key === undefined ? s : { ...s, at_ms: grabbed.get(key)! + delta };
		}) as T['grid'];
	}
	if (touched('timeline-take')) {
		// A take moves with its window, so a slice takes what it plays with it.
		out.tracks = timeline.tracks.map((t) => {
			const from = has('timeline-take', t.id);
			if (from === undefined) return t;
			const by = from + delta - t.offset_ms;
			const shift = (ms: number | null) => (ms === null ? null : Math.max(0, ms + by));
			return { ...t, offset_ms: from + delta, from_ms: shift(t.from_ms), to_ms: shift(t.to_ms) };
		}) as T['tracks'];
	}
	return out;
}

function splitKey(key: string): [ItemKind, string] {
	const at = key.indexOf(':');
	return [key.slice(0, at) as ItemKind, key.slice(at + 1)];
}

/**
 * Every grabbed item removed, as the columns that changed. Grid segments are removed by
 * index, since a segment has no id of its own.
 */
export function removed<T extends Movable>(
	timeline: T,
	refs: ItemRef[],
	segmentIndex: (id: string) => number
): Partial<Pick<T, keyof Movable>> {
	const ids = (kind: ItemKind) => new Set(refs.filter((r) => r.kind === kind).map((r) => r.id));
	const out: Partial<Pick<T, keyof Movable>> = {};
	const clips = ids('timeline-clip');
	if (clips.size) out.clips = timeline.clips.filter((c) => !clips.has(c.id)) as T['clips'];
	const events = ids('timeline-event');
	if (events.size) out.events = timeline.events.filter((e) => !events.has(e.id)) as T['events'];
	const markers = ids('timeline-marker');
	if (markers.size) out.markers = timeline.markers.filter((m) => !markers.has(m.id)) as T['markers'];
	const segments = new Set([...ids('timeline-segment')].map(segmentIndex));
	if (segments.size) out.grid = timeline.grid.filter((_, i) => !segments.has(i)) as T['grid'];
	const tracks = ids('timeline-take');
	if (tracks.size) out.tracks = timeline.tracks.filter((t) => !tracks.has(t.id)) as T['tracks'];
	return out;
}

// ── Grid segments have no id ──────────────────────────────────────────────────

/**
 * A segment's id: its timeline and its index. A segment is two numbers in a list and
 * nothing in the schema names one, so the index is the honest name — and the list is
 * kept sorted, so the index of a segment does not change while it is being looked at.
 */
export const segmentId = (timelineId: string, index: number) => `${timelineId}/${index}`;

export function segmentIndex(id: string): number {
	const at = id.lastIndexOf('/');
	return at < 0 ? -1 : Number(id.slice(at + 1));
}

export const segmentTimeline = (id: string) => id.slice(0, Math.max(0, id.lastIndexOf('/')));

// ── Zoom buttons, for a hand with no wheel ────────────────────────────────────

/** The view that shows the whole of `durationMs`. */
export const fitAll = (durationMs: number): View => ({ startMs: 0, spanMs: Math.max(2_000, durationMs) });
