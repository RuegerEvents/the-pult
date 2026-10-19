<script lang="ts">
	/**
	 * The picture of the song, and what is written against it.
	 *
	 * Drawn on a canvas rather than as elements, for the reason `beam.ts` draws beams
	 * on the GPU rather than as geometry: a four-minute song is 24 000 waveform columns
	 * and several thousand beat lines, and a DOM node per column is a panel that stops
	 * being draggable.
	 *
	 * **Redrawn when something changes, and while the playhead is moving.** A stopped
	 * timeline costs one draw when something is edited; a running one costs one an
	 * animation frame, because the playhead is arithmetic over an anchor and nothing on
	 * the wire carries a moving position. The rule the rig view already follows.
	 *
	 * **The playhead shows a gap until {@link consoleNow} has an offset**, exactly as
	 * the readout above it does: a page evaluating an anchor against an unadjusted
	 * `Date.now()` draws a playhead out by however wrong this machine's clock is,
	 * silently.
	 *
	 * Dragging an event, a marker or a clip is **one gesture and one write per
	 * animation frame**, which is `stores/editor.ts`'s rule for dragging a truss applied
	 * to dragging a Go: `events` and `clips` are one column each, so each of those
	 * frames rewrites the whole array, and without the gesture putting one back would
	 * be a key somebody holds down.
	 *
	 * **Clips are blocks on lanes**, a lane per clip that would otherwise overlap the
	 * one before it, each drawing its own file's waveform over the part of the file it
	 * plays. The body moves a clip, an edge trims it — see `dragClip` in
	 * `$lib/waveform.ts` for what a trim keeps still.
	 */

	import { onMount } from 'svelte';

	import type {
		AudioClip,
		Detected,
		GridSegment,
		Marker,
		Timeline,
		TimelineEvent
	} from '$lib/generated/index.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { peaksFor } from '$lib/stores/peaks.js';
	import {
		clampView,
		clipLanes,
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
	import { getDataContext } from '$lib/ws/context.js';

	type Props = {
		timeline: Timeline;
		/** The detector's proposal, drawn over the grid while it is being considered. */
		proposal: Detected | null;
		snapping: boolean;
		onLocate: (ms: number) => void;
		/** The clip the panel below is showing, drawn picked out. */
		selectedClip?: string | null;
		onSelectClip?: (id: string) => void;
		/** Clips the station says are not playing, by id, drawn in red. */
		faulted?: Partial<Record<string, string>>;
	};
	let {
		timeline,
		proposal,
		snapping,
		onLocate,
		selectedClip = null,
		onSelectClip,
		faulted = {}
	}: Props = $props();

	const data = getDataContext();

	let canvas = $state<HTMLCanvasElement | null>(null);
	let box = $state<HTMLDivElement | null>(null);
	let width = $state(800);
	/** What is on screen. Per browser and per timeline, in `localStorage`. */
	let view = $state<View>({ startMs: 0, spanMs: 60_000 });
	type Grab =
		| { kind: 'event' | 'marker'; id: string }
		| {
				kind: 'clip';
				id: string;
				grip: ClipGrip;
				/** Where the pointer went down, unsnapped. */
				fromMs: number;
				/** The clip as it was when the drag began, which every frame is measured from. */
				grabbed: Pick<AudioClip, 'start_ms' | 'offset_ms' | 'length_ms'>;
		  };
	let dragging = $state<Grab | null>(null);
	let hovering = $state<string | null>(null);

	/** The ruler strip above the lanes, where events and markers have their flags. */
	const RULER = 34;
	const LANE = 46;
	/** How near a dragged position has to be, on the screen, to take a grid line. */
	const SNAP_PX = 7;

	const viewKey = $derived(`pult.waveform.${timeline.id}`);

	/**
	 * How long the song is, or — with no audio — as far as anything is written.
	 *
	 * A timeline with no audio still gets a ruler to drag events against, which is the
	 * whole of what "timecode without timecode" means: the picture is empty and the
	 * positions are real.
	 */
	const durationMs = $derived.by(() => {
		const written = Math.max(
			0,
			...timeline.events.map((e) => e.at_ms),
			...timeline.markers.map((m) => m.at_ms),
			timeline.end?.at_ms ?? 0
		);
		const sounding = Math.max(
			0,
			...timeline.clips.filter((c) => !c.looped).map((c) => clipSpan(c, fileMs(c), 0).toMs)
		);
		return Math.max(sounding, written + 10_000, 30_000);
	});

	/** How long a clip's file is, which a browser knows from the peaks and nothing else. */
	function fileMs(clip: AudioClip): number | null {
		const peaks = clip.peaks ? peaksFor(clip.peaks, () => (dirty = true)) : null;
		return peaks?.durationMs ?? null;
	}

	/** Each clip's span on the timeline and its lane, in the order the timeline holds them. */
	const laid = $derived.by(() => {
		const until = timeline.end?.at_ms ?? durationMs;
		const spans = timeline.clips.map((clip) => clipSpan(clip, fileMs(clip), until));
		const lanes = clipLanes(spans);
		return timeline.clips.map((clip, i) => ({ clip, span: spans[i], lane: lanes[i] }));
	});
	const laneCount = $derived(Math.max(1, ...laid.map((l) => l.lane + 1)));
	const HEIGHT = $derived(RULER + laneCount * LANE + 8);

	let dirty = $state(true);

	function saveView() {
		try {
			localStorage.setItem(viewKey, JSON.stringify(view));
		} catch {
			// A private window, or storage the browser will not give. A remembered zoom
			// is a convenience and never a reason for a panel not to draw.
		}
	}

	function loadView() {
		try {
			const raw = localStorage.getItem(viewKey);
			if (raw) view = clampView(JSON.parse(raw) as View, durationMs);
		} catch {
			view = { startMs: 0, spanMs: Math.min(60_000, durationMs) };
		}
	}

	/** Where the playhead is, or `null` until this browser knows the station's clock. */
	function playhead(): number | null {
		const now = consoleNow();
		if (now === null) return null;
		if (!timeline.running) return timeline.position_at_anchor_ms;
		if (now <= timeline.anchor_ms) return timeline.position_at_anchor_ms;
		return timeline.position_at_anchor_ms + (now - timeline.anchor_ms) * timeline.rate;
	}

	/** Everything a dragged position may land on. */
	function snapTargets(): SnapTarget[] {
		if (!snapping) return [];
		return [
			...gridLines(timeline.grid as GridSegment[], view.startMs, view.startMs + view.spanMs, 2_000),
			...timeline.markers.map((marker: Marker) => ({
				atMs: marker.at_ms,
				what: 'marker' as const
			}))
		];
	}

	function draw() {
		if (!canvas) return;
		const context = canvas.getContext('2d');
		if (!context) return;
		const scale = window.devicePixelRatio || 1;
		if (canvas.width !== Math.round(width * scale) || canvas.height !== Math.round(HEIGHT * scale)) {
			canvas.width = Math.round(width * scale);
			canvas.height = Math.round(HEIGHT * scale);
		}
		context.setTransform(scale, 0, 0, scale, 0, 0);
		context.clearRect(0, 0, width, HEIGHT);
		context.fillStyle = '#121212';
		context.fillRect(0, 0, width, HEIGHT);

		// The sound: a block per clip on its lane, and inside each the part of its own
		// file it plays. One vertical line per screen column, folding however many bins
		// fall in it — which is what makes a zoomed-out view cheap: the work is the
		// width of the panel and not the length of the song.
		for (const { clip, span, lane } of laid) {
			const left = Math.max(0, xOf(span.fromMs, view, width));
			const right = Math.min(width, xOf(span.toMs, view, width));
			if (right < 0 || left > width || right - left < 1) continue;
			const top = RULER + lane * LANE + 2;
			const height = LANE - 4;
			const picked = clip.id === selectedClip;
			context.fillStyle = picked ? '#1d3354' : '#18263b';
			context.fillRect(left, top, right - left, height);
			context.strokeStyle = faulted[clip.id] ? '#e05555' : picked ? '#7fb0f5' : '#2f6fd0';
			context.strokeRect(left + 0.5, top + 0.5, right - left - 1, height - 1);

			const peaks = clip.peaks ? peaksFor(clip.peaks, () => (dirty = true)) : null;
			if (peaks && span.regionMs > 0) {
				const msPerBin = (peaks.samplesPerBin * 1000) / peaks.sampleRate;
				const middle = top + height / 2;
				const scale = height / 2 - 3;
				context.strokeStyle = clip.looped ? '#4a7fc4' : '#5d95e0';
				context.beginPath();
				for (let x = Math.floor(left); x < right; x++) {
					const at = msOf(x, view, width) - clip.start_ms;
					if (at < 0) continue;
					const into = clip.looped ? at % span.regionMs : at;
					if (into >= span.regionMs) continue;
					const fileAt = clip.offset_ms + into;
					const step = Math.max(1, Math.floor(view.spanMs / width / msPerBin));
					const from = Math.floor(fileAt / msPerBin);
					let low = 0;
					let high = 0;
					for (let bin = from; bin < from + step && bin * 2 + 1 < peaks.bins.length; bin++) {
						low = Math.min(low, peaks.bins[bin * 2]);
						high = Math.max(high, peaks.bins[bin * 2 + 1]);
					}
					const gain = Math.min(1, Math.pow(10, clip.gain_db / 20));
					context.moveTo(x + 0.5, middle - high * gain * scale);
					context.lineTo(x + 0.5, middle - low * gain * scale);
				}
				context.stroke();
			}
			context.fillStyle = faulted[clip.id] ? '#e05555' : '#cfe0ff';
			context.font = '11px system-ui, sans-serif';
			const label = `${clip.name || 'clip'}${clip.looped ? ' ⟲' : ''}${peaks ? '' : ' — reducing…'}`;
			context.fillText(label, left + 5, top + 12, Math.max(10, right - left - 10));
		}
		if (timeline.clips.length === 0) {
			context.fillStyle = '#555';
			context.font = '12px system-ui, sans-serif';
			context.fillText('no sound on this timeline', 10, RULER + LANE / 2 + 4);
		}
		// The grid over it: a bar line per downbeat, a tick per beat. Beats are dropped
		// where there would be more of them than pixels, which is not a compromise —
		// four hundred lines across eight hundred pixels is a grey rectangle.
		const lines = gridLines(
			timeline.grid as GridSegment[],
			view.startMs,
			view.startMs + view.spanMs,
			2_000
		);
		const beatsFitFor = lines.length < width / 4;
		for (const line of lines) {
			if (line.what === 'beat' && !beatsFitFor) continue;
			const x = Math.round(xOf(line.atMs, view, width)) + 0.5;
			context.strokeStyle = line.what === 'bar' ? 'rgba(255,255,255,0.28)' : 'rgba(255,255,255,0.09)';
			context.beginPath();
			context.moveTo(x, line.what === 'bar' ? 0 : HEIGHT - 26);
			context.lineTo(x, HEIGHT);
			context.stroke();
		}

		// The detector's proposal, in its own colour and above the grid, because the
		// whole point of drawing it is to be compared with what is already there.
		if (proposal) {
			context.strokeStyle = 'rgba(74,222,128,0.75)';
			for (const at of proposal.downbeats_ms) {
				const x = Math.round(xOf(at, view, width)) + 0.5;
				context.beginPath();
				context.moveTo(x, 0);
				context.lineTo(x, 14);
				context.stroke();
			}
			context.strokeStyle = 'rgba(74,222,128,0.3)';
			if (proposal.beats_ms.length < width / 3) {
				for (const at of proposal.beats_ms) {
					const x = Math.round(xOf(at, view, width)) + 0.5;
					context.beginPath();
					context.moveTo(x, 0);
					context.lineTo(x, 8);
					context.stroke();
				}
			}
		}

		// Markers, then events as flags. Events on top because they are the thing that
		// makes something happen.
		for (const marker of timeline.markers) {
			const x = Math.round(xOf(marker.at_ms, view, width)) + 0.5;
			if (x < -40 || x > width + 40) continue;
			context.strokeStyle = hovering === marker.id ? '#f0c674' : 'rgba(240,198,116,0.7)';
			context.beginPath();
			context.moveTo(x, 16);
			context.lineTo(x, HEIGHT);
			context.stroke();
			context.fillStyle = '#f0c674';
			context.font = '11px system-ui, sans-serif';
			context.fillText(marker.name, x + 4, 26);
		}
		for (const event of timeline.events) {
			const x = Math.round(xOf(event.at_ms, view, width)) + 0.5;
			if (x < -40 || x > width + 40) continue;
			context.strokeStyle = hovering === event.id ? '#fff' : '#e05555';
			context.beginPath();
			context.moveTo(x, 0);
			context.lineTo(x, HEIGHT);
			context.stroke();
			context.fillStyle = hovering === event.id ? '#fff' : '#e05555';
			context.fillRect(x, 0, 9, 9);
		}

		// And the playhead, or a gap where it would be.
		const at = playhead();
		if (at === null) {
			context.fillStyle = '#777';
			context.font = '11px system-ui, sans-serif';
			context.fillText('waiting for the station clock', 10, HEIGHT - 8);
		} else {
			const x = Math.round(xOf(at, view, width)) + 0.5;
			context.strokeStyle = timeline.running ? '#4ade80' : '#bbb';
			context.lineWidth = 2;
			context.beginPath();
			context.moveTo(x, 0);
			context.lineTo(x, HEIGHT);
			context.stroke();
			context.lineWidth = 1;
		}
	}

	/** How near an edge, on the screen, a press has to be to trim rather than move. */
	const EDGE_PX = 6;

	/**
	 * What is under the pointer: an event, a marker, a clip and which part of it, or
	 * the ruler. Over the ruler strip a flag wins; over a lane the clip does, because a
	 * Go drawn through a clip must not make the clip impossible to grab.
	 */
	function pick(x: number, y: number): Grab | null {
		const flag = (): Grab | null => {
			for (const event of timeline.events) {
				if (Math.abs(xOf(event.at_ms, view, width) - x) <= 5) return { kind: 'event', id: event.id };
			}
			for (const marker of timeline.markers) {
				if (Math.abs(xOf(marker.at_ms, view, width) - x) <= 5) return { kind: 'marker', id: marker.id };
			}
			return null;
		};
		if (y < RULER) return flag();
		const lane = Math.floor((y - RULER) / LANE);
		for (const { clip, span, lane: at } of laid) {
			if (at !== lane) continue;
			const left = xOf(span.fromMs, view, width);
			const right = xOf(span.toMs, view, width);
			if (x < left - 2 || x > right + 2) continue;
			const grip: ClipGrip =
				x - left <= EDGE_PX ? 'front' : !clip.looped && right - x <= EDGE_PX ? 'back' : 'body';
			return {
				kind: 'clip',
				id: clip.id,
				grip,
				fromMs: msOf(x, view, width),
				grabbed: { start_ms: clip.start_ms, offset_ms: clip.offset_ms, length_ms: clip.length_ms }
			};
		}
		return flag();
	}

	function pointerAt(e: PointerEvent): { x: number; y: number } {
		const rect = box?.getBoundingClientRect();
		return { x: e.clientX - (rect?.left ?? 0), y: e.clientY - (rect?.top ?? 0) };
	}

	const snapped = (ms: number) => Math.max(0, snap(ms, snapTargets(), view, width, SNAP_PX));

	function positionIn(e: PointerEvent): number {
		return snapped(msOf(pointerAt(e).x, view, width));
	}

	/** Coalesced to one write an animation frame — see the header. */
	let pending: number | null = null;
	let frame: number | null = null;

	/**
	 * A clip after the pointer has moved to `rawMs`. What snaps is the edge being moved —
	 * the front for a move, since that is where a scare's sound begins — and not the
	 * pointer, which was taken somewhere in the middle of the block.
	 */
	function draggedClip(grab: Extract<Grab, { kind: 'clip' }>, rawMs: number): AudioClip | null {
		const clip = timeline.clips.find((c) => c.id === grab.id);
		if (!clip) return null;
		const file = fileMs(clip);
		const delta = rawMs - grab.fromMs;
		const region = clipSpan({ ...clip, ...grab.grabbed }, file, 0).regionMs;
		const edge =
			grab.grip === 'back' ? grab.grabbed.start_ms + region : grab.grabbed.start_ms;
		const landed = snapped(edge + delta) - edge;
		return { ...clip, ...dragClip(grab.grip, 0, landed, file, grab.grabbed) };
	}

	function moveDragged(ms: number) {
		pending = ms;
		if (frame !== null) return;
		frame = requestAnimationFrame(async () => {
			frame = null;
			const at = pending;
			pending = null;
			const grab = dragging;
			if (at === null || !grab) return;
			if (grab.kind === 'clip') {
				const moved = draggedClip(grab, at);
				if (!moved) return;
				await data.timelines
					.byId(timeline.id)
					.clips.set(timeline.clips.map((each: AudioClip) => (each.id === grab.id ? moved : each)));
			} else if (grab.kind === 'event') {
				await data.timelines
					.byId(timeline.id)
					.events.set(
						timeline.events.map((each: TimelineEvent) =>
							each.id === grab.id ? { ...each, at_ms: at } : each
						)
					);
			} else {
				await data.timelines
					.byId(timeline.id)
					.markers.set(
						timeline.markers.map((each: Marker) =>
							each.id === grab.id ? { ...each, at_ms: at } : each
						)
					);
			}
		});
	}

	function onPointerDown(e: PointerEvent) {
		const { x, y } = pointerAt(e);
		const found = pick(x, y);
		if (found) {
			// One gesture for the whole drag, so putting a clip back is one Ctrl-Z rather
			// than sixty.
			dragging = found;
			if (found.kind === 'clip') onSelectClip?.(found.id);
			beginGesture();
			(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
			e.preventDefault();
			return;
		}
		onLocate(positionIn(e));
	}

	function onPointerMove(e: PointerEvent) {
		const { x, y } = pointerAt(e);
		if (!dragging) {
			const found = pick(x, y);
			hovering = found?.id ?? null;
			if (box) {
				box.style.cursor =
					found?.kind === 'clip' ? (found.grip === 'body' ? 'grab' : 'ew-resize') : 'crosshair';
			}
			dirty = true;
			return;
		}
		if (dragging.kind === 'clip') {
			moveDragged(msOf(x, view, width));
			return;
		}
		moveDragged(positionIn(e));
	}

	function onPointerUp(e: PointerEvent) {
		if (!dragging) return;
		dragging = null;
		endGesture();
		(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
	}

	function onWheel(e: WheelEvent) {
		e.preventDefault();
		const rect = box?.getBoundingClientRect();
		const x = e.clientX - (rect?.left ?? 0);
		if (e.shiftKey) {
			// Shift is a scroll, which is what every drawing program does.
			view = clampView(
				{ startMs: view.startMs + (e.deltaY / width) * view.spanMs, spanMs: view.spanMs },
				durationMs
			);
		} else {
			view = zoomAt(view, durationMs, msOf(x, view, width), e.deltaY > 0 ? 1.2 : 1 / 1.2);
		}
		saveView();
		dirty = true;
	}

	onMount(() => {
		loadView();
		const observer = new ResizeObserver((entries) => {
			width = Math.max(80, entries[0].contentRect.width);
			dirty = true;
		});
		if (box) observer.observe(box);

		// One loop, drawing only when there is something new to see — the rule the rig
		// view follows. A running timeline is always something new, because the
		// playhead is arithmetic over a clock.
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
			observer.disconnect();
		};
	});

	// Anything the panel changes is something new to see.
	$effect(() => {
		void timeline;
		void proposal;
		void snapping;
		void view;
		void selectedClip;
		void faulted;
		dirty = true;
	});
</script>

<div
	class="waveform"
	bind:this={box}
	role="slider"
	tabindex="0"
	aria-label="Timeline position"
	aria-valuenow={timeline.position_at_anchor_ms}
	onpointerdown={onPointerDown}
	onpointermove={onPointerMove}
	onpointerup={onPointerUp}
	onpointercancel={onPointerUp}
	onwheel={onWheel}
	onkeydown={(e) => {
		if (e.key === 'ArrowLeft') onLocate(Math.max(0, timeline.position_at_anchor_ms - 1_000));
		if (e.key === 'ArrowRight') onLocate(timeline.position_at_anchor_ms + 1_000);
	}}
>
	<canvas bind:this={canvas} style="width: 100%; height: {HEIGHT}px"></canvas>
</div>
<p class="hint">
	Click to locate, drag an event, a marker or a clip to move it, and a clip's edge to trim it.
	Wheel to zoom, shift-wheel to scroll — both are this browser's and are remembered here.
</p>

<style>
	.waveform {
		border: 1px solid #2a2a2a;
		border-radius: 3px;
		overflow: hidden;
		cursor: crosshair;
		touch-action: none;
	}
	.waveform:focus-visible {
		outline: 1px solid #2f6fd0;
	}
	canvas {
		display: block;
	}
	.hint {
		color: #666;
		font-size: 11px;
		margin: 4px 0 0;
	}
</style>
