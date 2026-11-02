<script lang="ts">
	/**
	 * An upright playback fader that only moves when its cap is dragged.
	 *
	 * A native range jumps to wherever it is touched, so a thumb scrolling a page of
	 * executors past it set a sequence's level on the way. Here the track takes no
	 * input at all — a touch on it scrolls the panel — and the cap moves *by* how far
	 * the pointer went rather than *to* it, the way a motor fader under a finger does,
	 * so grabbing it never changes the level by itself. Arrow keys step, shift steps
	 * finer, Page steps coarser, Home and End go to the ends.
	 */

	import { asOneGesture, nudging } from '$lib/stores/gesture.js';

	let {
		value,
		oninput,
		label = 'Level'
	}: {
		value: number;
		oninput: (value: number) => void;
		label?: string;
	} = $props();

	let track = $state<HTMLDivElement | null>(null);
	/// Where the drag started: the pointer's height and the level under it.
	let anchor: { y: number; value: number } | null = $state(null);

	const clamp = (v: number) => Math.min(1, Math.max(0, v));
	const level = $derived(clamp(value));

	/** The cap's height in pixels, which the CSS below says too. */
	const CAP = 26;

	function down(event: PointerEvent) {
		if (event.button !== 0) return;
		event.preventDefault();
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		anchor = { y: event.clientY, value: level };
	}

	function move(event: PointerEvent) {
		if (!anchor || !track) return;
		// The cap travels the track less its own height, so a full drag is a full throw.
		const travel = Math.max(track.clientHeight - CAP, 1);
		const fine = event.shiftKey ? 0.2 : 1;
		const next = clamp(anchor.value + ((anchor.y - event.clientY) / travel) * fine);
		if (next !== level) oninput(next);
	}

	function up(event: PointerEvent) {
		anchor = null;
		(event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
	}

	function key(event: KeyboardEvent) {
		const step = event.shiftKey ? 0.001 : 0.01;
		const to = (v: number) => {
			event.preventDefault();
			// A held key is a drag with no pointer to say when it stopped.
			nudging();
			oninput(clamp(v));
		};
		switch (event.key) {
			case 'ArrowUp':
			case 'ArrowRight':
				return to(level + step);
			case 'ArrowDown':
			case 'ArrowLeft':
				return to(level - step);
			case 'PageUp':
				return to(level + 0.1);
			case 'PageDown':
				return to(level - 0.1);
			case 'Home':
				return to(0);
			case 'End':
				return to(1);
		}
	}

</script>

<div class="track" bind:this={track} style:--level={level}>
	<div class="fill"></div>
	<div
		class="cap"
		class:held={anchor !== null}
		use:asOneGesture
		role="slider"
		tabindex="0"
		aria-label={label}
		aria-orientation="vertical"
		aria-valuemin="0"
		aria-valuemax="1"
		aria-valuenow={level}
		aria-valuetext="{Math.round(level * 100)}%"
		onpointerdown={down}
		onpointermove={move}
		onpointerup={up}
		onpointercancel={up}
		onkeydown={key}
	></div>
</div>

<style>
	.track {
		position: relative;
		flex: 1;
		min-height: 0;
		width: 40px;
	}
	/* The slot, and how far up it the level is. */
	.track::before,
	.fill {
		content: '';
		position: absolute;
		left: 50%;
		width: 6px;
		transform: translateX(-50%);
		border-radius: 3px;
	}
	.track::before {
		top: 13px;
		bottom: 13px;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
	}
	.fill {
		bottom: 13px;
		height: calc((100% - 26px) * var(--level));
		background: var(--live);
		opacity: 0.7;
	}

	.cap {
		position: absolute;
		left: 0;
		right: 0;
		height: 26px;
		bottom: calc((100% - 26px) * var(--level));
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		background: linear-gradient(var(--bg-raised), var(--bg-chrome));
		box-shadow: inset 0 -1px 0 var(--line-strong);
		cursor: grab;
		/* The cap is the only thing that takes a drag, so it is the only thing that
		   keeps a touch from scrolling. */
		touch-action: none;
	}
	.cap::after {
		content: '';
		position: absolute;
		left: 6px;
		right: 6px;
		top: 50%;
		height: 2px;
		margin-top: -1px;
		background: var(--live);
	}
	.cap.held {
		cursor: grabbing;
		border-color: var(--live);
	}
	.cap:focus-visible {
		outline: 1px solid var(--accent);
		outline-offset: 1px;
	}
</style>
