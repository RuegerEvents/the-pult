<script lang="ts">
	/**
	 * Pan and tilt on one pad.
	 *
	 * Left to right is pan, bottom to top is tilt — up on the pad is up in the room,
	 * which is the opposite of how a screen counts and the only way round that reads
	 * as pointing. A head that has only one of the two axes gets a pad that only
	 * moves in that direction.
	 */

	let {
		pan,
		tilt,
		onpan,
		ontilt
	}: {
		pan: number | null;
		tilt: number | null;
		onpan?: (value: number) => void;
		ontilt?: (value: number) => void;
	} = $props();

	let pad = $state<HTMLButtonElement | null>(null);
	let dragging = $state(false);

	const clamp = (v: number) => Math.min(1, Math.max(0, v));

	function aim(event: PointerEvent) {
		if (!pad) return;
		const box = pad.getBoundingClientRect();
		if (onpan) onpan(clamp((event.clientX - box.left) / Math.max(box.width, 1)));
		if (ontilt) ontilt(1 - clamp((event.clientY - box.top) / Math.max(box.height, 1)));
	}

	function key(event: KeyboardEvent) {
		const step = event.shiftKey ? 0.002 : 0.02;
		const move = (dp: number, dt: number) => {
			event.preventDefault();
			if (dp && onpan && pan !== null) onpan(clamp(pan + dp));
			if (dt && ontilt && tilt !== null) ontilt(clamp(tilt + dt));
		};
		if (event.key === 'ArrowLeft') move(-step, 0);
		else if (event.key === 'ArrowRight') move(step, 0);
		else if (event.key === 'ArrowUp') move(0, step);
		else if (event.key === 'ArrowDown') move(0, -step);
	}
</script>

<!-- A button rather than a div with a tabindex: it is a control, it takes focus,
     and the browser already knows both of those about a button. -->
<button
	type="button"
	class="pad"
	bind:this={pad}
	aria-label="Pan and tilt"
	onpointerdown={(e) => {
		pad?.setPointerCapture(e.pointerId);
		dragging = true;
		aim(e);
	}}
	onpointermove={(e) => dragging && aim(e)}
	onpointerup={(e) => {
		dragging = false;
		pad?.releasePointerCapture?.(e.pointerId);
	}}
	onpointercancel={() => (dragging = false)}
	onkeydown={key}
>
	<span class="cross v"></span>
	<span class="cross h"></span>
	<span
		class="handle"
		style:left="{(pan ?? 0.5) * 100}%"
		style:top="{(1 - (tilt ?? 0.5)) * 100}%"
	></span>
</button>

<style>
	.pad {
		display: block;
		position: relative;
		padding: 0;
		width: 100%;
		aspect-ratio: 4 / 3;
		max-height: 130px;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		background: var(--bg-sunken);
		cursor: crosshair;
		touch-action: none;
	}
	.pad:focus-visible {
		outline: 1px solid var(--accent);
		outline-offset: 1px;
	}

	/* Centre lines: where the head hangs, so "back to rest" is somewhere to aim for. */
	.cross {
		position: absolute;
		background: var(--line-strong);
		pointer-events: none;
	}
	.cross.v {
		left: 50%;
		top: 0;
		bottom: 0;
		width: 1px;
	}
	.cross.h {
		top: 50%;
		left: 0;
		right: 0;
		height: 1px;
	}

	.handle {
		position: absolute;
		width: 11px;
		height: 11px;
		margin: -6px 0 0 -6px;
		border: 2px solid var(--live);
		border-radius: 50%;
		pointer-events: none;
	}
</style>
