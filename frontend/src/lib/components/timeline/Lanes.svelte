<script lang="ts">
	/**
	 * The timeline as lanes, on one ruler: markers, events, a lane per place sound goes,
	 * a lane per take — `$lib/timeline/lanes.ts` lays them out and says what is under the
	 * pointer; this draws it and turns pointers into writes.
	 *
	 * Drawn on a canvas, for the reason `beam.ts` draws beams on the GPU: a four-minute
	 * song is 24 000 waveform columns and several thousand beat lines, and a DOM node per
	 * column is a panel that stops being draggable. The lane names beside it are DOM,
	 * because text a screen reader can find and a person can select is worth a few
	 * elements.
	 *
	 * **Redrawn when something changes, and while the playhead is moving** — the rule the
	 * rig view follows. A stopped timeline costs one draw per edit.
	 *
	 * **The grammar is the console's**: a click picks something up and the Inspector
	 * edits it, ⌘/Ctrl- or Shift-click adds to what is picked, a drag across empty lanes
	 * is a rubber band, a right-click or long-press is the context menu, Delete removes
	 * what is picked. A click on nothing locates and puts the timeline itself back in the
	 * Inspector. A double-click on the event or marker lane writes one there.
	 *
	 * **A drag is one gesture and one write per animation frame**, measured from where
	 * the drag began — `stores/editor.ts`'s rule for dragging a truss. Several things
	 * dragged together move by one amount and stop together at the top; a clip's edge
	 * trims, and only when it is the one thing picked up.
	 *
	 * **The end is cropped where it is drawn.** A handle on the ruler: drag it, and
	 * everything past it goes dark, so what is shaded is what does not play. A timeline
	 * with no end has a faint one waiting where its content runs out; dragging that is
	 * how an end gets made. A click on it asks whether the end stops, goes round, or is
	 * taken away. Typing a time in the Inspector is still there for the exact number.
	 *
	 * **A take's change points are handles while it is the one thing picked up.** Drag
	 * one in time and level, double-click its row to add one, and Delete removes the one
	 * picked. Nothing is written while a handle moves: letting go writes a new recording of
	 * that one key — through the one encoder, `encode_key` in the wasm — and points the row
	 * at it, one write and one Ctrl-Z, and the recording it replaced stays as it was for
	 * every other row that plays it. **A flash is a block on its row**: drag it to move it,
	 * an end to stretch it. And the Recordings lane's lower edge drags its rows taller,
	 * for this browser.
	 */

	import { onMount, untrack } from 'svelte';

	import ZoomIn from '@lucide/svelte/icons/zoom-in';
	import ZoomOut from '@lucide/svelte/icons/zoom-out';
	import Maximize from '@lucide/svelte/icons/maximize-2';

	import type { AtEnd, AudioClip, AudioDestination, Detected, GridSegment, Marker, Timeline, TimelineAction } from '$lib/generated/index.js';
	import { audioNodes, destinationText } from '$lib/destination.js';
	import { clearSubject, openMenu, selectSubject, subject, type ObjectRef } from '$lib/objects.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { peaksFor } from '$lib/stores/peaks.js';
	import { collection, showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { outlines, wantOutline } from '$lib/stores/takeOutlines.js';
	import { audioStatus, current, devices, drivingReport, locate, positionOf, takeRowHeight, toggle, writeColumns } from '$lib/stores/timelines.js';
	import { encodeKey, trackPoints, type TrackPoint } from '$lib/evaluator.js';
	import { addToast } from '$lib/toasts.js';
	import { entries as programmerEntries } from '$lib/stores/programmer.js';
	import { LivePass, levelOf } from '$lib/timeline/liveTake.js';
	import { cropped, parameterRow, rowKeys, rowSpan, splitRows } from '$lib/takes.js';
	import type { TimelineTrack } from '$lib/generated/index.js';
	import { zones } from '$lib/stores/zones.js';
	import { describeOnRow, eventRow } from '$lib/timeline/describe.js';
	import {
		CLIP_ROW_H,
		RULER_H,
		fitAll,
		grabbedFrom,
		hit,
		inBand,
		laneAt,
		layOut,
		moved,
		positionAt,
		rectOf,
		refKey,
		removed,
		segmentId,
		segmentIndex,
		stretched,
		KEY_ROW_MAX,
		KEY_ROW_MIN,
		type ItemRef,
		type Layout,
		type Placed,
		type Rect
	} from '$lib/timeline/lanes.js';
	import {
		clampView,
		clipSpan,
		dragClip,
		gridLines,
		msOf,
		snap,
		xOf,
		zoomAt,
		type ClipGrip,
		type SnapTarget,
		type View
	} from '$lib/waveform.js';
	import { consoleNow } from '$lib/ws/clock.js';
	import { pinchStart, pinchView, wheelZoomFactor, type PinchStart } from '$lib/timeline/pinch.js';
	import { uuid } from '$lib/uuid.js';

	type Props = {
		timeline: Timeline;
		/** The detector's proposal, drawn over the grid while it is being considered. */
		proposal: Detected | null;
		snapping: boolean;
		/** Sound files dragged in from the desktop, and where on the timeline they landed. */
		ondropfiles?: (files: File[], atMs: number) => void;
	};
	let { timeline, proposal, snapping, ondropfiles }: Props = $props();

	/** Files over the lanes: shown, so a drop is aimed rather than hoped for. */
	let dropping = $state(false);
	const carriesFiles = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].includes('Files');
	function onDragOver(e: DragEvent) {
		if (!ondropfiles || !$canEdit || !carriesFiles(e)) return;
		e.preventDefault();
		if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
		dropping = true;
	}
	function onDrop(e: DragEvent) {
		dropping = false;
		if (!ondropfiles || !$canEdit || !carriesFiles(e)) return;
		e.preventDefault();
		const files = [...(e.dataTransfer?.files ?? [])];
		if (files.length === 0) return;
		const { x } = pointerAt(e);
		ondropfiles(files, Math.max(0, Math.round(snapped(msOf(x, view, width)))));
	}

	/** A marker at the playhead, for M and the + Marker button. */
	export async function markerAtPlayhead() {
		const live = current(timeline.id);
		if (!live || !$canEdit) return;
		const id = uuid();
		const at = Math.round(playhead() ?? 0);
		await writeColumns(timeline.id, { markers: [...live.markers, { id, at_ms: at, name: `Marker ${live.markers.length + 1}` }] });
		selectSubject({ kind: 'timeline-marker', id });
	}

	const sequences = collection('sequences');
	const cues = collection('cues');
	// For naming a timeline's events when they play another timeline or change a
	// variable.
	const allTimelines = collection('timelines');
	const variables = collection('variables');
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	/** The lane names' column. */
	const LABELS = 150;

	let canvas = $state<HTMLCanvasElement | null>(null);
	let box = $state<HTMLDivElement | null>(null);
	let scroller = $state<HTMLDivElement | null>(null);
	let width = $state(800);
	let visibleHeight = $state(200);
	/** What is on screen. Per browser and per timeline, in `localStorage`. */
	let view = $state<View>({ startMs: 0, spanMs: 60_000 });
	let dirty = $state(true);
	let hovering = $state<string | null>(null);

	type Drag =
		| { mode: 'pending'; x: number; y: number; item: Placed | null; additive: boolean; pointer: number }
		| { mode: 'move'; grabbed: Map<string, number>; fromMs: number; anchorMs: number }
		| {
				mode: 'trim';
				id: string;
				grip: ClipGrip;
				fromMs: number;
				grabbed: Pick<AudioClip, 'start_ms' | 'offset_ms' | 'length_ms'>;
		  }
		| { mode: 'crop'; id: string; grip: 'front' | 'back'; fromMs: number; span: { fromMs: number; toMs: number } }
		| { mode: 'stretch'; id: string; grip: 'front' | 'back'; fromMs: number; grabbed: { at_ms: number; duration_ms: number } }
		| { mode: 'point'; index: number; x: number; y: number; moved: boolean }
		| { mode: 'band'; x0: number; y0: number; x1: number; y1: number; additive: boolean; before: ObjectRef[] }
		| { mode: 'scrub' }
		| { mode: 'end'; x: number; then: AtEnd; moved: boolean }
		| { mode: 'pinch'; start: PinchStart };
	let drag = $state<Drag | null>(null);
	/** The end's own menu, open under its handle. */
	let endMenu = $state<{ x: number } | null>(null);
	/** Fingers on the glass, by pointer id: two of them is a pinch. */
	const touches = new Map<number, { x: number; y: number }>();
	let longPress: ReturnType<typeof setTimeout> | null = null;

	const viewKey = $derived(`pult.waveform.${timeline.id}`);
	const nodes = $derived(audioNodes($devices, $drivingReport));
	const faulted = $derived($audioStatus?.[timeline.id]?.clip_faults ?? {});
	/// Clips sounding through some of where they go and not the rest: drawn amber with a
	/// red stripe rather than red, which is kept for a clip nothing plays.
	const partial = $derived($audioStatus?.[timeline.id]?.clip_partial ?? {});
	/// What the pointer is over says what failed, since a canvas has no tooltips.
	const hoverTitle = $derived.by(() => {
		if (!hovering) return '';
		if (faulted[hovering]) return `Not playing: ${faulted[hovering]}`;
		const failed = partial[hovering];
		return failed?.length ? `Playing, except: ${failed.join('; ')}` : '';
	});

	// ── What is on the timeline, laid out ─────────────────────────────────────

	/** How long a clip's file is, which a browser knows from the peaks and nothing else. */
	function fileMs(clip: AudioClip): number | null {
		const peaks = clip.peaks ? peaksFor(clip.peaks, () => (dirty = true)) : null;
		return peaks?.durationMs ?? null;
	}

	/** As far as anything is written, or the song, and never less than thirty seconds. */
	const durationMs = $derived.by(() => {
		const written = Math.max(0, ...timeline.events.map((e) => e.at_ms), ...timeline.markers.map((m) => m.at_ms), timeline.end?.at_ms ?? 0);
		const sounding = Math.max(0, ...timeline.clips.filter((c) => !c.looped).map((c) => clipSpan(c, fileMs(c), 0).toMs));
		// A sixth more than the furthest thing, so the end is on screen with room to drag
		// past it; and never under ten seconds, so an empty timeline still has a ruler.
		return Math.max(10_000, Math.ceil(Math.max(sounding, written) * 1.15));
	});

	/** Where the content runs out: the last sound, event or marker. Where a new end waits. */
	const contentEndMs = $derived.by(() => {
		const written = Math.max(0, ...timeline.events.map((e) => e.at_ms), ...timeline.markers.map((m) => m.at_ms));
		const sounding = Math.max(0, ...timeline.clips.filter((c) => !c.looped).map((c) => clipSpan(c, fileMs(c), 0).toMs));
		return Math.max(written, sounding) || 10_000;
	});

	/** Where the end handle is: the end, or where one would go. */
	const endHandleMs = $derived(timeline.end?.at_ms ?? contentEndMs);

	/** How near the handle, on the screen, a press on the ruler takes it: the tab, which
	 * hangs to the left of the line, and a little either side. */
	const END_GRIP_PX = 8;
	const END_TAB_PX = 48;
	function onEndHandle(x: number, y: number): boolean {
		const at = xOf(endHandleMs, view, width);
		return y < RULER_H && x >= at - END_TAB_PX && x <= at + END_GRIP_PX;
	}

	function setEnd(end: Timeline['end']) {
		endMenu = null;
		void showData().timelines.byId(timeline.id).end.set(end);
	}

	const destinationKey = (d: AudioDestination) => ('Zone' in d ? `zone:${d.Zone.zone}` : `node:${d.Node.serial}`);

	/** A sound lane's color: its zone's, where it is a zone that has one. */
	const laneColor = (d: AudioDestination) => ('Zone' in d ? ($zones.find((z) => z.id === d.Zone.zone)?.color ?? null) : null);

	const fixtureName = (fixtureId: string) => $fixtures.find((f) => f.id === fixtureId)?.name ?? 'a gone fixture';

	/** What an operator calls one of a fixture's parameters: "Jaw", "Color". */
	function parameterLabel(fixtureId: string, key: string): string {
		const fixture = $fixtures.find((f) => f.id === fixtureId);
		const type = fixture ? $types.find((t) => t.id === fixture.fixture_type_id) : undefined;
		const parameter = type?.parameters.find((p) => parameterKey(p.kind) === key);
		return parameter ? displayLabel(parameter.kind) : key;
	}

	/** The take being recorded, as one more lane, while it has moved anything. */
	function liveTakeInput() {
		void liveVersion;
		const pass = livePass;
		if (!pass || pass.keys.size === 0) return [];
		const now = positionOf(timeline, consoleNow()) ?? 0;
		// On each parameter's own row, beside the takes it is about to replace.
		return [...pass.keys.values()].map((k) => ({
			id: 'live',
			keys: [
				{
					row: parameterRow(k.fixtureId, k.key),
					label: keyLabel(k.fixtureId, k.key),
					fromMs: k.points[0]?.[0] ?? 0,
					toMs: Math.max(now, k.points[k.points.length - 1]?.[0] ?? 0)
				}
			]
		}));
	}

	/** "Skeleton 1 · Jaw", where a take moved more than one fixture. */
	const keyLabel = (fixtureId: string, key: string) => `${fixtureName(fixtureId)} · ${parameterLabel(fixtureId, key)}`;

	/** A sound lane's name: the destination, and a node by its name or, unseen, as a node. */
	function laneName(destination: AudioDestination): string {
		if ('Node' in destination && !nodes.some((n) => n.serial === destination.Node.serial)) {
			return `Node ${destination.Node.serial}${destination.Node.mode === 'Stream' ? ' · stream' : ''}`;
		}
		return destinationText(destination, $zones, nodes);
	}

	$effect(() => {
		for (const track of timeline.tracks) wantOutline(track.asset);
	});

	const layout: Layout = $derived.by(() => {
		const until = timeline.end?.at_ms ?? durationMs;
		return layOut({
			segments: timeline.grid.map((s, i) => ({ id: segmentId(timeline.id, i), atMs: s.at_ms })),
			markers: timeline.markers.map((m) => ({ id: m.id, atMs: m.at_ms })),
			events: timeline.events.map((e) => ({ id: e.id, atMs: e.at_ms, ...eventRow(e.action, $sequences) })),
			clips: timeline.clips.map((clip) => {
				const span = clipSpan(clip, fileMs(clip), until);
				return {
					id: clip.id,
					destination: destinationKey(clip.destination),
					destinationLabel: laneName(clip.destination),
					color: laneColor(clip.destination),
					fromMs: span.fromMs,
					toMs: span.toMs
				};
			}),
			takes: timeline.tracks.map((track) => {
				const outline = $outlines[track.asset];
				if (!Array.isArray(outline)) return { id: track.id, keys: null };
				const keys = rowKeys(track, outline);
				const span = rowSpan(track, keys);
				return {
					id: track.id,
					keys: span
						? keys.map((k) => ({
								row: parameterRow(k.fixture_id, k.key),
								label: keyLabel(k.fixture_id, k.key),
								fromMs: span.fromMs,
								toMs: span.toMs
							}))
						: []
				};
			}).concat(liveTakeInput()),
			flashes: (timeline.flashes ?? []).map((f) => ({
				id: f.id,
				row: parameterRow(f.fixture_id, f.key),
				label: keyLabel(f.fixture_id, f.key),
				fromMs: f.at_ms,
				toMs: f.at_ms + Math.max(1, f.duration_ms)
			}))
		}, $takeRowHeight);
	});

	// ── A take's change points, as handles ────────────────────────────────────

	/** The one take picked up, with its change points in its recording's own time. */
	type Editing = { trackId: string; asset: string; fixtureId: string; key: string; points: TrackPoint[]; chosen: number | null };
	let editing = $state.raw<Editing | null>(null);
	const TRACK_MIME = 'application/vnd.pult.track';
	/** Past this many points in view the handles are a smear, and are not drawn. */
	const MAX_HANDLES = 300;
	const HANDLE_PX = 6;

	const soleTake = $derived.by(() => {
		const refs = $subject.filter((r) => picked.has(refKey(r)));
		if (refs.length !== 1 || refs[0].kind !== 'timeline-take') return null;
		return timeline.tracks.find((t) => t.id === refs[0].id) ?? null;
	});

	$effect(() => {
		const track = soleTake;
		const editable = $canEdit;
		const was = untrack(() => editing);
		if (!track || !editable || track.fixture_id === null || track.key === null) {
			if (was) editing = null;
			return;
		}
		if (was && was.trackId === track.id && was.asset === track.asset) return;
		// Another take, or the same one at another recording: no handles until it is read.
		if (was) editing = null;
		const { id, asset } = track;
		const fixtureId = track.fixture_id;
		const key = track.key;
		void (async () => {
			try {
				const response = await fetch(`/assets/${asset}`);
				if (!response.ok) return;
				const points = await trackPoints(new Uint8Array(await response.arrayBuffer()), fixtureId, key);
				// Still the one picked up, at the recording it was read from.
				const now = untrack(() => soleTake);
				if (points && now?.id === id && now.asset === asset) {
					editing = { trackId: id, asset, fixtureId, key, points, chosen: null };
					dirty = true;
				}
			} catch {
				// A station that has not got the take yet: no handles until it has.
			}
		})();
	});

	/** Where the edited take is drawn, its window in its recording's time, and its scale. */
	function editFrame() {
		const edit = editing;
		if (!edit) return null;
		const item = layout.items.find((i) => i.ref.kind === 'timeline-take' && i.ref.id === edit.trackId);
		const track = timeline.tracks.find((t) => t.id === edit.trackId);
		const span = track ? spanOfTake(track) : null;
		if (!item || !track || !span) return null;
		const r = rectOf(item, layout, view, width);
		const levels = edit.points.filter((p) => !p.release).map((p) => levelOf(p.value));
		const lo = Math.min(0, ...levels);
		const hi = Math.max(1, ...levels);
		const top = r.top + 1;
		const bottom = r.bottom - 1;
		return {
			edit,
			track,
			rect: r,
			// The window, in the recording's own milliseconds.
			from: span.fromMs - track.offset_ms,
			to: span.toMs - track.offset_ms,
			x: (ms: number) => xOf(ms + track.offset_ms, view, width),
			y: (level: number) => bottom - ((level - lo) / (hi - lo)) * (bottom - top),
			level: (y: number) => lo + ((bottom - Math.max(top, Math.min(bottom, y))) / Math.max(1, bottom - top)) * (hi - lo)
		};
	}

	/** The points of the edited take inside its window, with their index in the whole list. */
	function handles(frame: NonNullable<ReturnType<typeof editFrame>>): { index: number; x: number; y: number; release: boolean }[] {
		const out: { index: number; x: number; y: number; release: boolean }[] = [];
		frame.edit.points.forEach((p, index) => {
			if (p.ms < frame.from || p.ms >= frame.to) return;
			const x = frame.x(p.ms);
			if (x < -HANDLE_PX || x > width + HANDLE_PX) return;
			out.push({ index, x, y: p.release ? frame.rect.bottom - 2 : frame.y(levelOf(p.value)), release: !!p.release });
		});
		return out.length > MAX_HANDLES ? [] : out;
	}

	function handleAt(x: number, y: number): number | null {
		const frame = editFrame();
		if (!frame) return null;
		for (const h of handles(frame)) if (Math.abs(h.x - x) <= HANDLE_PX && Math.abs(h.y - y) <= HANDLE_PX) return h.index;
		return null;
	}

	/** A value like `like` at a level, for a point dragged up or down. Only a number moves. */
	function valueAt(like: TrackPoint['value'], level: number): TrackPoint['value'] {
		if (like.type === 'Float') return { type: 'Float', value: Math.max(0, Math.min(1, level)) };
		if (like.type === 'Int') return { type: 'Int', value: Math.round(Math.max(0, level)) };
		return like;
	}

	/** Write the edited points as a recording of their own, and point the row at it. */
	async function commitPoints(points: TrackPoint[]) {
		const edit = editing;
		const live = current(timeline.id);
		if (!edit || !live) return;
		try {
			const bytes = await encodeKey(edit.fixtureId, edit.key, points);
			if (!bytes) return;
			const response = await fetch('/assets', { method: 'POST', headers: { 'content-type': TRACK_MIME }, body: bytes.slice().buffer });
			if (!response.ok) throw new Error(await response.text());
			const { sha256 } = (await response.json()) as { sha256: string };
			const sorted = [...points].sort((a, b) => a.ms - b.ms);
			editing = { ...edit, asset: sha256, points: sorted, chosen: null };
			beginGesture();
			try {
				await writeColumns(timeline.id, {
					tracks: live.tracks.map((t) => (t.id === edit.trackId ? { ...t, asset: sha256 } : t))
				});
			} finally {
				endGesture();
			}
		} catch (e) {
			addToast(`The take could not be changed: ${e instanceof Error ? e.message : e}`);
		}
	}

	/** Move a handle to where the pointer is, inside the take's window. Drawn, not written. */
	function movePoint(index: number, x: number, y: number) {
		const frame = editFrame();
		if (!frame) return;
		const point = frame.edit.points[index];
		if (!point) return;
		const ms = Math.round(Math.max(frame.from, Math.min(frame.to - 1, msOf(x, view, width) - frame.track.offset_ms)));
		const value = point.release ? point.value : valueAt(point.value, frame.level(y));
		const points = frame.edit.points.map((p, i) => (i === index ? { ...p, ms, value } : p));
		editing = { ...frame.edit, points, chosen: index };
		dirty = true;
	}

	// ── The Recordings lane's rows, made taller ───────────────────────────────

	let rowsDrag: { y0: number; h0: number; rows: number } | null = null;
	function startRows(e: PointerEvent, rows: number) {
		e.preventDefault();
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		rowsDrag = { y0: e.clientY, h0: $takeRowHeight, rows: Math.max(1, rows) };
	}
	function moveRows(e: PointerEvent) {
		if (!rowsDrag) return;
		const h = rowsDrag.h0 + (e.clientY - rowsDrag.y0) / rowsDrag.rows;
		takeRowHeight.set(Math.round(Math.max(KEY_ROW_MIN, Math.min(KEY_ROW_MAX, h))));
	}
	function endRows() {
		rowsDrag = null;
	}

	const canvasHeight = $derived(Math.max(layout.height, visibleHeight));

	// ── A take being recorded, drawn as it goes ───────────────────────────────
	//
	// See `timeline/liveTake.ts`: a preview built here from the programmer this page
	// already holds, by the station's rules, replaced by the stored take when it lands.
	//
	// Every programmer write is a point, but not a relayout: the canvas is drawn every
	// frame while the timeline runs and `drawLiveTake` reads the points itself, so what
	// the layout needs is a row for a new key and a lane that grows with the playhead.
	// Relaid on every write it was sixty layouts a second per moving fader, which is
	// what lagged the page the first time this lane was here.
	let livePass = $state.raw<LivePass | null>(null);
	let liveVersion = $state(0);
	let liveKeys = 0;
	let relaidAt = 0;
	let relayLater: ReturnType<typeof setTimeout> | null = null;
	const RELAY_MS = 250;
	function relay() {
		if (relayLater !== null) clearTimeout(relayLater);
		relayLater = null;
		relaidAt = performance.now();
		liveVersion = untrack(() => liveVersion) + 1;
	}
	// From the timeline's own SYNCED `recording` and `running`, which every station
	// holds. It used to wait for the LOCAL `takes` row as well, which only the station
	// recording the take writes — the leader — so a page on any other station never drew
	// the lane at all (found at the gig, 2026-10-25).
	const recordingProgrammerNow = $derived(
		!!timeline.recording && 'Programmer' in timeline.recording && timeline.running
	);
	$effect(() => {
		if (!recordingProgrammerNow) {
			livePass = null;
			liveKeys = 0;
			return;
		}
		const entries = $programmerEntries;
		const position = positionOf(timeline, consoleNow());
		if (position === null) return;
		const pass = untrack(() => livePass) ?? new LivePass();
		pass.observe(position, entries);
		livePass = pass;
		if (pass.keys.size !== liveKeys || performance.now() - relaidAt >= RELAY_MS) {
			liveKeys = pass.keys.size;
			relay();
		} else if (relayLater === null) {
			relayLater = setTimeout(relay, RELAY_MS);
		}
	});
	$effect(() => () => {
		if (relayLater !== null) clearTimeout(relayLater);
	});

	/** What is picked up on this timeline. */
	const picked = $derived(new Set($subject.filter((r) => r.kind.startsWith('timeline-') && layout.items.some((i) => refKey(i.ref) === refKey(r))).map(refKey)));

	// ── Remembering the view ──────────────────────────────────────────────────

	function saveView() {
		try {
			localStorage.setItem(viewKey, JSON.stringify(view));
		} catch {
			// A private window. A remembered zoom is a convenience, never a reason not to draw.
		}
	}

	function loadView() {
		try {
			const raw = localStorage.getItem(viewKey);
			view = raw ? clampView(JSON.parse(raw) as View, durationMs) : clampView(fitAll(durationMs), durationMs);
		} catch {
			view = { startMs: 0, spanMs: Math.min(60_000, durationMs) };
		}
	}

	$effect(() => {
		void timeline.id;
		loadView();
	});

	function zoom(factor: number) {
		view = zoomAt(view, durationMs, view.startMs + view.spanMs / 2, factor);
		saveView();
	}

	function fit() {
		view = clampView(fitAll(durationMs), durationMs);
		saveView();
	}

	// ── Drawing ───────────────────────────────────────────────────────────────

	function playhead(): number | null {
		return positionOf(timeline, consoleNow());
	}

	function snapTargets(): SnapTarget[] {
		if (!snapping) return [];
		return [
			...gridLines(timeline.grid as GridSegment[], view.startMs, view.startMs + view.spanMs, 2_000),
			...timeline.markers.map((m: Marker) => ({ atMs: m.at_ms, what: 'marker' as const }))
		];
	}

	/** A readable step for the ruler's labels at this zoom. */
	function tickStep(): number {
		const perPx = view.spanMs / Math.max(1, width);
		for (const step of [100, 250, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000]) {
			if (step / perPx >= 70) return step;
		}
		return 600_000;
	}

	const label = (ms: number) => {
		const s = ms / 1000;
		const m = Math.floor(s / 60);
		const rest = s - m * 60;
		return `${m}:${rest < 10 ? '0' : ''}${rest.toFixed(ms % 1000 === 0 ? 0 : 1)}`;
	};

	/** The take being recorded: red, on its parameter's row, stepwise, gaps where it let go. */
	function drawLiveTake(ctx: CanvasRenderingContext2D, item: (typeof layout.items)[number]) {
		const pass = livePass;
		const lane = layout.lanes.find((l) => l.key === item.lane);
		const key = lane?.rowKeys?.[item.row];
		const live = key ? pass?.keys.get(key) : undefined;
		if (!lane || !live) return;
		const r = rectOf(item, layout, view, width);
		ctx.fillStyle = 'rgba(239,68,68,0.10)';
		ctx.fillRect(r.left, r.top, Math.max(2, r.right - r.left), r.bottom - r.top);
		const end = positionOf(timeline, consoleNow()) ?? 0;
		const top = r.top + 1;
		const bottom = r.bottom - 1;
		const y = (v: number) => bottom - Math.max(0, Math.min(1, v)) * (bottom - top);
		ctx.strokeStyle = '#f87171';
		ctx.beginPath();
		live.points.forEach(([ms, level], i) => {
			if (level === null) return;
			// Stepwise to the next point, or on to the playhead while it holds.
			const next = live.points[i + 1]?.[0] ?? end;
			ctx.moveTo(xOf(ms, view, width), y(level));
			ctx.lineTo(xOf(next, view, width), y(level));
		});
		ctx.stroke();
	}

	function draw() {
		if (!canvas) return;
		const ctx = canvas.getContext('2d');
		if (!ctx) return;
		const H = canvasHeight;
		const scale = window.devicePixelRatio || 1;
		if (canvas.width !== Math.round(width * scale) || canvas.height !== Math.round(H * scale)) {
			canvas.width = Math.round(width * scale);
			canvas.height = Math.round(H * scale);
		}
		ctx.setTransform(scale, 0, 0, scale, 0, 0);
		ctx.fillStyle = '#111';
		ctx.fillRect(0, 0, width, H);

		// Lane backgrounds, alternating, with a rule between rows.
		layout.lanes.forEach((lane, i) => {
			ctx.fillStyle = i % 2 === 0 ? '#151515' : '#181818';
			ctx.fillRect(0, lane.top, width, lane.height);
			ctx.fillStyle = '#222';
			ctx.fillRect(0, lane.top + lane.height - 1, width, 1);
		});

		// Past the end is cropped: dark, so what is lit is what plays.
		if (timeline.end) {
			const x = Math.round(xOf(timeline.end.at_ms, view, width));
			ctx.fillStyle = 'rgba(0,0,0,0.55)';
			ctx.fillRect(x, RULER_H, Math.max(0, width - x), H - RULER_H);
			ctx.strokeStyle = timeline.end.then === 'Loop' ? '#34d399' : '#c8c8c8';
			ctx.lineWidth = 2;
			ctx.beginPath();
			ctx.moveTo(x, RULER_H);
			ctx.lineTo(x, H);
			ctx.stroke();
			ctx.lineWidth = 1;
		}

		// The grid over the lanes: a bar line per downbeat, a tick per beat where they fit.
		const lines = gridLines(timeline.grid as GridSegment[], view.startMs, view.startMs + view.spanMs, 2_000);
		const beatsFit = lines.length < width / 5;
		for (const line of lines) {
			if (line.what === 'beat' && !beatsFit) continue;
			const x = Math.round(xOf(line.atMs, view, width)) + 0.5;
			ctx.strokeStyle = line.what === 'bar' ? 'rgba(255,255,255,0.16)' : 'rgba(255,255,255,0.05)';
			ctx.beginPath();
			ctx.moveTo(x, RULER_H);
			ctx.lineTo(x, H);
			ctx.stroke();
		}

		// Sound.
		for (const item of layout.items) {
			if (item.ref.kind !== 'timeline-clip') continue;
			const clip = timeline.clips.find((c) => c.id === item.ref.id);
			if (!clip) continue;
			const r = rectOf(item, layout, view, width);
			if (r.right < 0 || r.left > width) continue;
			const left = Math.max(0, r.left);
			const right = Math.min(width, r.right);
			const chosen = picked.has(refKey(item.ref));
			const fault = !!faulted[clip.id];
			const half = !fault && !!partial[clip.id]?.length;
			ctx.fillStyle = chosen ? '#1f3d66' : '#18263b';
			ctx.fillRect(left, r.top, right - left, r.bottom - r.top);
			const peaks = clip.peaks ? peaksFor(clip.peaks, () => (dirty = true)) : null;
			const span = clipSpan(clip, fileMs(clip), timeline.end?.at_ms ?? durationMs);
			if (peaks && span.regionMs > 0) {
				const msPerBin = (peaks.samplesPerBin * 1000) / peaks.sampleRate;
				const middle = (r.top + r.bottom) / 2;
				const amp = (r.bottom - r.top) / 2 - 3;
				ctx.strokeStyle = clip.looped ? '#4a7fc4' : '#5d95e0';
				ctx.beginPath();
				for (let x = Math.floor(left); x < right; x++) {
					const at = msOf(x, view, width) - clip.start_ms;
					if (at < 0) continue;
					const into = clip.looped ? at % span.regionMs : at;
					if (into >= span.regionMs) continue;
					const step = Math.max(1, Math.floor(view.spanMs / width / msPerBin));
					const from = Math.floor((clip.offset_ms + into) / msPerBin);
					let low = 0;
					let high = 0;
					for (let bin = from; bin < from + step && bin * 2 + 1 < peaks.bins.length; bin++) {
						low = Math.min(low, peaks.bins[bin * 2]);
						high = Math.max(high, peaks.bins[bin * 2 + 1]);
					}
					const gain = Math.min(1, Math.pow(10, clip.gain_db / 20));
					ctx.moveTo(x + 0.5, middle - high * gain * amp);
					ctx.lineTo(x + 0.5, middle - low * gain * amp);
				}
				ctx.stroke();
			}
			// Fades, as the slope a person reads them as.
			ctx.strokeStyle = 'rgba(255,255,255,0.35)';
			ctx.beginPath();
			if (clip.fade_in_ms > 0) {
				ctx.moveTo(r.left, r.bottom);
				ctx.lineTo(xOf(clip.start_ms + clip.fade_in_ms, view, width), r.top);
			}
			if (clip.fade_out_ms > 0 && !clip.looped) {
				ctx.moveTo(xOf(clip.start_ms + span.regionMs - clip.fade_out_ms, view, width), r.top);
				ctx.lineTo(r.right, r.bottom);
			}
			ctx.stroke();
			const zoneColor = layout.lanes.find((l) => l.key === item.lane)?.color;
			ctx.strokeStyle = fault ? '#ef4444' : half ? '#f5a524' : chosen ? '#cfe0ff' : (zoneColor ?? '#2f6fd0');
			ctx.lineWidth = chosen ? 2 : 1;
			ctx.strokeRect(left + 0.5, r.top + 0.5, right - left - 1, r.bottom - r.top - 1);
			ctx.lineWidth = 1;
			if (half) {
				// The part that is not sounding, as a stripe along the bottom edge.
				ctx.fillStyle = '#ef4444';
				ctx.fillRect(left + 1, r.bottom - 4, right - left - 2, 3);
			}
			ctx.fillStyle = fault ? '#ef4444' : half ? '#f5a524' : '#e6efff';
			ctx.font = '11px system-ui, sans-serif';
			ctx.fillText(`${clip.name || 'clip'}${clip.looped ? ' ⟲' : ''}${peaks ? '' : ' — reducing…'}`, left + 5, r.top + 13, Math.max(10, right - left - 10));
		}

		// Takes: each a segment on its parameter's row, its movement as a stepped line,
		// cut to the window it plays.
		for (const item of layout.items) {
			if (item.ref.kind !== 'timeline-take') continue;
			if (item.ref.id === 'live') {
				drawLiveTake(ctx, item);
				continue;
			}
			const track = timeline.tracks.find((t) => t.id === item.ref.id);
			const lane = layout.lanes.find((l) => l.key === item.lane);
			if (!track || !lane) continue;
			const chosen = picked.has(refKey(item.ref));
			const outline = $outlines[track.asset];
			const r = rectOf(item, layout, view, width);
			if (r.right < 0 || r.left > width) continue;
			ctx.fillStyle = chosen ? 'rgba(74,222,128,0.18)' : track.enabled ? 'rgba(74,222,128,0.08)' : 'rgba(255,255,255,0.03)';
			ctx.fillRect(r.left, r.top, Math.max(2, r.right - r.left), r.bottom - r.top);
			const rowKey = lane.rowKeys?.[item.row];
			const key = Array.isArray(outline) ? outline.find((k) => parameterRow(k.fixture_id, k.key) === rowKey) : undefined;
			if (key && key.levels.length) {
				const top = r.top + 1;
				const bottom = r.bottom - 1;
				const values = key.levels.map(([, v]) => v);
				const lo = Math.min(0, ...values);
				const hi = Math.max(1, ...values);
				const y = (v: number) => bottom - ((v - lo) / (hi - lo)) * (bottom - top);
				ctx.save();
				ctx.beginPath();
				ctx.rect(r.left, r.top, Math.max(2, r.right - r.left), r.bottom - r.top);
				ctx.clip();
				ctx.strokeStyle = track.enabled ? '#4ade80' : '#3f5f48';
				ctx.beginPath();
				key.levels.forEach(([ms, v], i) => {
					const x = xOf(ms + track.offset_ms, view, width);
					// Stepwise, because replay is: a point holds until the next one.
					if (i === 0) ctx.moveTo(x, y(v));
					else {
						ctx.lineTo(x, y(key.levels[i - 1][1]));
						ctx.lineTo(x, y(v));
					}
				});
				ctx.lineTo(r.right, y(key.levels[key.levels.length - 1][1]));
				ctx.stroke();
				ctx.restore();
			} else if (!Array.isArray(outline)) {
				ctx.fillStyle = '#666';
				ctx.font = '10px system-ui, sans-serif';
				ctx.fillText(outline === 'failed' ? 'this station has not got the take yet' : 'reading…', Math.max(4, r.left + 4), r.top + 12);
			}
			ctx.strokeStyle = chosen ? '#bbf7d0' : track.enabled ? 'rgba(74,222,128,0.45)' : 'rgba(255,255,255,0.12)';
			ctx.strokeRect(r.left + 0.5, r.top + 0.5, Math.max(2, r.right - r.left) - 1, r.bottom - r.top - 1);
		}

		// Flashes: a block per press, over the takes on their row, as long as the key was held.
		for (const item of layout.items) {
			if (item.ref.kind !== 'timeline-flash') continue;
			const r = rectOf(item, layout, view, width);
			if (r.right < 0 || r.left > width) continue;
			const chosen = picked.has(refKey(item.ref));
			ctx.fillStyle = chosen ? 'rgba(251,191,36,0.6)' : 'rgba(251,191,36,0.32)';
			ctx.fillRect(r.left, r.top, Math.max(2, r.right - r.left), r.bottom - r.top);
			ctx.strokeStyle = chosen ? '#fde68a' : '#f59e0b';
			ctx.strokeRect(r.left + 0.5, r.top + 0.5, Math.max(2, r.right - r.left) - 1, r.bottom - r.top - 1);
		}

		// The picked take's change points, as handles: hollow where it lets go.
		const frame = editFrame();
		if (frame) {
			for (const h of handles(frame)) {
				const chosenPoint = frame.edit.chosen === h.index;
				ctx.fillStyle = chosenPoint ? '#fff' : '#4ade80';
				ctx.strokeStyle = '#052e16';
				ctx.beginPath();
				if (h.release) ctx.rect(h.x - 3, h.y - 3, 6, 6);
				else ctx.arc(h.x, h.y, chosenPoint ? 4 : 3, 0, Math.PI * 2);
				if (h.release && !chosenPoint) {
					ctx.strokeStyle = '#4ade80';
					ctx.stroke();
				} else {
					ctx.fill();
					ctx.stroke();
				}
			}
		}

		// Markers and events.
		const markersLane = layout.lanes.find((l) => l.kind === 'markers')!;
		for (const marker of timeline.markers) {
			const x = Math.round(xOf(marker.at_ms, view, width)) + 0.5;
			if (x < -100 || x > width + 100) continue;
			const chosen = picked.has(`timeline-marker:${marker.id}`);
			ctx.strokeStyle = chosen || hovering === marker.id ? '#fde68a' : 'rgba(240,198,116,0.45)';
			ctx.beginPath();
			ctx.moveTo(x, markersLane.top);
			ctx.lineTo(x, H);
			ctx.stroke();
			ctx.fillStyle = chosen ? '#fde68a' : '#f0c674';
			ctx.fillRect(x, markersLane.top + 3, 3, markersLane.height - 6);
			ctx.font = '11px system-ui, sans-serif';
			ctx.fillText(marker.name, x + 6, markersLane.top + 15);
		}
		const eventsLane = layout.lanes.find((l) => l.kind === 'events')!;
		for (const event of timeline.events) {
			const rowTop = eventsLane.top + Math.max(0, eventsLane.rowKeys?.indexOf(eventRow(event.action, $sequences).row) ?? 0) * eventsLane.rowHeight;
			const x = Math.round(xOf(event.at_ms, view, width)) + 0.5;
			if (x < -150 || x > width + 150) continue;
			const chosen = picked.has(`timeline-event:${event.id}`);
			ctx.strokeStyle = chosen || hovering === event.id ? '#fff' : 'rgba(52,211,153,0.5)';
			ctx.beginPath();
			ctx.moveTo(x, rowTop);
			ctx.lineTo(x, H);
			ctx.stroke();
			const mid = rowTop + eventsLane.rowHeight / 2;
			ctx.fillStyle = chosen ? '#fff' : '#34d399';
			ctx.beginPath();
			ctx.moveTo(x, mid - 6);
			ctx.lineTo(x + 6, mid);
			ctx.lineTo(x, mid + 6);
			ctx.lineTo(x - 6, mid);
			ctx.closePath();
			ctx.fill();
			ctx.fillStyle = chosen ? '#fff' : '#a7f3d0';
			ctx.font = '11px system-ui, sans-serif';
			ctx.fillText(describeOnRow(event.action, $sequences, $cues, $allTimelines, $variables), x + 9, mid + 4);
		}

		// The ruler, over everything: time, the tempo segments, the detector's proposal.
		ctx.fillStyle = '#0d0d0d';
		ctx.fillRect(0, 0, width, RULER_H);
		ctx.fillStyle = '#2a2a2a';
		ctx.fillRect(0, RULER_H - 1, width, 1);
		const step = tickStep();
		ctx.font = '10px system-ui, sans-serif';
		for (let at = Math.ceil(view.startMs / step) * step; at <= view.startMs + view.spanMs; at += step) {
			const x = Math.round(xOf(at, view, width)) + 0.5;
			ctx.strokeStyle = '#3a3a3a';
			ctx.beginPath();
			ctx.moveTo(x, RULER_H - 7);
			ctx.lineTo(x, RULER_H - 1);
			ctx.stroke();
			ctx.fillStyle = '#888';
			ctx.fillText(label(at), x + 3, 11);
		}
		timeline.grid.forEach((segment, i) => {
			const x = Math.round(xOf(segment.at_ms, view, width)) + 0.5;
			const chosen = picked.has(`timeline-segment:${segmentId(timeline.id, i)}`);
			ctx.fillStyle = chosen ? '#fff' : '#b9a6fc';
			ctx.beginPath();
			ctx.moveTo(x - 5, RULER_H - 1);
			ctx.lineTo(x + 5, RULER_H - 1);
			ctx.lineTo(x, RULER_H - 8);
			ctx.closePath();
			ctx.fill();
			ctx.fillText(`${Math.round(segment.bpm * 10) / 10}`, x + 6, RULER_H - 3);
		});
		drawEndHandle(ctx);
		if (proposal) {
			// In its own color and above the grid: the whole point of drawing it is to be
			// compared with what is already there.
			ctx.strokeStyle = 'rgba(74,222,128,0.8)';
			for (const at of proposal.downbeats_ms) {
				const x = Math.round(xOf(at, view, width)) + 0.5;
				ctx.beginPath();
				ctx.moveTo(x, RULER_H);
				ctx.lineTo(x, H);
				ctx.stroke();
			}
			if (proposal.beats_ms.length < width / 3) {
				ctx.strokeStyle = 'rgba(74,222,128,0.25)';
				for (const at of proposal.beats_ms) {
					const x = Math.round(xOf(at, view, width)) + 0.5;
					ctx.beginPath();
					ctx.moveTo(x, RULER_H);
					ctx.lineTo(x, RULER_H + 10);
					ctx.stroke();
				}
			}
		}

		// The rubber band.
		if (drag?.mode === 'band') {
			ctx.fillStyle = 'rgba(74,158,255,0.12)';
			ctx.strokeStyle = 'rgba(74,158,255,0.8)';
			const x = Math.min(drag.x0, drag.x1);
			const y = Math.min(drag.y0, drag.y1);
			ctx.fillRect(x, y, Math.abs(drag.x1 - drag.x0), Math.abs(drag.y1 - drag.y0));
			ctx.strokeRect(x + 0.5, y + 0.5, Math.abs(drag.x1 - drag.x0), Math.abs(drag.y1 - drag.y0));
		}

		// And the playhead, or a gap where it would be.
		const at = playhead();
		if (at === null) {
			ctx.fillStyle = '#777';
			ctx.font = '11px system-ui, sans-serif';
			ctx.fillText('waiting for the station clock', width - 190, 11);
		} else {
			const shown = timeline.end?.then === 'Loop' && timeline.end.at_ms > 0 ? at % timeline.end.at_ms : at;
			const x = Math.round(xOf(shown, view, width)) + 0.5;
			ctx.strokeStyle = timeline.running ? '#4ade80' : '#ddd';
			ctx.lineWidth = 2;
			ctx.beginPath();
			ctx.moveTo(x, 0);
			ctx.lineTo(x, H);
			ctx.stroke();
			ctx.lineWidth = 1;
		}
	}

	/**
	 * The end's handle on the ruler: a tab hanging from the top, with how it ends on it.
	 * Solid where there is an end, a faint outline where one would go.
	 */
	function drawEndHandle(ctx: CanvasRenderingContext2D) {
		const x = Math.round(xOf(endHandleMs, view, width));
		if (x < -40 || x > width + 40) return;
		const end = timeline.end;
		const loop = end?.then === 'Loop';
		const color = !end ? 'rgba(200,200,200,0.45)' : loop ? '#34d399' : '#c8c8c8';
		const text = !end ? 'end' : loop ? '↻ loop' : '■ stop';
		ctx.font = '10px system-ui, sans-serif';
		const w = Math.ceil(ctx.measureText(text).width) + 10;
		const left = x - w;
		const grabbed = drag?.mode === 'end';
		ctx.fillStyle = end ? (grabbed ? '#fff' : color) : 'rgba(17,17,17,0.9)';
		ctx.strokeStyle = color;
		ctx.beginPath();
		ctx.roundRect(left, 2, w, RULER_H - 4, [3, 0, 0, 3]);
		if (end) ctx.fill();
		else {
			ctx.fill();
			ctx.setLineDash([3, 2]);
			ctx.stroke();
			ctx.setLineDash([]);
		}
		ctx.fillStyle = end ? '#111' : color;
		ctx.fillText(text, left + 5, RULER_H - 6);
		ctx.fillStyle = color;
		ctx.fillRect(x - 1, 0, 2, RULER_H);
	}

	// ── Pointers ──────────────────────────────────────────────────────────────

	/** How near an edge, on the screen, a press has to be to trim rather than move. */
	const EDGE_PX = 6;
	/** How far a press travels before it is a drag and not a click. */
	const SLOP_PX = 4;
	/** How near a dragged position has to be, on the screen, to take a grid line. */
	const SNAP_PX = 7;

	function pointerAt(e: PointerEvent | MouseEvent): { x: number; y: number } {
		const rect = canvas?.getBoundingClientRect();
		return { x: e.clientX - (rect?.left ?? 0), y: e.clientY - (rect?.top ?? 0) };
	}

	const snapped = (ms: number) => Math.max(0, snap(ms, snapTargets(), view, width, SNAP_PX));

	function gripOf(item: Placed, x: number): ClipGrip {
		const r = rectOf(item, layout, view, width);
		const clip = timeline.clips.find((c) => c.id === item.ref.id);
		if (x - r.left <= EDGE_PX) return 'front';
		if (clip && !clip.looped && r.right - x <= EDGE_PX) return 'back';
		return 'body';
	}

	/** Everything picked up on this timeline, as refs. */
	const pickedRefs = (): ItemRef[] =>
		$subject.filter((r) => picked.has(refKey(r))).map((r) => ({ kind: r.kind as ItemRef['kind'], id: r.id }));

	/** Coalesced to one write an animation frame. */
	let pendingMs: number | null = null;
	let frame: number | null = null;

	function schedule(ms: number) {
		pendingMs = ms;
		if (frame !== null) return;
		frame = requestAnimationFrame(() => {
			frame = null;
			const at = pendingMs;
			pendingMs = null;
			if (at !== null) void apply(at);
		});
	}

	async function apply(rawMs: number) {
		const live = current(timeline.id);
		const drag = dragNow();
		if (!live || !drag) return;
		if (drag.mode === 'scrub') {
			await locate(timeline.id, rawMs);
			return;
		}
		if (drag.mode === 'end') {
			const at = Math.max(1, Math.round(snapped(rawMs)));
			await showData().timelines.byId(timeline.id).end.set({ at_ms: at, then: drag.then });
			return;
		}
		if (drag.mode === 'move') {
			// What snaps is the grabbed item's own position, not the pointer, which was
			// taken somewhere in the middle of it.
			const delta = snapped(drag.anchorMs + rawMs - drag.fromMs) - drag.anchorMs;
			await writeColumns(timeline.id, moved(live, drag.grabbed, delta, segmentIndex, takeStart));
			return;
		}
		if (drag.mode === 'crop') {
			const edge = drag.grip === 'front' ? drag.span.fromMs : drag.span.toMs;
			const at = snapped(edge + rawMs - drag.fromMs);
			await writeColumns(timeline.id, {
				tracks: live.tracks.map((t) => (t.id === drag.id ? cropped(t, drag.grip, at, drag.span) : t))
			});
			return;
		}
		if (drag.mode === 'stretch') {
			const edge = drag.grip === 'front' ? drag.grabbed.at_ms : drag.grabbed.at_ms + drag.grabbed.duration_ms;
			const at = snapped(edge + rawMs - drag.fromMs);
			await writeColumns(timeline.id, {
				flashes: (live.flashes ?? []).map((f) => (f.id === drag.id ? stretched(f, drag.grip, at, drag.grabbed) : f))
			});
			return;
		}
		if (drag.mode === 'trim') {
			const clip = live.clips.find((c) => c.id === drag.id);
			if (!clip) return;
			const file = fileMs(clip);
			const region = clipSpan({ ...clip, ...drag.grabbed }, file, 0).regionMs;
			const edge = drag.grip === 'back' ? drag.grabbed.start_ms + region : drag.grabbed.start_ms;
			const landed = snapped(edge + rawMs - drag.fromMs) - edge;
			const next = { ...clip, ...dragClip(drag.grip, 0, landed, file, drag.grabbed) };
			await writeColumns(timeline.id, { clips: live.clips.map((c) => (c.id === drag.id ? next : c)) });
		}
	}

	/** The drag as it is now, read once so a branch can narrow it. */
	const dragNow = (): Drag | null => drag;

	/** Where a take's first point is in its own time, so a move cannot start it before the top. */
	function takeStart(id: string): number {
		const track = timeline.tracks.find((t) => t.id === id);
		const span = track ? spanOfTake(track) : null;
		return track && span ? span.fromMs - track.offset_ms : 0;
	}

	/** Where a take's row is on the timeline, from its window or its recording. */
	function spanOfTake(track: TimelineTrack): { fromMs: number; toMs: number } | null {
		const outline = $outlines[track.asset];
		return rowSpan(track, Array.isArray(outline) ? rowKeys(track, outline) : null);
	}

	/** Which end of a take's segment a press is on, if either. */
	function takeGrip(item: Placed, x: number): 'front' | 'back' | null {
		const r = rectOf(item, layout, view, width);
		if (r.right - r.left < EDGE_PX * 3) return null;
		if (x - r.left <= EDGE_PX) return 'front';
		if (r.right - x <= EDGE_PX) return 'back';
		return null;
	}

	/**
	 * Slice every picked take at the playhead — or, with none picked, every take the
	 * playhead is inside — into two rows on the same recording. One write, one Ctrl-Z.
	 */
	async function splitAtPlayhead() {
		const live = current(timeline.id);
		const at = playhead();
		if (!live || at === null || !$canEdit) return;
		const chosen = pickedRefs().filter((r) => r.kind === 'timeline-take').map((r) => r.id);
		const ids = new Set(chosen.length ? chosen : live.tracks.map((t) => t.id));
		const tracks = splitRows(live.tracks, ids, at, spanOfTake, uuid);
		if (!tracks) return;
		beginGesture();
		try {
			await writeColumns(timeline.id, { tracks });
		} finally {
			endGesture();
		}
	}

	function cancelLongPress() {
		if (longPress) clearTimeout(longPress);
		longPress = null;
	}

	function onPointerDown(e: PointerEvent) {
		if (e.button !== 0 && e.pointerType === 'mouse') return;
		const { x, y } = pointerAt(e);
		if (e.pointerType === 'touch') {
			touches.set(e.pointerId, { x, y });
			// A second finger before the first has started moving anything, or while it
			// is only drawing a band, is a pinch: the band is dropped and the selection
			// put back. A finger already dragging a clip keeps dragging it.
			if (touches.size === 2 && (!drag || drag.mode === 'pending' || drag.mode === 'band')) {
				cancelLongPress();
				if (drag?.mode === 'band') subject.set(drag.before);
				const [a, b] = [...touches.values()];
				drag = { mode: 'pinch', start: pinchStart(view, width, a, b) };
				(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
				return;
			}
		}
		box?.focus();
		const item = hit(layout, view, width, x, y);
		const additive = e.shiftKey || e.metaKey || e.ctrlKey;
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		const handle = $canEdit && !additive ? handleAt(x, y) : null;
		if (handle !== null && editing) {
			editing = { ...editing, chosen: handle };
			drag = { mode: 'point', index: handle, x, y, moved: false };
			dirty = true;
			return;
		}
		if (y < RULER_H && $canEdit && onEndHandle(x, y)) {
			drag = { mode: 'end', x, then: timeline.end?.then ?? 'Stop', moved: false };
			return;
		}
		endMenu = null;
		if (!item && y < RULER_H) {
			drag = { mode: 'scrub' };
			void locate(timeline.id, positionAt(x, view, width));
			return;
		}
		drag = { mode: 'pending', x, y, item, additive, pointer: e.pointerId };
		if (item && e.pointerType !== 'mouse') {
			// A finger has no right button: holding still on a thing is its menu.
			const at = { x: e.clientX, y: e.clientY };
			longPress = setTimeout(() => {
				longPress = null;
				if (drag?.mode !== 'pending') return;
				drag = null;
				selectSubject(item.ref);
				openMenu(item.ref, at.x, at.y);
			}, 550);
		}
	}

	function begin(pending: Extract<Drag, { mode: 'pending' }>, x: number, y: number) {
		cancelLongPress();
		const { item } = pending;
		if (!item) {
			drag = { mode: 'band', x0: pending.x, y0: pending.y, x1: x, y1: y, additive: pending.additive, before: pending.additive ? [...$subject] : [] };
			return;
		}
		if (!$canEdit) {
			drag = null;
			return;
		}
		if (!picked.has(refKey(item.ref))) selectSubject(item.ref, pending.additive);
		const live = current(timeline.id);
		if (!live) return;
		const fromMs = msOf(pending.x, view, width);
		beginGesture();
		const refs = pickedRefs().length ? pickedRefs() : [item.ref];
		if (item.ref.kind === 'timeline-clip' && refs.length === 1) {
			const grip = gripOf(item, pending.x);
			const clip = live.clips.find((c) => c.id === item.ref.id);
			if (clip && grip !== 'body') {
				drag = { mode: 'trim', id: clip.id, grip, fromMs, grabbed: { start_ms: clip.start_ms, offset_ms: clip.offset_ms, length_ms: clip.length_ms } };
				return;
			}
		}
		if (item.ref.kind === 'timeline-take' && refs.length === 1) {
			const grip = takeGrip(item, pending.x);
			const track = live.tracks.find((t) => t.id === item.ref.id);
			const span = track ? spanOfTake(track) : null;
			if (track && span && grip) {
				drag = { mode: 'crop', id: track.id, grip, fromMs, span };
				return;
			}
		}
		if (item.ref.kind === 'timeline-flash' && refs.length === 1) {
			const grip = takeGrip(item, pending.x);
			const flash = (live.flashes ?? []).find((f) => f.id === item.ref.id);
			if (flash && grip) {
				drag = { mode: 'stretch', id: flash.id, grip, fromMs, grabbed: { at_ms: flash.at_ms, duration_ms: flash.duration_ms } };
				return;
			}
		}
		const grabbed = grabbedFrom(live, refs, segmentIndex);
		const anchorMs = item.fromMs;
		drag = { mode: 'move', grabbed, fromMs, anchorMs };
	}

	function onPointerMove(e: PointerEvent) {
		const { x, y } = pointerAt(e);
		if (touches.has(e.pointerId)) touches.set(e.pointerId, { x, y });
		if (drag?.mode === 'pinch') {
			if (touches.size >= 2) {
				const [a, b] = [...touches.values()];
				view = pinchView(drag.start, width, durationMs, a, b);
				dirty = true;
			}
			return;
		}
		if (!drag) {
			const item = hit(layout, view, width, x, y);
			hovering = item?.ref.id ?? null;
			if (box) {
				box.style.cursor = $canEdit && onEndHandle(x, y)
					? 'ew-resize'
					: !item
					? y < RULER_H
						? 'col-resize'
						: 'crosshair'
					: (item.ref.kind === 'timeline-clip' && gripOf(item, x) !== 'body') ||
						  ((item.ref.kind === 'timeline-take' || item.ref.kind === 'timeline-flash') && item.ref.id !== 'live' && takeGrip(item, x))
						? 'ew-resize'
						: 'grab';
			}
			dirty = true;
			return;
		}
		if (drag.mode === 'point') {
			if (!drag.moved && Math.hypot(x - drag.x, y - drag.y) < SLOP_PX) return;
			drag = { ...drag, moved: true };
			movePoint(drag.index, x, y);
			return;
		}
		if (drag.mode === 'pending') {
			if (Math.hypot(x - drag.x, y - drag.y) < SLOP_PX) return;
			begin(drag, x, y);
		}
		if (!drag) return;
		if (drag.mode === 'band') {
			drag = { ...drag, x1: x, y1: y };
			const band: Rect = { left: drag.x0, right: drag.x1, top: drag.y0, bottom: drag.y1 };
			const caught = inBand(layout, view, width, band);
			const keep = drag.before.filter((r) => !caught.some((c) => refKey(c) === refKey(r)));
			subject.set([...keep, ...caught]);
			dirty = true;
			return;
		}
		if (drag.mode === 'scrub') {
			schedule(positionAt(x, view, width));
			return;
		}
		if (drag.mode === 'end') {
			if (!drag.moved) {
				if (Math.abs(x - drag.x) < SLOP_PX) return;
				drag = { ...drag, moved: true };
				beginGesture();
			}
			schedule(msOf(x, view, width));
			return;
		}
		schedule(msOf(x, view, width));
	}

	function onPointerUp(e: PointerEvent) {
		cancelLongPress();
		touches.delete(e.pointerId);
		if (drag?.mode === 'pinch') {
			// One finger up ends the pinch; the other does nothing until it lifts too.
			drag = null;
			saveView();
			try {
				(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
			} catch {
				// Already released.
			}
			return;
		}
		const was = drag;
		drag = null;
		dirty = true;
		try {
			(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
		} catch {
			// Already released, by a cancel.
		}
		if (!was) return;
		if (was.mode === 'end') {
			if (was.moved) endGesture();
			else endMenu = { x: xOf(endHandleMs, view, width) };
			return;
		}
		if (was.mode === 'pending') {
			// A click: on a thing, pick it up; on nothing, go there and give the Inspector
			// back to the timeline.
			if (was.item) selectSubject(was.item.ref, was.additive);
			else {
				if (!was.additive) clearTimelineItems();
				void locate(timeline.id, snapped(msOf(was.x, view, width)));
			}
			return;
		}
		if (was.mode === 'point') {
			if (was.moved && editing) void commitPoints(editing.points);
			return;
		}
		if (was.mode === 'move' || was.mode === 'trim' || was.mode === 'crop' || was.mode === 'stretch') endGesture();
	}

	function clearTimelineItems() {
		const rest = $subject.filter((r) => !picked.has(refKey(r)));
		if (rest.length === $subject.length) return;
		if (rest.length === 0) clearSubject();
		else subject.set(rest);
	}

	function onContextMenu(e: MouseEvent) {
		const { x, y } = pointerAt(e);
		const item = hit(layout, view, width, x, y);
		if (!item) return;
		e.preventDefault();
		if (!picked.has(refKey(item.ref))) selectSubject(item.ref);
		openMenu(item.ref, e.clientX, e.clientY);
	}

	/** A new event at a position, picked up into the Inspector. `row` is a row of the
	 *  events lane (`eventRow`), which says what it acts on. */
	async function addEvent(at: number, row = '') {
		const live = current(timeline.id);
		if (!live || !$canEdit) return;
		const id = uuid();
		const onSequence = row.startsWith('sequence:') ? row.slice('sequence:'.length) : (row === '' ? $sequences[0]?.id : undefined);
		const action: TimelineAction = onSequence
			? { GoNext: { sequence_id: onSequence } }
			: row === 'variables' && $variables[0]
				? { SetVariable: { variable_id: $variables[0].id, op: 'Toggle' } }
				: { StopTimeline: { timeline_id: timeline.id } };
		const events = [...live.events, { id, at_ms: Math.round(at), action }];
		await writeColumns(timeline.id, { events: events.sort((a, b) => a.at_ms - b.at_ms) });
		selectSubject({ kind: 'timeline-event', id });
	}

	/** An event at the playhead, for the panel's button. */
	export async function eventAtPlayhead() {
		await addEvent(Math.round(snapped(playhead() ?? 0)));
	}

	/** Writes an event or a marker where somebody double-clicked its lane. */
	async function onDoubleClick(e: MouseEvent) {
		if (!$canEdit) return;
		const { x, y } = pointerAt(e);
		const frame = editFrame();
		if (frame && handleAt(x, y) === null && x >= frame.rect.left && x <= frame.rect.right && y >= frame.rect.top && y <= frame.rect.bottom) {
			const ms = Math.round(msOf(x, view, width) - frame.track.offset_ms);
			const like = frame.edit.points.find((p) => !p.release)?.value ?? { type: 'Float', value: 0 };
			void commitPoints([...frame.edit.points.filter((p) => p.ms !== ms), { ms, value: valueAt(like, frame.level(y)) }]);
			return;
		}
		if (hit(layout, view, width, x, y)) return;
		const lane = laneAt(layout, y);
		const live = current(timeline.id);
		if (!lane || !live) return;
		const at = Math.round(snapped(msOf(x, view, width)));
		const id = uuid();
		if (lane.kind === 'events') {
			// Like the row it was asked on: a Go on that row's sequence, a stop on the
			// timelines row. Otherwise a Go into the first sequence, or a play where the
			// show has none — which the Inspector turns into whatever was meant.
			const row = lane.rowKeys?.[Math.floor((y - lane.top) / lane.rowHeight)] ?? '';
			await addEvent(at, row);
		} else if (lane.kind === 'markers') {
			await writeColumns(timeline.id, { markers: [...live.markers, { id, at_ms: at, name: 'Marker' }] });
			selectSubject({ kind: 'timeline-marker', id });
		}
	}

	async function onKeyDown(e: KeyboardEvent) {
		if (endMenu && e.key === 'Escape') {
			endMenu = null;
			return;
		}
		if ((e.key === 'Delete' || e.key === 'Backspace') && $canEdit && editing?.chosen != null) {
			e.preventDefault();
			const gone = editing.chosen;
			void commitPoints(editing.points.filter((_, i) => i !== gone));
			return;
		}
		if ((e.key === 'Delete' || e.key === 'Backspace') && $canEdit) {
			const refs = pickedRefs();
			const live = current(timeline.id);
			if (!refs.length || !live) return;
			e.preventDefault();
			beginGesture();
			try {
				await writeColumns(timeline.id, removed(live, refs, segmentIndex));
			} finally {
				endGesture();
			}
			clearTimelineItems();
		} else if (e.key === 'Escape') {
			clearTimelineItems();
		} else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
			const at = playhead() ?? 0;
			void locate(timeline.id, at + (e.key === 'ArrowLeft' ? -1_000 : 1_000));
		} else if (e.key === ' ' && !e.repeat) {
			// Play and stop while the timeline has the focus. Stopped here, so the
			// console's own Space — Go on the cue sheet touched last — does not also fire.
			e.preventDefault();
			e.stopPropagation();
			const live = current(timeline.id);
			if (live) void toggle(live);
		} else if ((e.key === 'm' || e.key === 'M') && !e.metaKey && !e.ctrlKey && !e.altKey) {
			e.preventDefault();
			void markerAtPlayhead();
		} else if ((e.key === 's' || e.key === 'S') && !e.metaKey && !e.ctrlKey && !e.altKey) {
			e.preventDefault();
			void splitAtPlayhead();
		}
	}

	function onWheel(e: WheelEvent) {
		// Vertical wheel inside the lanes scrolls the lanes when they overflow, unless
		// it is a zoom (no modifier and a mostly-vertical wheel on the ruler, or ⌘/Ctrl).
		const { x, y } = pointerAt(e);
		const zooming = e.metaKey || e.ctrlKey || y < RULER_H || (Math.abs(e.deltaY) > Math.abs(e.deltaX) && !e.shiftKey && layout.height <= visibleHeight);
		if (Math.abs(e.deltaX) > Math.abs(e.deltaY) || e.shiftKey) {
			e.preventDefault();
			const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
			view = clampView({ startMs: view.startMs + (d / width) * view.spanMs, spanMs: view.spanMs }, durationMs);
		} else if (zooming) {
			e.preventDefault();
			// A trackpad pinch arrives as a wheel with Ctrl held and small deltas, many of
			// them: the factor is continuous so it follows the fingers.
			const factor = wheelZoomFactor(e.deltaY, e.deltaMode, e.ctrlKey && !e.metaKey);
			view = zoomAt(view, durationMs, msOf(x, view, width), factor);
		} else {
			return;
		}
		saveView();
		dirty = true;
	}

	onMount(() => {
		const observer = new ResizeObserver(() => {
			width = Math.max(80, (box?.clientWidth ?? 800) - LABELS);
			visibleHeight = Math.max(120, scroller?.clientHeight ?? 200);
			dirty = true;
		});
		if (box) observer.observe(box);
		if (scroller) observer.observe(scroller);
		let raf = requestAnimationFrame(function tick() {
			if (dirty || timeline.running) {
				dirty = false;
				draw();
			}
			raf = requestAnimationFrame(tick);
		});
		return () => {
			cancelAnimationFrame(raf);
			if (frame !== null) cancelAnimationFrame(frame);
			cancelLongPress();
			observer.disconnect();
		};
	});

	// Anything here changing is something new to see.
	$effect(() => {
		void layout;
		void proposal;
		void snapping;
		void view;
		void picked;
		void faulted;
		void partial;
		void $outlines;
		void $sequences;
		void $cues;
		void canvasHeight;
		void editing;
		dirty = true;
	});
</script>

<div class="lanes-box" bind:this={box}>
	<div class="zoom" role="group" aria-label="Zoom">
		<button class="btn btn-icon" title="Zoom in" aria-label="Zoom in" onclick={() => zoom(1 / 1.5)}><ZoomIn size={14} /></button>
		<button class="btn btn-icon" title="Zoom out" aria-label="Zoom out" onclick={() => zoom(1.5)}><ZoomOut size={14} /></button>
		<button class="btn btn-icon" title="The whole timeline" aria-label="Fit" onclick={fit}><Maximize size={14} /></button>
	</div>
	<div class="scroller" bind:this={scroller}>
		<div class="inner" style:height="{canvasHeight}px">
			<div class="labels" style:width="{LABELS}px">
				{#each layout.lanes as lane (lane.key)}
					<div
						class="lane-label {lane.kind}"
						style:top="{lane.top}px"
						style:height="{lane.height}px"
						style:--zone={lane.color ?? null}
					>
						{#if lane.kind === 'take'}
							{#each lane.rows as row, i (i)}
								{#if row}<span class="row" style:top="{lane.headHeight + 2 + i * lane.rowHeight}px" title={row}>{row}</span>{/if}
							{/each}
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<div
								class="row-grip"
								title="Drag to make the rows taller or shorter"
								onpointerdown={(e) => startRows(e, lane.rows.length)}
								onpointermove={moveRows}
								onpointerup={endRows}
								onpointercancel={endRows}
							></div>
						{:else if lane.kind === 'events'}
							{#each lane.rows as row, i (i)}
								<span class="row event-row" style:top="{i * lane.rowHeight + 3}px" title={row}>{row}</span>
							{/each}
						{:else}
							<span class="name" title={lane.label}>{lane.label}</span>
							{#if lane.kind === 'audio' && lane.height > CLIP_ROW_H}<span class="detail">{lane.rows.length} overlapping</span>{/if}
						{/if}
					</div>
				{/each}
			</div>
			<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
			<div
				class="surface"
				role="application"
				aria-label="Timeline lanes"
				tabindex="0"
				onpointerdown={onPointerDown}
				onpointermove={onPointerMove}
				onpointerup={onPointerUp}
				onpointercancel={onPointerUp}
				oncontextmenu={onContextMenu}
				ondblclick={onDoubleClick}
				onkeydown={onKeyDown}
				ondragover={onDragOver}
				ondragleave={() => (dropping = false)}
				ondrop={onDrop}
				class:dropping
				onwheel={onWheel}
				title={hoverTitle}
			>
				<canvas bind:this={canvas} style:width="{width}px" style:height="{canvasHeight}px"></canvas>
				{#if endMenu}
					<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
					<div class="catch" onpointerdown={(e) => e.stopPropagation()} onclick={() => (endMenu = null)}></div>
					<div
						class="end-menu"
						role="menu"
						tabindex="-1"
						style:left="{Math.min(Math.max(0, endMenu.x - 210), Math.max(0, width - 220))}px"
						onpointerdown={(e) => e.stopPropagation()}
					>
						<span class="heading">At {label(endHandleMs)}</span>
						<button role="menuitemradio" aria-checked={timeline.end?.then === 'Stop'} class:on={timeline.end?.then === 'Stop'} onclick={() => setEnd({ at_ms: Math.round(endHandleMs), then: 'Stop' })}>Stop here</button>
						<button role="menuitemradio" aria-checked={timeline.end?.then === 'Loop'} class:on={timeline.end?.then === 'Loop'} onclick={() => setEnd({ at_ms: Math.round(endHandleMs), then: 'Loop' })}>Go round to the top</button>
						<button role="menuitemradio" aria-checked={!timeline.end} class:on={!timeline.end} onclick={() => setEnd(null)}>Run until stopped</button>
					</div>
				{/if}
			</div>
		</div>
	</div>
</div>

<style>
	.lanes-box {
		position: relative;
		display: flex;
		flex-direction: column;
		min-height: 0;
		flex: 1;
		border: 1px solid var(--line);
		border-radius: var(--radius);
		overflow: hidden;
		background: #111;
	}
	.scroller {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overflow-x: hidden;
	}
	.inner {
		position: relative;
		display: flex;
	}
	.labels {
		position: relative;
		flex: none;
		border-right: 1px solid var(--line-strong);
		background: var(--bg-chrome);
	}
	.lane-label {
		position: absolute;
		left: 0;
		right: 0;
		display: flex;
		flex-direction: column;
		justify-content: center;
		padding: 0 8px;
		border-left: 3px solid var(--zone, transparent);
		border-bottom: 1px solid #222;
		overflow: hidden;
		font-size: var(--font-xs);
	}
	.lane-label.take {
		justify-content: flex-start;
	}
	.row-grip {
		position: absolute;
		left: 0;
		right: 0;
		bottom: 0;
		height: 6px;
		cursor: ns-resize;
		touch-action: none;
	}
	.row-grip:hover {
		background: rgb(255 255 255 / 12%);
	}
	.lane-label .name {
		color: var(--text);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.lane-label.take .name {
		position: absolute;
		left: 8px;
		right: 6px;
		top: 1px;
		color: var(--src-track);
		font-size: 10px;
		font-weight: 600;
	}
	.lane-label .row {
		position: absolute;
		left: 14px;
		right: 6px;
		color: var(--text-dim);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		font-size: 10px;
		line-height: 16px;
	}
	.lane-label .detail {
		color: var(--text-faint);
		font-size: 10px;
	}
	.lane-label.markers .name {
		color: #f0c674;
	}
	.lane-label.events {
		justify-content: flex-start;
	}
	.lane-label .event-row {
		left: 8px;
		color: var(--cat-playback);
	}
	.lane-label.events .name {
		color: var(--cat-playback);
	}
	.surface {
		position: relative;
		flex: 1;
		touch-action: none;
		outline: none;
	}
	/* A sound file being dragged over: the lanes say they will take it. */
	.surface.dropping {
		box-shadow: inset 0 0 0 2px var(--accent);
		background: color-mix(in srgb, var(--accent) 8%, transparent);
	}
	.surface:focus-visible {
		box-shadow: inset 0 0 0 1px var(--accent);
	}
	canvas {
		display: block;
	}
	.catch {
		position: fixed;
		inset: 0;
		z-index: 19;
	}
	.end-menu {
		position: absolute;
		top: 24px;
		z-index: 20;
		width: 210px;
		padding: 6px;
		display: flex;
		flex-direction: column;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.45);
	}
	.end-menu .heading {
		padding: 4px 8px;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint);
	}
	.end-menu button {
		text-align: left;
		padding: 7px 8px;
		border: none;
		border-radius: 4px;
		background: none;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.end-menu button:hover {
		background: var(--bg-hover);
	}
	.end-menu button.on {
		color: var(--accent);
	}
	.end-menu button.on::before {
		content: '● ';
	}
	.zoom {
		position: absolute;
		right: 6px;
		bottom: 6px;
		z-index: 2;
		display: flex;
		gap: 2px;
		opacity: 0.85;
	}
	.zoom :global(.btn) {
		min-height: 28px;
		min-width: 28px;
		padding: 0 6px;
	}
</style>
