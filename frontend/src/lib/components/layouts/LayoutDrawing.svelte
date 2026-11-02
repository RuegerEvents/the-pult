<script lang="ts">
	/**
	 * A layout, drawn: the backdrop, the pieces and every reading painted where it is.
	 *
	 * The desk's panel and the page a phone is handed are this one component, so what
	 * the desk draws is what the phone draws; neither presses anything through it. What
	 * each element looks like is `elementLook` in `$lib/layoutView.ts` — this only puts
	 * it on the screen.
	 *
	 * **It draws frames only while something on it is moving.** A contact fading out, a
	 * timeline counting down: an animation-frame loop runs exactly while one of them is
	 * on the drawing and stops when the last one settles. A maze with nobody in it, on a
	 * phone in a pocket, costs nothing — the rule the rig view and the surface follow.
	 * The one exception is a driven parameter, which is the output store's frame loop
	 * and is subscribed to only while the layout has one: what the rig is putting out is
	 * evaluated per frame, and there is no cheaper way to know a fade has moved.
	 */

	import type { AudioClip, Layout, LayoutElement } from '$lib/generated/index.js';
	import {
		boxOf,
		drivenKeys,
		elementLook,
		labelSize,
		noteFlag,
		type FlagMemory,
		type LayoutShow,
		type Look,
		type Size,
		type View
	} from '$lib/layoutView.js';
	import { NOTHING_YET, output, watching, type Showing } from '$lib/stores/output.js';
	import { peaksFor } from '$lib/stores/peaks.js';
	import { clock as stationClock, consoleNow } from '$lib/ws/clock.js';
	import { getClientContext } from '$lib/ws/context.js';

	let {
		layout,
		show,
		view,
		screen,
		selected = new Set<string>(),
		overrides = null
	}: {
		layout: Layout;
		show: LayoutShow;
		/** Where the canvas is on the screen. */
		view: View;
		/** The drawing's own size in CSS pixels. */
		screen: Size;
		/** Elements drawn outlined: the Inspector's subject. */
		selected?: Set<string>;
		/**
		 * Elements as a drag has them this frame, by id, drawn in place of the stored ones
		 * so the drawing follows the pointer rather than the round trip.
		 */
		overrides?: Map<string, LayoutElement> | null;
	} = $props();

	const client = getClientContext();

	// ── What the rig is putting out, for the elements that show it ───────────

	const keys = $derived(drivenKeys(layout, show));
	let showing = $state.raw<Showing>(NOTHING_YET);
	$effect(() => {
		if (keys.length === 0) {
			showing = NOTHING_YET;
			return;
		}
		const registered = watching(keys);
		const stop = output.subscribe((s) => (showing = s));
		return () => {
			stop();
			registered.stop();
		};
	});

	// ── The clock, and frames while something is moving ─────────────────────

	let frame = $state(0);
	/** The console millisecond, read again whenever anything this draws from changes. */
	const now = $derived.by(() => {
		void frame;
		void $stationClock;
		void layout;
		void show;
		void showing;
		return consoleNow();
	});

	/** What this page has seen of each flag: see `noteFlag` on why a variable needs it. */
	const flags: FlagMemory = new Map();

	let peaksArrived = $state(0);
	function fileMs(clip: AudioClip): number | null {
		void peaksArrived;
		const peaks = clip.peaks ? peaksFor(clip.peaks, () => peaksArrived++) : null;
		return peaks?.durationMs ?? null;
	}

	const elements = $derived(
		overrides?.size ? layout.elements.map((e) => overrides.get(e.id) ?? e) : layout.elements
	);

	const looks = $derived(
		elements.map((element) => ({
			element,
			box: boxOf(element),
			look: elementLook(element, layout, show, {
				now,
				driven: (fixtureId, key) => showing.value(fixtureId, key),
				flagOffAt: (id, on) => noteFlag(flags, id, on, now),
				fileMs
			})
		}))
	);

	const moving = $derived(looks.some((l) => l.look.animating));
	$effect(() => {
		if (!moving) return;
		let handle = requestAnimationFrame(function tick() {
			frame++;
			handle = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(handle);
	});

	const backdrop = $derived(layout.backdrop ? client.httpUrl(`/assets/${layout.backdrop}`) : null);

	/**
	 * Where an element's words go. Inside the box when they can be read there; under it
	 * when the box is a dot too small to hold its own name, which is most sensors.
	 */
	function words(box: { x: number; y: number; w: number; h: number }, look: Look) {
		const two = look.label !== '' && look.figure !== null;
		const inLabel = look.label ? labelSize(box, look.label, two) : 0;
		const inFigure = look.figure ? labelSize(box, look.figure, two) : 0;
		const cx = box.x + box.w / 2;
		const cy = box.y + box.h / 2;
		const outside = look.label !== '' && inLabel < box.h * 0.2;
		if (outside) {
			const size = Math.max(box.h * 0.3, 1);
			return {
				label: { x: cx, y: box.y + box.h + size * 0.75, size, outside: true },
				figure: look.figure ? { x: cx, y: cy, size: labelSize(box, look.figure) } : null
			};
		}
		if (two) {
			return {
				label: { x: cx, y: cy - box.h * 0.17, size: inLabel, outside: false },
				figure: { x: cx, y: cy + box.h * 0.2, size: inFigure }
			};
		}
		return {
			label: look.label ? { x: cx, y: cy, size: inLabel, outside: false } : null,
			figure: look.figure ? { x: cx, y: cy, size: inFigure } : null
		};
	}

	const corner = (box: { w: number; h: number }) => Math.min(box.w, box.h) * 0.12;
</script>

<svg width={screen.width} height={screen.height} class="drawing" role="img" aria-label={layout.name}>
	<g transform="translate({view.x} {view.y}) scale({view.scale})">
		<rect class="canvas" width={layout.width} height={layout.height} />
		{#if backdrop}
			<image href={backdrop} width={layout.width} height={layout.height} preserveAspectRatio="none" />
		{/if}
		{#each looks as { element, box, look } (element.id)}
			{@const at = words(box, look)}
			{@const outlined = selected.has(element.id)}
			<g class="element" class:missing={look.missing}>
				{#if element.target.kind === 'Line'}
					<polyline
						points={element.target.points.map((p) => p.join(',')).join(' ')}
						fill="none"
						stroke={look.stroke}
						stroke-width="3"
						stroke-linecap="round"
						stroke-linejoin="round"
						vector-effect="non-scaling-stroke"
					/>
					{#if outlined}
						<rect class="frame" x={box.x} y={box.y} width={box.w} height={box.h} vector-effect="non-scaling-stroke" />
					{/if}
				{:else if element.target.kind === 'Text'}
					{#if outlined}
						<rect class="frame" x={box.x} y={box.y} width={box.w} height={box.h} vector-effect="non-scaling-stroke" />
					{/if}
				{:else if element.shape === 'Ellipse'}
					<ellipse
						cx={box.x + box.w / 2}
						cy={box.y + box.h / 2}
						rx={box.w / 2}
						ry={box.h / 2}
						fill={look.fill}
						class:outline={outlined}
						vector-effect="non-scaling-stroke"
					/>
				{:else}
					<rect
						x={box.x}
						y={box.y}
						width={box.w}
						height={box.h}
						rx={corner(box)}
						fill={look.fill}
						class:outline={outlined}
						vector-effect="non-scaling-stroke"
					/>
				{/if}
				{#if look.fraction !== null}
					<!-- How far through, along the foot: the countdown's bar. -->
					<rect
						class="progress"
						x={box.x}
						y={box.y + box.h * 0.86}
						width={box.w * look.fraction}
						height={box.h * 0.14}
						fill={look.ink}
					/>
				{/if}
				{#if at.label}
					<text
						x={at.label.x}
						y={at.label.y}
						font-size={at.label.size}
						fill={at.label.outside ? '#e5e7eb' : look.ink}
						class:outside={at.label.outside}
						text-anchor="middle"
						dominant-baseline="central">{look.label}</text
					>
				{/if}
				{#if at.figure}
					<text
						class="figure"
						x={at.figure.x}
						y={at.figure.y}
						font-size={at.figure.size}
						fill={look.ink}
						text-anchor="middle"
						dominant-baseline="central">{look.figure}</text
					>
				{/if}
			</g>
		{/each}
	</g>
</svg>

<style>
	.drawing {
		display: block;
		user-select: none;
		-webkit-user-select: none;
	}
	.canvas {
		fill: #15181d;
		stroke: #2a2f37;
		stroke-width: 1px;
		vector-effect: non-scaling-stroke;
	}
	text {
		font-family: inherit;
		font-weight: 600;
		pointer-events: none;
	}
	/* Words under a dot sit on whatever is behind them, so they carry their own edge. */
	text.outside {
		paint-order: stroke;
		stroke: #0d0f12;
		stroke-width: 3px;
		stroke-linejoin: round;
	}
	.figure {
		font-variant-numeric: tabular-nums;
		font-weight: 700;
	}
	.progress {
		opacity: 0.35;
	}
	.outline {
		stroke: var(--accent, #4a90d9);
		stroke-width: 2px;
	}
	.frame {
		fill: none;
		stroke: var(--accent, #4a90d9);
		stroke-width: 2px;
		stroke-dasharray: 5 3;
	}
	.missing rect,
	.missing ellipse {
		stroke: #888;
		stroke-width: 1.5px;
		stroke-dasharray: 4 3;
		opacity: 0.6;
	}
</style>
