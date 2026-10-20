<script lang="ts">
	/**
	 * The effect's shape, and the handles that change it.
	 *
	 * Two cycles of the curve with a dot per fixture where it sits now. The picture is
	 * the control: drag across it to make it faster or slower, drag the dashed line to
	 * change a square's duty or a storm's window, drag the two levels to set where it
	 * travels between. The numbers beside it are for precision, not for getting there.
	 *
	 * Every drag is measured from where it started, so a long drag does not gather
	 * rounding, and is one gesture.
	 */

	import type { EffectLayer, ParameterValue } from '$lib/generated/index.js';
	import { curveAt, cyclePosition, stepValue } from '$lib/effects.js';
	import { draggedLevel, draggedRate, draggedWidth, layerPhases } from '$lib/effectPool.js';
	import { loadEvaluator, shapeLevel } from '$lib/evaluator.js';
	import { asOneGesture } from '$lib/stores/gesture.js';

	let {
		layer,
		rateHz,
		count,
		t0,
		onrate,
		onwidth,
		onlow,
		onhigh
	}: {
		layer: EffectLayer;
		rateHz: number;
		count: number;
		t0: number;
		onrate: (hz: number) => void;
		onwidth: (width: number) => void;
		onlow: (value: ParameterValue) => void;
		onhigh: (value: ParameterValue) => void;
	} = $props();

	const CYCLES = 2;
	let box = $state<HTMLDivElement | null>(null);
	let ready = $state(false);
	$effect(() => {
		void loadEvaluator().then((loaded) => (ready = !!loaded));
	});

	const isShape = $derived('Shape' in layer.curve);
	const shape = $derived('Shape' in layer.curve ? layer.curve.Shape : null);
	const floatEnds = $derived(layer.low.type === 'Float' && layer.high.type === 'Float');
	const lowLevel = $derived(layer.low.type === 'Float' ? layer.low.value : 0);
	const highLevel = $derived(layer.high.type === 'Float' ? layer.high.value : 1);
	const hasWidth = $derived(shape === 'Square' || shape === 'Lightning');

	/** 0..1 on the screen, for a cycle position: a shape between its ends, steps as values. */
	function level(x: number): number {
		const curve = layer.curve;
		if ('Steps' in curve) {
			const v = stepValue(curve.Steps, ((x % 1) + 1) % 1);
			if (v && v.type === 'Float') return v.value;
			return curveAt(curve, layer.width, ((x % 1) + 1) % 1);
		}
		const raw =
			curve.Shape === 'Flicker' || curve.Shape === 'Lightning'
				? (shapeLevel(curve.Shape, layer.width, x) ?? 0)
				: curveAt(curve, layer.width, ((x % 1) + 1) % 1);
		return floatEnds ? lowLevel + (highLevel - lowLevel) * raw : raw;
	}

	const outline = $derived.by(() => {
		void ready;
		const steps = 240;
		const points: string[] = [];
		for (let i = 0; i <= steps; i++) {
			const x = (i / steps) * CYCLES;
			points.push(`${((x / CYCLES) * 100).toFixed(2)},${((1 - level(x)) * 100).toFixed(2)}`);
		}
		return points.join(' ');
	});

	// ── Dots, moving at the rate the lights do ────────────────────────────────

	let now = $state(Date.now());
	$effect(() => {
		let frame = 0;
		const tick = () => {
			if (!document.hidden) now = Date.now();
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	const dots = $derived.by(() => {
		const offsets = layerPhases(layer, Math.min(count, 64));
		const backward = layer.direction === 'Backward';
		return offsets.map((phase) => {
			const x = cyclePosition(rateHz, backward, phase, t0, now);
			return { x, y: level(x) };
		});
	});

	// ── Dragging ──────────────────────────────────────────────────────────────

	type Drag =
		| { what: 'rate'; x: number; hz: number }
		| { what: 'width' }
		| { what: 'low' }
		| { what: 'high' };
	let drag = $state<Drag | null>(null);

	function fraction(event: PointerEvent): { x: number; y: number } {
		const r = box?.getBoundingClientRect();
		if (!r) return { x: 0, y: 0 };
		return {
			x: (event.clientX - r.left) / Math.max(r.width, 1),
			y: 1 - (event.clientY - r.top) / Math.max(r.height, 1)
		};
	}

	function start(event: PointerEvent) {
		// Which handle, if any, by what was pressed: one listener, on the element that
		// captures the pointer, so the gesture action sees every drag begin.
		const named = (event.target as HTMLElement).dataset?.drag as Drag['what'] | undefined;
		const what = named ?? 'rate';
		box?.setPointerCapture(event.pointerId);
		drag = what === 'rate' ? { what, x: event.clientX, hz: rateHz } : { what };
	}

	function move(event: PointerEvent) {
		if (!drag) return;
		const at = fraction(event);
		if (drag.what === 'rate') onrate(draggedRate(drag.hz, event.clientX - drag.x));
		else if (drag.what === 'width') onwidth(draggedWidth(at.x * CYCLES));
		else if (drag.what === 'low')
			onlow({ type: 'Float', value: Math.min(draggedLevel(at.y), highLevel) });
		else onhigh({ type: 'Float', value: Math.max(draggedLevel(at.y), lowLevel) });
	}

	function end(event: PointerEvent) {
		drag = null;
		box?.releasePointerCapture?.(event.pointerId);
	}
</script>

<div
	class="wave"
	class:dragging={drag?.what === 'rate'}
	bind:this={box}
	use:asOneGesture
	role="slider"
	tabindex="0"
	aria-label="Rate: drag across the shape"
	aria-valuenow={rateHz}
	onpointerdown={start}
	onpointermove={move}
	onpointerup={end}
	onpointercancel={end}
>
	<svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
		<line class="cycle" x1="50" y1="0" x2="50" y2="100" />
		<polyline points={outline} />
	</svg>

	{#each dots as dot, i (i)}
		<span class="dot" style:left="{(dot.x / CYCLES) * 100}%" style:bottom="{dot.y * 100}%"></span>
	{/each}

	{#if floatEnds && isShape}
		<button
			class="level high"
			style:bottom="{highLevel * 100}%"
			aria-label="High: drag up and down"
			data-drag="high"
		>{Math.round(highLevel * 100)}</button>
		<button
			class="level low"
			style:bottom="{lowLevel * 100}%"
			aria-label="Low: drag up and down"
			data-drag="low"
		>{Math.round(lowLevel * 100)}</button>
	{/if}

	{#if hasWidth}
		<button
			class="width"
			style:left="{(layer.width / CYCLES) * 100}%"
			aria-label="Width: drag left and right"
			data-drag="width"
		></button>
	{/if}

	<span class="rate">{rateHz.toFixed(2)} Hz · drag to change</span>
</div>

<style>
	.wave {
		position: relative;
		height: 130px;
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-sunken);
		cursor: ew-resize;
		touch-action: none;
		user-select: none;
	}
	.wave:focus-visible {
		outline: 1px solid var(--accent);
	}
	.wave.dragging {
		border-color: var(--accent);
	}
	svg {
		width: 100%;
		height: 100%;
		display: block;
	}
	polyline {
		fill: none;
		stroke: var(--accent);
		stroke-width: 1.5;
		vector-effect: non-scaling-stroke;
	}
	.cycle {
		stroke: var(--line);
		stroke-dasharray: 2 3;
		vector-effect: non-scaling-stroke;
	}
	.dot {
		position: absolute;
		width: 10px;
		height: 10px;
		margin: 0 0 -5px -5px;
		border-radius: 50%;
		background: var(--live);
		pointer-events: none;
	}
	.level {
		position: absolute;
		right: 4px;
		transform: translateY(50%);
		min-width: 30px;
		height: 20px;
		padding: 0 6px;
		font-size: var(--font-xs);
		border: 1px solid var(--accent);
		border-radius: var(--radius);
		background: var(--bg);
		color: var(--accent);
		cursor: ns-resize;
		touch-action: none;
	}
	.level.low {
		right: 40px;
	}
	.width {
		position: absolute;
		top: 0;
		bottom: 0;
		width: 12px;
		margin-left: -6px;
		border: none;
		background: transparent;
		border-left: 2px dashed var(--accent);
		cursor: col-resize;
		touch-action: none;
		transform: translateX(5px);
	}
	.rate {
		position: absolute;
		left: 6px;
		top: 4px;
		font-size: var(--font-xs);
		color: var(--text-dim);
		pointer-events: none;
	}
</style>
