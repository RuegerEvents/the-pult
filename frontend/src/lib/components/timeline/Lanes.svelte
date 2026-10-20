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
	 */

	import { onMount } from 'svelte';

	import ZoomIn from '@lucide/svelte/icons/zoom-in';
	import ZoomOut from '@lucide/svelte/icons/zoom-out';
	import Maximize from '@lucide/svelte/icons/maximize-2';

	import type { AudioClip, AudioDestination, Detected, GridSegment, Marker, Timeline } from '$lib/generated/index.js';
	import { audioNodes, destinationText } from '$lib/destination.js';
	import { clearSubject, openMenu, selectSubject, subject, type ObjectRef } from '$lib/objects.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { peaksFor } from '$lib/stores/peaks.js';
	import { collection } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { outlines, wantOutline } from '$lib/stores/takeOutlines.js';
	import { audioStatus, current, devices, locate, positionOf, writeColumns } from '$lib/stores/timelines.js';
	import { zones } from '$lib/stores/zones.js';
	import { describeAction } from '$lib/timeline/describe.js';
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

	type Props = {
		timeline: Timeline;
		/** The detector's proposal, drawn over the grid while it is being considered. */
		proposal: Detected | null;
		snapping: boolean;
	};
	let { timeline, proposal, snapping }: Props = $props();

	const sequences = collection('sequences');
	const cues = collection('cues');
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
		| { mode: 'band'; x0: number; y0: number; x1: number; y1: number; additive: boolean; before: ObjectRef[] }
		| { mode: 'scrub' };
	let drag = $state<Drag | null>(null);
	let longPress: ReturnType<typeof setTimeout> | null = null;

	const viewKey = $derived(`pult.waveform.${timeline.id}`);
	const nodes = $derived(audioNodes($devices));
	const faulted = $derived($audioStatus?.[timeline.id]?.clip_faults ?? {});

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
			events: timeline.events.map((e) => ({ id: e.id, atMs: e.at_ms })),
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
				const keys = Array.isArray(outline) ? outline : null;
				// One fixture is said once, on the take's own line, and each row is then
				// just what moved — "Jaw", not "Skeleton 1 · Jaw" four times over.
				const fixtures = new Set(keys?.map((k) => k.fixture_id) ?? []);
				const one = fixtures.size === 1 ? [...fixtures][0] : null;
				return {
					id: track.id,
					name: track.name,
					// …unless the take's own name already says it.
					subject: one && !track.name.includes(fixtureName(one)) ? fixtureName(one) : undefined,
					source: track.input_id === null ? 'from the programmer' : 'from an input',
					offsetMs: track.offset_ms,
					keys: keys ? keys.map((k) => ({ label: one ? parameterLabel(k.fixture_id, k.key) : keyLabel(k.fixture_id, k.key), fromMs: k.from_ms, toMs: k.to_ms })) : null
				};
			})
		});
	});

	const canvasHeight = $derived(Math.max(layout.height, visibleHeight));

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

		// The end: a stop is a wall, a loop is where it goes round.
		if (timeline.end) {
			const x = Math.round(xOf(timeline.end.at_ms, view, width));
			ctx.fillStyle = 'rgba(0,0,0,0.35)';
			if (timeline.end.then === 'Stop') ctx.fillRect(x, RULER_H, Math.max(0, width - x), H - RULER_H);
			ctx.strokeStyle = timeline.end.then === 'Loop' ? '#34d399' : '#888';
			ctx.setLineDash([4, 3]);
			ctx.beginPath();
			ctx.moveTo(x + 0.5, 0);
			ctx.lineTo(x + 0.5, H);
			ctx.stroke();
			ctx.setLineDash([]);
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
			ctx.strokeStyle = fault ? '#ef4444' : chosen ? '#cfe0ff' : (zoneColor ?? '#2f6fd0');
			ctx.lineWidth = chosen ? 2 : 1;
			ctx.strokeRect(left + 0.5, r.top + 0.5, right - left - 1, r.bottom - r.top - 1);
			ctx.lineWidth = 1;
			ctx.fillStyle = fault ? '#ef4444' : '#e6efff';
			ctx.font = '11px system-ui, sans-serif';
			ctx.fillText(`${clip.name || 'clip'}${clip.looped ? ' ⟲' : ''}${peaks ? '' : ' — reducing…'}`, left + 5, r.top + 13, Math.max(10, right - left - 10));
		}

		// Takes: a row per key, its movement as a filled line.
		for (const item of layout.items) {
			if (item.ref.kind !== 'timeline-take') continue;
			const track = timeline.tracks.find((t) => t.id === item.ref.id);
			const lane = layout.lanes.find((l) => l.key === item.lane);
			if (!track || !lane) continue;
			const chosen = picked.has(refKey(item.ref));
			const outline = $outlines[track.asset];
			const r = rectOf(item, layout, view, width);
			ctx.fillStyle = chosen ? 'rgba(74,222,128,0.16)' : track.enabled ? 'rgba(74,222,128,0.07)' : 'rgba(255,255,255,0.03)';
			ctx.fillRect(r.left, r.top, Math.max(2, r.right - r.left), r.bottom - r.top);
			if (Array.isArray(outline)) {
				outline.forEach((key, row) => {
					const top = lane.top + lane.headHeight + 2 + row * lane.rowHeight;
					const bottom = top + lane.rowHeight - 3;
					const values = key.levels.map(([, v]) => v);
					const lo = Math.min(0, ...values);
					const hi = Math.max(1, ...values);
					ctx.strokeStyle = track.enabled ? '#4ade80' : '#3f5f48';
					ctx.beginPath();
					key.levels.forEach(([ms, v], i) => {
						const x = xOf(ms + track.offset_ms, view, width);
						const y = bottom - ((v - lo) / (hi - lo)) * (bottom - top);
						// Stepwise, because replay is: a point holds until the next one.
						if (i === 0) ctx.moveTo(x, y);
						else {
							const prev = key.levels[i - 1];
							ctx.lineTo(x, bottom - ((prev[1] - lo) / (hi - lo)) * (bottom - top));
							ctx.lineTo(x, y);
						}
					});
					ctx.stroke();
				});
			} else {
				ctx.fillStyle = '#666';
				ctx.font = '10px system-ui, sans-serif';
				ctx.fillText(outline === 'failed' ? 'this station has not got the take yet' : 'reading…', 8, lane.top + lane.headHeight + 13);
			}
			if (chosen) {
				ctx.strokeStyle = '#bbf7d0';
				ctx.strokeRect(r.left + 0.5, r.top + 0.5, Math.max(2, r.right - r.left) - 1, r.bottom - r.top - 1);
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
			const x = Math.round(xOf(event.at_ms, view, width)) + 0.5;
			if (x < -150 || x > width + 150) continue;
			const chosen = picked.has(`timeline-event:${event.id}`);
			ctx.strokeStyle = chosen || hovering === event.id ? '#fff' : 'rgba(52,211,153,0.5)';
			ctx.beginPath();
			ctx.moveTo(x, eventsLane.top);
			ctx.lineTo(x, H);
			ctx.stroke();
			const mid = eventsLane.top + eventsLane.height / 2;
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
			ctx.fillText(describeAction(event.action, $sequences, $cues), x + 9, mid + 4);
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
		if (drag.mode === 'move') {
			// What snaps is the grabbed item's own position, not the pointer, which was
			// taken somewhere in the middle of it.
			const delta = snapped(drag.anchorMs + rawMs - drag.fromMs) - drag.anchorMs;
			await writeColumns(timeline.id, moved(live, drag.grabbed, delta, segmentIndex, takeStart));
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
		const outline = track ? $outlines[track.asset] : null;
		return Array.isArray(outline) && outline.length ? Math.min(...outline.map((k) => k.from_ms)) : 0;
	}

	function cancelLongPress() {
		if (longPress) clearTimeout(longPress);
		longPress = null;
	}

	function onPointerDown(e: PointerEvent) {
		if (e.button !== 0 && e.pointerType === 'mouse') return;
		const { x, y } = pointerAt(e);
		box?.focus();
		const item = hit(layout, view, width, x, y);
		const additive = e.shiftKey || e.metaKey || e.ctrlKey;
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
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
		const grabbed = grabbedFrom(live, refs, segmentIndex);
		const anchorMs = item.fromMs;
		drag = { mode: 'move', grabbed, fromMs, anchorMs };
	}

	function onPointerMove(e: PointerEvent) {
		const { x, y } = pointerAt(e);
		if (!drag) {
			const item = hit(layout, view, width, x, y);
			hovering = item?.ref.id ?? null;
			if (box) {
				box.style.cursor = !item
					? y < RULER_H
						? 'col-resize'
						: 'crosshair'
					: item.ref.kind === 'timeline-clip' && gripOf(item, x) !== 'body'
						? 'ew-resize'
						: 'grab';
			}
			dirty = true;
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
		schedule(msOf(x, view, width));
	}

	function onPointerUp(e: PointerEvent) {
		cancelLongPress();
		const was = drag;
		drag = null;
		dirty = true;
		try {
			(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
		} catch {
			// Already released, by a cancel.
		}
		if (!was) return;
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
		if (was.mode === 'move' || was.mode === 'trim') endGesture();
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

	/** Writes an event or a marker where somebody double-clicked its lane. */
	async function onDoubleClick(e: MouseEvent) {
		if (!$canEdit) return;
		const { x, y } = pointerAt(e);
		if (hit(layout, view, width, x, y)) return;
		const lane = laneAt(layout, y);
		const live = current(timeline.id);
		if (!lane || !live) return;
		const at = Math.round(snapped(msOf(x, view, width)));
		const id = crypto.randomUUID();
		if (lane.kind === 'events') {
			const sequence = $sequences[0];
			if (!sequence) return;
			const events = [...live.events, { id, at_ms: at, action: { GoNext: { sequence_id: sequence.id } } }];
			await writeColumns(timeline.id, { events: events.sort((a, b) => a.at_ms - b.at_ms) });
			selectSubject({ kind: 'timeline-event', id });
		} else if (lane.kind === 'markers') {
			await writeColumns(timeline.id, { markers: [...live.markers, { id, at_ms: at, name: 'Marker' }] });
			selectSubject({ kind: 'timeline-marker', id });
		}
	}

	async function onKeyDown(e: KeyboardEvent) {
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
			view = zoomAt(view, durationMs, msOf(x, view, width), e.deltaY > 0 ? 1.2 : 1 / 1.2);
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
		void $outlines;
		void $sequences;
		void $cues;
		void canvasHeight;
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
							<span class="name" title="{lane.label} — {lane.detail}">{lane.label}</span>
							{#each lane.rows as row, i (i)}
								{#if row}<span class="row" style:top="{lane.headHeight + 2 + i * lane.rowHeight}px" title={row}>{row}</span>{/if}
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
				onwheel={onWheel}
			>
				<canvas bind:this={canvas} style:width="{width}px" style:height="{canvasHeight}px"></canvas>
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
	.lane-label.events .name {
		color: var(--cat-playback);
	}
	.surface {
		position: relative;
		flex: 1;
		touch-action: none;
		outline: none;
	}
	.surface:focus-visible {
		box-shadow: inset 0 0 0 1px var(--accent);
	}
	canvas {
		display: block;
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
