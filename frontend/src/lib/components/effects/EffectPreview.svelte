<script lang="ts">
	/**
	 * What an effect does across a selection, before it is applied and after.
	 *
	 * One cell per fixture in selection order, evaluated by the wasm evaluator — the
	 * arithmetic the lamps take — so the row shows what the rig *will* do rather than
	 * what this file thinks a flicker looks like. Every layer is drawn together, the
	 * color a fixture would take scaled by the intensity it would take, because fire
	 * is the two of them at once. A strip's cells sit edge to edge, as a line of light,
	 * and a row of heads gets gaps between them.
	 *
	 * It animates only while it can be seen: a preview scrolled out of sight or in a
	 * background tab asks for nothing. Nothing it draws reaches an output.
	 *
	 * The handle across the top is the spread: drag it right and the effect covers
	 * more cycles across the row. That is "segments" on a strip and "how far apart"
	 * on a row of heads, and it is one number either way.
	 */

	import type { EffectLayer, RunningEffect } from '$lib/generated/index.js';
	import { effectValues, loadEvaluator, STRIDE } from '$lib/evaluator.js';
	import { draggedCycles, type LayerApply } from '$lib/effectPool.js';
	import { parameterKey } from '$lib/patch.js';
	import { asOneGesture } from '$lib/stores/gesture.js';

	let {
		order,
		applies,
		rateOf,
		layer,
		t0,
		strip = false,
		oncycles
	}: {
		/** The selection, in its order: one cell each. */
		order: string[];
		/** What Apply would write, layer by layer. */
		applies: LayerApply[];
		/** A layer's rate in hertz, by its index, with any speed master resolved. */
		rateOf: (layer: number) => number;
		/** The layer being edited, whose spread the handle sets. */
		layer: EffectLayer;
		t0: number;
		/** Cells edge to edge, as the pixels of one strip. */
		strip?: boolean;
		oncycles?: (cycles: number) => void;
	} = $props();
	const count = $derived(order.length);

	let canvas = $state<HTMLCanvasElement | null>(null);
	let row = $state<HTMLDivElement | null>(null);
	let visible = $state(false);
	let ready = $state(false);

	$effect(() => {
		void loadEvaluator().then((loaded) => (ready = !!loaded));
	});

	$effect(() => {
		if (!row) return;
		const seen = new IntersectionObserver((entries) => {
			visible = entries.some((e) => e.isIntersecting);
		});
		seen.observe(row);
		return () => seen.disconnect();
	});

	/** One list per layer, in selection order where the layer reaches, for one call each. */
	const runs = $derived(
		applies.map((apply) => {
			const key = parameterKey(apply.kind);
			const hz = rateOf(apply.layer);
			const at = new Map(apply.order.map((id, i) => [id, i]));
			const effects: RunningEffect[] = apply.order.map((id) => {
				const spec = apply.specs[id];
				return {
					effect_id: spec.effect_id,
					curve: spec.curve,
					rate_hz: hz,
					low: spec.low,
					high: spec.high,
					width: spec.width,
					direction: spec.direction,
					phase: spec.phase,
					t0,
					source: 'Programmer'
				};
			});
			return { key, at, effects };
		})
	);

	function draw(now: number) {
		const c = canvas;
		if (!c) return;
		const ctx = c.getContext('2d');
		if (!ctx) return;
		const width = c.clientWidth;
		const height = c.clientHeight;
		const scale = window.devicePixelRatio || 1;
		if (c.width !== Math.round(width * scale) || c.height !== Math.round(height * scale)) {
			c.width = Math.round(width * scale);
			c.height = Math.round(height * scale);
		}
		ctx.setTransform(scale, 0, 0, scale, 0, 0);
		ctx.clearRect(0, 0, width, height);
		const n = count;
		if (n === 0) return;
		// Per fixture: a color, white unless a layer says otherwise, and a level.
		const r = new Float32Array(n).fill(1);
		const g = new Float32Array(n).fill(1);
		const b = new Float32Array(n).fill(1);
		const level = new Float32Array(n).fill(1);
		const reached = new Uint8Array(n);
		const index = new Map(order.map((id, i) => [id, i]));
		for (const run of runs) {
			const packed = effectValues(run.effects, now);
			if (!packed) continue;
			for (const [id, k] of run.at) {
				const i = index.get(id);
				if (i === undefined) continue;
				reached[i] = 1;
				const tag = packed[k * STRIDE];
				const a = packed[k * STRIDE + 1];
				if (tag === 4) {
					r[i] = a;
					g[i] = packed[k * STRIDE + 2];
					b[i] = packed[k * STRIDE + 3];
				} else if (tag === 3) {
					level[i] *= a ? 1 : 0.08;
				} else if (tag === 1 || tag === 2) {
					level[i] *= Math.min(1, Math.max(0, a));
				}
			}
		}
		const gap = strip || n > 48 ? 0 : 3;
		const cell = (width - gap * (n - 1)) / n;
		for (let i = 0; i < n; i++) {
			const x = i * (cell + gap);
			if (!reached[i]) {
				ctx.fillStyle = 'rgb(28,28,32)';
			} else {
				const l = level[i] * 255;
				ctx.fillStyle = `rgb(${r[i] * l},${g[i] * l},${b[i] * l})`;
			}
			ctx.fillRect(x, 0, Math.max(cell, 1), height);
		}
	}

	$effect(() => {
		if (!visible || !ready) return;
		let frame = 0;
		const tick = () => {
			if (!document.hidden) draw(Date.now());
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	// ── The spread handle ─────────────────────────────────────────────────────

	let dragging = $state(false);
	const handleAt = $derived(Math.min(1, (layer.spread_cycles ?? 1) / 4));
	const spreadMatters = $derived(layer.spread !== 'Even' && count > 1);

	function at(event: PointerEvent): number {
		const box = row?.getBoundingClientRect();
		if (!box) return handleAt;
		return (event.clientX - box.left) / Math.max(box.width, 1);
	}

	function down(event: PointerEvent) {
		(event.currentTarget as Element).setPointerCapture(event.pointerId);
		dragging = true;
		oncycles?.(draggedCycles(at(event)));
	}
	function move(event: PointerEvent) {
		if (dragging) oncycles?.(draggedCycles(at(event)));
	}
	function up() {
		dragging = false;
	}
</script>

<div class="preview" bind:this={row}>
	{#if spreadMatters && oncycles}
		<div
			class="spread"
			role="slider"
			tabindex="0"
			aria-label="How many cycles the spread covers"
			aria-valuemin="0"
			aria-valuemax="4"
			aria-valuenow={layer.spread_cycles}
			use:asOneGesture
			onpointerdown={down}
			onpointermove={move}
			onpointerup={up}
			onpointercancel={up}
			onkeydown={(e) => {
				const by = e.key === 'ArrowRight' ? 0.25 : e.key === 'ArrowLeft' ? -0.25 : 0;
				if (by) {
					e.preventDefault();
					oncycles?.(Math.min(4, Math.max(0, (layer.spread_cycles ?? 1) + by)));
				}
			}}
		>
			<span class="rail" style:width="{handleAt * 100}%"></span>
			<span class="handle" class:dragging style:left="{handleAt * 100}%">
				{layer.spread_cycles}×
			</span>
		</div>
	{/if}
	<canvas bind:this={canvas} class:strip aria-label="What the effect does across the selection"></canvas>
	{#if !ready}
		<span class="waiting">Loading the evaluator…</span>
	{/if}
</div>

<style>
	.preview {
		position: relative;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	canvas {
		width: 100%;
		height: 34px;
		border-radius: var(--radius);
		background: #000;
		display: block;
	}
	canvas.strip {
		height: 18px;
	}
	.waiting {
		position: absolute;
		inset: auto 0 8px 0;
		text-align: center;
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.spread {
		position: relative;
		height: 22px;
		cursor: ew-resize;
		touch-action: none;
		border-radius: var(--radius);
		background: var(--bg-sunken);
		border: 1px solid var(--line);
	}
	.spread:focus-visible {
		outline: 1px solid var(--accent);
	}
	.rail {
		position: absolute;
		inset: 0 auto 0 0;
		background: var(--accent);
		opacity: 0.25;
		pointer-events: none;
	}
	.handle {
		position: absolute;
		top: 1px;
		transform: translateX(-50%);
		padding: 0 6px;
		font-size: var(--font-xs);
		line-height: 18px;
		border-radius: var(--radius);
		background: var(--accent);
		color: var(--bg);
		pointer-events: none;
		white-space: nowrap;
	}
	.handle.dragging {
		box-shadow: 0 0 0 2px var(--accent);
	}
</style>
