<script lang="ts">
	/**
	 * The line between two tiles, and the handle for moving it.
	 *
	 * The drag is sent as a change per move rather than as a position, because the
	 * tree is the only thing that knows how big the tiles either side are allowed to
	 * get: the gutter says "a bit more this way" and the layout decides how much of
	 * that it can actually give.
	 */

	let {
		axis,
		extent,
		onmove
	}: {
		axis: 'x' | 'y';
		/** How many pixels the whole split is, so a drag can be turned into a share. */
		extent: number;
		onmove: (delta: number) => void;
	} = $props();

	let bar = $state<HTMLDivElement | null>(null);
	let last = $state<number | null>(null);

	const along = (event: PointerEvent) => (axis === 'x' ? event.clientX : event.clientY);

	function down(event: PointerEvent) {
		bar?.setPointerCapture(event.pointerId);
		last = along(event);
	}

	function move(event: PointerEvent) {
		if (last === null) return;
		const now = along(event);
		onmove((now - last) / Math.max(extent, 1));
		last = now;
	}

	function up(event: PointerEvent) {
		last = null;
		bar?.releasePointerCapture?.(event.pointerId);
	}

	function key(event: KeyboardEvent) {
		const step = 12 / Math.max(extent, 1);
		const wanted = axis === 'x' ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown'];
		const at = wanted.indexOf(event.key);
		if (at < 0) return;
		event.preventDefault();
		onmove(at === 0 ? -step : step);
	}
</script>

<!-- A focusable `separator` is the ARIA window-splitter pattern exactly: the thing
     between two panes, which can be moved with the arrow keys. Svelte's rule reads
     `separator` as decoration, which is what it is everywhere else. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	class="gutter {axis}"
	class:active={last !== null}
	bind:this={bar}
	role="separator"
	tabindex="0"
	aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
	aria-label="Resize"
	onpointerdown={down}
	onpointermove={move}
	onpointerup={up}
	onpointercancel={up}
	onkeydown={key}
></div>

<style>
	.gutter {
		flex: none;
		background: var(--line);
		touch-action: none;
		position: relative;
	}
	.gutter.x {
		width: 1px;
		cursor: col-resize;
	}
	.gutter.y {
		height: 1px;
		cursor: row-resize;
	}
	/* A one-pixel line is the right thing to look at and the wrong thing to grab, so
	   the hit area reaches past it in both directions. */
	.gutter::after {
		content: '';
		position: absolute;
		inset: 0;
	}
	.gutter.x::after {
		left: -3px;
		right: -3px;
	}
	.gutter.y::after {
		top: -3px;
		bottom: -3px;
	}
	.gutter:hover,
	.gutter:focus-visible,
	.gutter.active {
		background: var(--accent);
		outline: none;
	}
</style>
