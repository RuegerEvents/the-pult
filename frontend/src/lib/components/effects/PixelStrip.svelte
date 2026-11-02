<script lang="ts">
	/**
	 * A node effect, running: a short strip of pixels drawn by the console's copy of
	 * the node's renderer (`pixel_effect` in the wasm), so picking an effect is picking
	 * what it looks like rather than guessing from a word.
	 *
	 * Drawn every animation frame while it is on screen, which a backgrounded tab does
	 * not get — and that is right, nobody is looking.
	 */

	import { pixelEffect } from '$lib/evaluator.js';
	import type { NodeEffect } from '$lib/stage.js';
	import { consoleNow } from '$lib/ws/clock.js';

	let { effect: fx, count = 24 }: { effect: NodeEffect; count?: number } = $props();

	let canvas = $state<HTMLCanvasElement | null>(null);

	$effect(() => {
		const c = canvas;
		if (!c) return;
		const ctx = c.getContext('2d');
		if (!ctx) return;
		let frame = 0;
		const draw = () => {
			// A preview of an effect at brightness zero would be a black bar; it is
			// shown lit, since what is being chosen is the movement and the colors.
			const lit = { ...fx, brightness: fx.brightness > 0 ? fx.brightness : 1 };
			const bytes = pixelEffect(lit, consoleNow() ?? Date.now(), count);
			const w = c.width / count;
			ctx.clearRect(0, 0, c.width, c.height);
			if (bytes) {
				for (let i = 0; i < count; i++) {
					ctx.fillStyle = `rgb(${bytes[i * 3]}, ${bytes[i * 3 + 1]}, ${bytes[i * 3 + 2]})`;
					ctx.fillRect(i * w + 0.5, 0, Math.max(1, w - 1), c.height);
				}
			}
			frame = requestAnimationFrame(draw);
		};
		frame = requestAnimationFrame(draw);
		return () => cancelAnimationFrame(frame);
	});
</script>

<canvas bind:this={canvas} width={count * 6} height="8" aria-hidden="true"></canvas>

<style>
	canvas {
		display: block;
		width: 100%;
		height: 8px;
		border-radius: 2px;
		background: #000;
		image-rendering: pixelated;
	}
</style>
