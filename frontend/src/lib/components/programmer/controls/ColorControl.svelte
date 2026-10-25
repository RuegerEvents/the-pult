<script lang="ts">
	/**
	 * A color: a swatch and its hex on one line, and the picker behind the swatch.
	 *
	 * One line, the height of a fader, because it sits in rows of them — the square,
	 * three faders and a hex field stacked in place made every row with a color five
	 * times the height of its neighbours. The picker opens over everything: the
	 * square for finding a color, the faders for adjusting one, and the hex field for
	 * the one somebody wrote down, all editing the same value.
	 *
	 * Hue and saturation only. How bright a lantern is belongs to Intensity, and a
	 * color picker that dimmed the light as well would leave two controls fighting
	 * over the same thing.
	 */

	import { hexToRgb, rgbToHex } from '$lib/programmer.js';
	import Fader from './Fader.svelte';

	type Rgb = { r: number; g: number; b: number };

	import { asOneGesture } from '$lib/stores/gesture.js';

	let { value, oninput }: { value: Rgb; oninput: (value: Rgb) => void } = $props();

	let square = $state<HTMLDivElement | null>(null);
	let swatchButton = $state<HTMLButtonElement | null>(null);
	let popover = $state<HTMLDivElement | null>(null);
	/** Where the picker opens: fixed to the page, so no panel's overflow can clip it. */
	let open = $state<{ left: number; top: number } | null>(null);

	const PICKER_W = 260;
	const PICKER_H = 230;
	function toggle() {
		if (open || !swatchButton) return (open = null);
		const b = swatchButton.getBoundingClientRect();
		const below = b.bottom + 6 + PICKER_H <= window.innerHeight;
		open = {
			left: Math.max(8, Math.min(b.left, window.innerWidth - PICKER_W - 8)),
			top: below ? b.bottom + 6 : Math.max(8, b.top - 6 - PICKER_H)
		};
	}
	function away(event: PointerEvent) {
		if (!open) return;
		const target = event.target as Node;
		if (popover?.contains(target) || swatchButton?.contains(target)) return;
		open = null;
	}
	function escape(event: KeyboardEvent) {
		if (open && event.key === 'Escape') {
			event.stopPropagation();
			open = null;
		}
	}
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

<svelte:window onpointerdown={away} onkeydown={escape} />

<div class="color">
	<button
		class="swatch-button"
		bind:this={swatchButton}
		style:background={hex}
		aria-label="Pick a color"
		aria-expanded={!!open}
		onclick={toggle}
	></button>
	<input
		class="hex mono"
		value={hexDraft ?? hex.slice(1).toUpperCase()}
		spellcheck="false"
		aria-label="Hex color"
		oninput={(e) => (hexDraft = e.currentTarget.value)}
		onblur={commitHex}
		onkeydown={(e) => {
			if (e.key === 'Enter') commitHex();
			if (e.key === 'Escape') hexDraft = null;
		}}
	/>
</div>

{#if open}
	<div class="picker" bind:this={popover} style:left="{open.left}px" style:top="{open.top}px">
		<div
			class="square"
			bind:this={square}
			use:asOneGesture
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
		</div>
	</div>
{/if}

<style>
	.color {
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 28px;
	}
	.swatch-button {
		flex: none;
		width: 44px;
		height: 26px;
		border: 1px solid #ffffff30;
		border-radius: var(--radius);
		cursor: pointer;
	}
	.swatch-button:hover,
	.swatch-button[aria-expanded='true'] {
		border-color: var(--text-dim);
	}
	.hex {
		width: 6.5em;
		min-height: 26px;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-family: monospace;
		font-size: var(--font-xs);
		padding: 0 6px;
	}
	.hex:focus {
		outline: none;
		border-color: var(--accent);
	}
	.mono {
		font-family: monospace;
	}

	.picker {
		position: fixed;
		z-index: 60;
		width: 260px;
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 10px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 10px 30px rgb(0 0 0 / 0.5);
	}

	.square {
		position: relative;
		height: 120px;
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
</style>
