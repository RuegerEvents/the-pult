<script lang="ts">
	/**
	 * A 0–1 fader.
	 *
	 * Dragging jumps to the pointer and follows it. Holding shift changes to a fine
	 * drag — the value moves a fifth as far as the pointer does, measured from
	 * wherever the key went down, so switching mid-drag does not make the value jump.
	 * Arrow keys step, shift steps finer, Page steps coarser, Home and End go to the
	 * ends.
	 */

	let {
		value,
		oninput,
		tint = 'var(--accent)',
		label = 'Level'
	}: {
		value: number;
		oninput: (value: number) => void;
		tint?: string;
		label?: string;
	} = $props();

	let track = $state<HTMLDivElement | null>(null);
	/// Where a fine drag is measured from. Reset whenever shift goes down or up, so
	/// the value carries on from where it is rather than snapping.
	let anchor: { x: number; value: number; fine: boolean } | null = $state(null);

	const clamp = (v: number) => Math.min(1, Math.max(0, v));

	function fractionAt(clientX: number): number {
		if (!track) return value;
		const box = track.getBoundingClientRect();
		return clamp((clientX - box.left) / Math.max(box.width, 1));
	}

	function down(event: PointerEvent) {
		track?.setPointerCapture(event.pointerId);
		anchor = { x: event.clientX, value, fine: event.shiftKey };
		if (!event.shiftKey) oninput(fractionAt(event.clientX));
	}

	function move(event: PointerEvent) {
		if (!anchor) return;
		if (event.shiftKey !== anchor.fine) {
			anchor = { x: event.clientX, value, fine: event.shiftKey };
			return;
		}
		if (!anchor.fine) {
			oninput(fractionAt(event.clientX));
			return;
		}
		const width = Math.max(track?.getBoundingClientRect().width ?? 1, 1);
		oninput(clamp(anchor.value + ((event.clientX - anchor.x) / width) * 0.2));
	}

	function up(event: PointerEvent) {
		anchor = null;
		track?.releasePointerCapture?.(event.pointerId);
	}

	function key(event: KeyboardEvent) {
		const step = event.shiftKey ? 0.001 : 0.01;
		const by = (delta: number) => {
			event.preventDefault();
			oninput(clamp(value + delta));
		};
		switch (event.key) {
			case 'ArrowRight':
			case 'ArrowUp':
				return by(step);
			case 'ArrowLeft':
			case 'ArrowDown':
				return by(-step);
			case 'PageUp':
				return by(0.1);
			case 'PageDown':
				return by(-0.1);
			case 'Home':
				event.preventDefault();
				return oninput(0);
			case 'End':
				event.preventDefault();
				return oninput(1);
		}
	}
</script>

<div
	class="fader"
	bind:this={track}
	role="slider"
	tabindex="0"
	aria-label={label}
	aria-valuemin="0"
	aria-valuemax="1"
	aria-valuenow={value}
	aria-valuetext="{Math.round(value * 100)}%"
	onpointerdown={down}
	onpointermove={move}
	onpointerup={up}
	onpointercancel={up}
	onkeydown={key}
>
	<div class="fill" style:width="{clamp(value) * 100}%" style:background={tint}></div>
	<span class="readout">{Math.round(clamp(value) * 100)}</span>
</div>

<style>
	.fader {
		position: relative;
		/* A fader is dragged, so unlike a button it has to *be* the size of the
		   gesture rather than reaching it with padding. `--fader-h` is the one
		   figure the touch conversion turns on. */
		height: var(--fader-h);
		min-width: 110px;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		background: var(--bg-sunken);
		cursor: ew-resize;
		touch-action: none;
		overflow: hidden;
	}
	.fader:focus-visible {
		outline: 1px solid var(--accent);
		outline-offset: 1px;
	}

	.fill {
		position: absolute;
		inset: 0 auto 0 0;
		opacity: 0.45;
		pointer-events: none;
	}

	.readout {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: flex-end;
		padding-right: 8px;
		font-family: monospace;
		font-size: var(--font-sm);
		color: var(--text);
		pointer-events: none;
	}
</style>
