<script lang="ts">
	/**
	 * A colour, three ways at once.
	 *
	 * The square is for finding a colour, the faders for adjusting one, and the hex
	 * field for entering the one somebody wrote down. They all edit the same value,
	 * so which is the right one depends only on what the operator already knows.
	 *
	 * Hue and saturation only. How bright a lantern is belongs to Intensity, and a
	 * colour picker that dimmed the light as well would leave two controls fighting
	 * over the same thing.
	 */

	import { hexToRgb, rgbToHex } from '$lib/programmer.js';
	import Fader from './Fader.svelte';

	type Rgb = { r: number; g: number; b: number };

	let { value, oninput }: { value: Rgb; oninput: (value: Rgb) => void } = $props();

	let square = $state<HTMLDivElement | null>(null);
	let dragging = $state(false);
	let hexDraft = $state<string | null>(null);

	const hsv = $derived(toHsv(value));
	const hex = $derived(rgbToHex(value));

	function pick(event: PointerEvent) {
		if (!square) return;
		const box = square.getBoundingClientRect();
		const h = clamp((event.clientX - box.left) / Math.max(box.width, 1)) * 360;
		const s = 1 - clamp((event.clientY - box.top) / Math.max(box.height, 1));
		oninput(fromHsv(h, s, 1));
	}

	function commitHex() {
		const parsed = hexDraft === null ? null : hexToRgb(hexDraft);
		if (parsed) oninput(parsed);
		hexDraft = null;
	}

	const clamp = (v: number) => Math.min(1, Math.max(0, v));

	function toHsv({ r, g, b }: Rgb): { h: number; s: number } {
		const max = Math.max(r, g, b);
		const min = Math.min(r, g, b);
		const span = max - min;
		let h = 0;
		if (span > 0) {
			if (max === r) h = ((g - b) / span) % 6;
			else if (max === g) h = (b - r) / span + 2;
			else h = (r - g) / span + 4;
			h *= 60;
			if (h < 0) h += 360;
		}
		return { h, s: max === 0 ? 0 : span / max };
	}

	function fromHsv(h: number, s: number, v: number): Rgb {
		const c = v * s;
		const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
		const m = v - c;
		const [r, g, b] =
			h < 60
				? [c, x, 0]
				: h < 120
					? [x, c, 0]
					: h < 180
						? [0, c, x]
						: h < 240
							? [0, x, c]
							: h < 300
								? [x, 0, c]
								: [c, 0, x];
		return { r: r + m, g: g + m, b: b + m };
	}
</script>

<div class="colour">
	<div
		class="square"
		bind:this={square}
		role="application"
		aria-label="Hue and saturation"
		onpointerdown={(e) => {
			square?.setPointerCapture(e.pointerId);
			dragging = true;
			pick(e);
		}}
		onpointermove={(e) => dragging && pick(e)}
		onpointerup={(e) => {
			dragging = false;
			square?.releasePointerCapture?.(e.pointerId);
		}}
		onpointercancel={() => (dragging = false)}
	>
		<div
			class="dot"
			style:left="{(hsv.h / 360) * 100}%"
			style:top="{(1 - hsv.s) * 100}%"
			style:background={hex}
		></div>
	</div>

	<div class="channels">
		<label><span>R</span><Fader label="Red" value={value.r} tint="#ef4444" oninput={(r) => oninput({ ...value, r })} /></label>
		<label><span>G</span><Fader label="Green" value={value.g} tint="#22c55e" oninput={(g) => oninput({ ...value, g })} /></label>
		<label><span>B</span><Fader label="Blue" value={value.b} tint="#4a9eff" oninput={(b) => oninput({ ...value, b })} /></label>
		<label class="hex">
			<span>#</span>
			<input
				value={hexDraft ?? hex.slice(1)}
				spellcheck="false"
				oninput={(e) => (hexDraft = e.currentTarget.value)}
				onblur={commitHex}
				onkeydown={(e) => {
					if (e.key === 'Enter') commitHex();
					if (e.key === 'Escape') hexDraft = null;
				}}
			/>
			<span class="swatch" style:background={hex}></span>
		</label>
	</div>
</div>

<style>
	.colour {
		display: flex;
		gap: 10px;
		align-items: flex-start;
	}

	.square {
		position: relative;
		width: 108px;
		height: 78px;
		flex: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		cursor: crosshair;
		touch-action: none;
		background:
			linear-gradient(to bottom, transparent, #fff),
			linear-gradient(
				to right,
				#f00 0%,
				#ff0 16.6%,
				#0f0 33.3%,
				#0ff 50%,
				#00f 66.6%,
				#f0f 83.3%,
				#f00 100%
			);
	}

	.dot {
		position: absolute;
		width: 9px;
		height: 9px;
		margin: -5px 0 0 -5px;
		border: 2px solid #fff;
		border-radius: 50%;
		box-shadow: 0 0 0 1px #0008;
		pointer-events: none;
	}

	.channels {
		display: flex;
		flex-direction: column;
		gap: 4px;
		flex: 1;
		min-width: 0;
	}

	label {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	label > span:first-child {
		width: 10px;
		color: var(--text-dim);
		font-size: var(--font-xs);
		font-family: monospace;
	}

	.hex input {
		flex: 1;
		min-width: 0;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-family: monospace;
		font-size: var(--font-xs);
		padding: 3px 6px;
	}
	.hex input:focus {
		outline: none;
		border-color: var(--accent);
	}

	.swatch {
		width: 18px;
		height: 18px;
		flex: none;
		border: 1px solid var(--line-strong);
		border-radius: 3px;
	}
</style>
