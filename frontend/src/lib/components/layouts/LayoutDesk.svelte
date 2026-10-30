<script lang="ts">
	/**
	 * A layout on the desk: drawn to fit its tile, with every element something to take
	 * hold of.
	 *
	 * The object grammar, as a card has it (`$lib/objects.ts`): a **tap acts** — an
	 * element with a press presses it, one without opens it in the Inspector, since a
	 * wall has nothing else to do; **⌘/Ctrl-click** or the corner dot makes it the
	 * Inspector's subject; **right-click** or a long-press opens the one context menu.
	 * With the Show lock open, a **drag** moves an element and the grips round the
	 * subject resize it, on the layout's grid; a press on the empty canvas makes the
	 * layout itself the subject, which is where its size and decay are edited.
	 *
	 * # A drag is one act, and it writes per frame
	 *
	 * `BlockHandles`' rule for a paperwork viewport, which is `stores/editor.ts`'s rule
	 * for a truss: a gesture opens when the pointer has moved far enough to be a drag
	 * rather than a tap, the element list is written at most once per animation frame,
	 * and the gesture closes when the pointer comes up. Meanwhile the dragged element is
	 * drawn from the pointer rather than from the round trip, so it does not lag behind
	 * the hand on a slow link.
	 *
	 * The handles are an overlay of plain boxes over the drawing rather than anything in
	 * the SVG, for the reason the paperwork's are: the drawing is a rendering of the
	 * model, and the thing being dragged has to be the element.
	 *
	 * # Like the flow editor, which is like a drawing program
	 *
	 * Several elements are a selection: a lasso on a one-finger or left-button drag over
	 * the empty canvas, ⌘/Ctrl/Shift-click, ⌘A. Dragging one of them moves them all, as
	 * one (`movedGroup`). The view moves with two fingers on a trackpad or Option, Space
	 * or the middle button with a drag, and zooms with a pinch; *Fit* puts it back.
	 * ⌘C, ⌘X and ⌘V copy elements through `localStorage`, across layouts and tabs, the
	 * paste landing new, selected and in the middle of the view; ⌘D duplicates in place,
	 * a cell off. The view is this tile's and not the show's.
	 */

	import type { Layout, LayoutElement } from '$lib/generated/index.js';
	import { longPress } from '$lib/actions.js';
	import {
		boundsOf,
		boxOf,
		clampView,
		copyElements,
		draggedRect,
		FIXTURE_MIME,
		fitView,
		inLasso,
		LAYOUT_CLIP_KEY,
		movedGroup,
		pastedElements,
		placed,
		pressOf,
		rectBetween,
		removeElements,
		sensorElements,
		toCanvas,
		zoomAt,
		type Grip,
		type LayoutClip,
		type LayoutShow,
		type Rect,
		type View
	} from '$lib/layoutView.js';
	import { openMenu, selectSubject, subject, clearSubject, type ObjectRef } from '$lib/objects.js';
	import { beginGesture, endGesture, nudging } from '$lib/stores/gesture.js';
	import { pressElement, writeElements } from '$lib/stores/layouts.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { addToast } from '$lib/toasts.js';
	import LayoutDrawing from './LayoutDrawing.svelte';

	let {
		layout,
		show,
		live = true
	}: {
		layout: Layout;
		show: LayoutShow;
		/** False while the console is not reachable: drawn, dimmed, and nothing presses. */
		live?: boolean;
	} = $props();

	let host = $state<HTMLDivElement | null>(null);
	let screen = $state({ width: 0, height: 0 });
	$effect(() => {
		if (!host) return;
		const observer = new ResizeObserver(([entry]) => {
			screen = { width: entry.contentRect.width, height: entry.contentRect.height };
		});
		observer.observe(host);
		return () => observer.disconnect();
	});

	const canvas = $derived({ width: layout.width, height: layout.height });
	/** Where somebody moved or zoomed the view to, or `null` for fitted. */
	let moved_ = $state<View | null>(null);
	const fitted = $derived(fitView(canvas, screen, 12));
	const view = $derived(moved_ ? clampView(moved_, canvas, screen) : fitted);
	// Another layout in the tile is a fresh look at it.
	$effect(() => {
		void layout.id;
		moved_ = null;
	});

	/** The elements of this layout the Inspector has hold of. */
	const chosen = $derived(
		new Set($subject.filter((r) => r.kind === 'layout-element').map((r) => r.id).filter((id) => layout.elements.some((e) => e.id === id)))
	);
	const one = $derived(chosen.size === 1 ? (layout.elements.find((e) => chosen.has(e.id)) ?? null) : null);

	const refOf = (element: LayoutElement): ObjectRef => ({ kind: 'layout-element', id: element.id });

	/** An element's box on the screen, at least a finger wide: a wall is a line with no height. */
	function onScreen(rect: Rect) {
		const least = 14;
		const w = rect.w * view.scale;
		const h = rect.h * view.scale;
		return {
			left: view.x + rect.x * view.scale - Math.max(0, least - w) / 2,
			top: view.y + rect.y * view.scale - Math.max(0, least - h) / 2,
			width: Math.max(w, least),
			height: Math.max(h, least)
		};
	}

	// ── Dragging ──────────────────────────────────────────────────────────────

	/** How far a pointer moves before a press is a drag rather than a tap. */
	const SLOP_PX = 4;

	let drag: {
		id: string;
		grip: Grip;
		startX: number;
		startY: number;
		/** Every element the drag carries, where it was: the selection, when this is in it. */
		from: Map<string, Rect>;
		dragging: boolean;
		frame: number | null;
		pending: LayoutElement[] | null;
	} | null = null;
	/** Whether the press that is ending was a drag, so its click is not also a tap. */
	let dragged = false;
	/** The dragged elements as the pointer has them, drawn ahead of the round trip. */
	let overrides = $state.raw<Map<string, LayoutElement> | null>(null);

	function begin(event: PointerEvent, element: LayoutElement, grip: Grip) {
		if (pansFrom(event)) {
			pan(event);
			return;
		}
		if (event.button !== 0 || !$canEdit) return;
		event.stopPropagation();
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		dragged = false;
		const carried = grip === 'move' && chosen.has(element.id) ? [...chosen] : [element.id];
		drag = {
			id: element.id,
			grip,
			startX: event.clientX,
			startY: event.clientY,
			from: new Map(
				carried.flatMap((id) => {
					const e = layout.elements.find((x) => x.id === id);
					return e ? [[id, boxOf(e)] as [string, Rect]] : [];
				})
			),
			// A grip has nothing to tap: it is a drag from the first pixel.
			dragging: false,
			frame: null,
			pending: null
		};
		if (grip !== 'move') start();
	}

	function start() {
		if (!drag || drag.dragging) return;
		drag.dragging = true;
		dragged = true;
		beginGesture();
	}

	function moved(event: PointerEvent) {
		if (!drag) return;
		const dx = event.clientX - drag.startX;
		const dy = event.clientY - drag.startY;
		if (!drag.dragging) {
			if (Math.hypot(dx, dy) < SLOP_PX) return;
			start();
		}
		// In canvas units from the start, so the arithmetic is the model's and a tile
		// resized mid-drag cannot make the element jump.
		const ux = dx / view.scale;
		const uy = dy / view.scale;
		if (drag.grip === 'move') {
			drag.pending = movedGroup(layout.elements, drag.from, ux, uy, canvas, layout.grid);
		} else {
			const id = drag.id;
			const rect = draggedRect(drag.from.get(id)!, drag.grip, ux, uy, canvas, layout.grid);
			drag.pending = layout.elements.map((e) => (e.id === id ? placed(e, rect) : e));
		}
		const carried = drag.from;
		overrides = new Map(drag.pending.filter((e) => carried.has(e.id)).map((e) => [e.id, e]));
		if (drag.frame === null) drag.frame = requestAnimationFrame(commit);
	}

	function commit(): Promise<void> | undefined {
		if (!drag) return;
		drag.frame = null;
		const next = drag.pending;
		if (!next) return;
		// `gesture: false` — the gesture is the drag, opened when it started.
		return writeElements(layout.id, next, false);
	}

	async function end(event: PointerEvent) {
		if (!drag) return;
		(event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
		const was = drag;
		if (was.frame !== null) cancelAnimationFrame(was.frame);
		if (!was.dragging) {
			drag = null;
			return;
		}
		const last = commit();
		drag = null;
		endGesture();
		// A selection dragged stays the selection; one element dragged becomes it.
		if (was.from.size === 1) selectSubject({ kind: 'layout-element', id: was.id });
		await last;
		overrides = null;
	}

	// ── Tapping ───────────────────────────────────────────────────────────────

	async function tap(event: MouseEvent, element: LayoutElement) {
		event.stopPropagation();
		if (dragged) {
			dragged = false;
			return;
		}
		if (event.metaKey || event.ctrlKey || event.shiftKey) {
			selectSubject(refOf(element), true);
			return;
		}
		if (pressOf(element)) {
			if (live) await pressElement(element);
			return;
		}
		selectSubject(refOf(element));
	}

	/** The empty canvas: the layout itself, for its size, grid and decay. */
	function background(event: MouseEvent) {
		if (event.target instanceof Element && event.target.closest('.handle, .fit')) return;
		// The click that ends a lasso or a pan is not also a press on the canvas.
		if (swept) {
			swept = false;
			return;
		}
		if ($canEdit || event.metaKey || event.ctrlKey) selectSubject({ kind: 'layout', id: layout.id });
		else clearSubject();
	}

	// ── Keys ──────────────────────────────────────────────────────────────────

	function key(event: KeyboardEvent) {
		if (event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]')) return;
		if ((event.metaKey || event.ctrlKey) && !event.altKey) {
			const k = event.key.toLowerCase();
			if (k === 'a') {
				event.preventDefault();
				subject.set(layout.elements.map(refOf));
			} else if (k === 'c') {
				if (copySelected()) event.preventDefault();
			} else if (k === 'x' && $canEdit && chosen.size > 0) {
				event.preventDefault();
				if (copySelected()) void removeChosen();
			} else if (k === 'v' && $canEdit) {
				event.preventDefault();
				void paste();
			} else if (k === 'd' && $canEdit && chosen.size > 0) {
				event.preventDefault();
				void duplicate();
			}
			return;
		}
		if (event.key === 'Escape' && chosen.size > 0) {
			event.preventDefault();
			clearSubject();
			return;
		}
		if (!$canEdit || chosen.size === 0) return;
		if (event.key === 'Delete' || event.key === 'Backspace') {
			event.preventDefault();
			void removeChosen();
			return;
		}
		const step = (layout.grid > 0 ? layout.grid : 1) * (event.shiftKey ? 5 : 1);
		const by = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[event.key];
		if (!by) return;
		event.preventDefault();
		// A held arrow is a drag by another name: one gesture until it is let go.
		nudging();
		const from = new Map(layout.elements.filter((e) => chosen.has(e.id)).map((e) => [e.id, boxOf(e)]));
		void writeElements(layout.id, movedGroup(layout.elements, from, by[0], by[1], canvas, layout.grid), false);
	}

	async function removeChosen() {
		const gone = new Set(chosen);
		clearSubject();
		await writeElements(layout.id, removeElements(layout.elements, gone));
	}

	// ── Copy, cut, paste and duplicate ───────────────────────────────────────
	//
	// Through `localStorage`, as the flow editor's are, and for its reason: a page off
	// `localhost` cannot read the system clipboard without asking. A copy in one layout
	// pastes into any other, in this tab or another on the same browser.

	function copySelected(): boolean {
		if (chosen.size === 0) return false;
		try {
			localStorage.setItem(LAYOUT_CLIP_KEY, JSON.stringify(copyElements(layout.elements, chosen)));
		} catch {
			return false;
		}
		return true;
	}

	/** New elements written in one act, and made the selection. */
	async function arrive(fresh: LayoutElement[]) {
		if (fresh.length === 0) return;
		await writeElements(layout.id, [...layout.elements, ...fresh]);
		subject.set(fresh.map(refOf));
	}

	/** What was copied, in the middle of the view. */
	async function paste() {
		let clip: LayoutClip | null = null;
		try {
			clip = JSON.parse(localStorage.getItem(LAYOUT_CLIP_KEY) ?? 'null');
		} catch {
			clip = null;
		}
		if (!clip?.elements?.length) return;
		const middle = toCanvas(view, screen.width / 2, screen.height / 2);
		await arrive(pastedElements(clip, middle, layout.elements, canvas, layout.grid));
	}

	/** The selection again, where it is — which steps it a cell off itself. */
	async function duplicate() {
		const clip = copyElements(layout.elements, chosen);
		const bounds = boundsOf(clip.elements.map(boxOf));
		if (!bounds) return;
		const at = { x: bounds.x + bounds.w / 2, y: bounds.y + bounds.h / 2 };
		await arrive(pastedElements(clip, at, layout.elements, canvas, layout.grid));
	}

	// ── The view, and the lasso ───────────────────────────────────────────────

	/** Space held down: the left button pans, as in a drawing program. */
	let spaceHeld = false;
	/** Whether the press that is ending swept a lasso or moved the view. */
	let swept = false;
	/** The lasso as drawn, in screen pixels inside the desk. */
	let lasso = $state<{ left: number; top: number; width: number; height: number } | null>(null);

	/** Whether a press moves the view rather than anything on it. */
	const pansFrom = (event: PointerEvent) => event.button === 1 || (event.button === 0 && (event.altKey || spaceHeld));

	const local = (event: { clientX: number; clientY: number }) => {
		const bounds = host!.getBoundingClientRect();
		return { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
	};

	/** Drag the view, from this press until the pointer comes up. */
	function pan(event: PointerEvent) {
		if (!host) return;
		event.preventDefault();
		event.stopPropagation();
		const start = { x: event.clientX, y: event.clientY, view };
		const target = host;
		target.setPointerCapture(event.pointerId);
		const move = (e: PointerEvent) => {
			if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > SLOP_PX) swept = true;
			moved_ = clampView(
				{ ...start.view, x: start.view.x + e.clientX - start.x, y: start.view.y + e.clientY - start.y },
				canvas,
				screen
			);
		};
		const up = (e: PointerEvent) => {
			target.releasePointerCapture?.(e.pointerId);
			target.removeEventListener('pointermove', move);
			target.removeEventListener('pointerup', up);
			target.removeEventListener('pointercancel', up);
		};
		target.addEventListener('pointermove', move);
		target.addEventListener('pointerup', up);
		target.addEventListener('pointercancel', up);
	}

	/** A press on the empty canvas: a pan, or a lasso over what it sweeps. */
	function pressCanvas(event: PointerEvent) {
		if (!host || (event.target instanceof Element && event.target.closest('.handle, .fit'))) return;
		if (pansFrom(event)) {
			pan(event);
			return;
		}
		if (event.button !== 0) return;
		const start = local(event);
		const adding = event.metaKey || event.ctrlKey || event.shiftKey;
		const before = adding ? $subject.filter((r) => r.kind === 'layout-element') : [];
		const target = host;
		target.setPointerCapture(event.pointerId);
		let sweeping = false;
		const move = (e: PointerEvent) => {
			const at = local(e);
			if (!sweeping && Math.hypot(at.x - start.x, at.y - start.y) < SLOP_PX) return;
			sweeping = true;
			swept = true;
			lasso = { left: Math.min(at.x, start.x), top: Math.min(at.y, start.y), width: Math.abs(at.x - start.x), height: Math.abs(at.y - start.y) };
			const caught = inLasso(layout.elements, rectBetween(toCanvas(view, start.x, start.y), toCanvas(view, at.x, at.y)));
			const refs = caught.map((id) => ({ kind: 'layout-element', id }) as ObjectRef);
			subject.set([...before, ...refs.filter((r) => !before.some((b) => b.id === r.id))]);
		};
		const up = (e: PointerEvent) => {
			target.releasePointerCapture?.(e.pointerId);
			target.removeEventListener('pointermove', move);
			target.removeEventListener('pointerup', up);
			target.removeEventListener('pointercancel', up);
			lasso = null;
		};
		target.addEventListener('pointermove', move);
		target.addEventListener('pointerup', up);
		target.addEventListener('pointercancel', up);
	}

	/**
	 * Two fingers on a trackpad move the view, and a pinch — which a browser reports as
	 * a wheel with Ctrl held — zooms about the pointer. Listened to by hand, because a
	 * passive listener cannot keep the page from scrolling as well.
	 */
	$effect(() => {
		if (!host) return;
		const target = host;
		const wheel = (event: WheelEvent) => {
			event.preventDefault();
			const at = local(event);
			if (event.ctrlKey) {
				moved_ = zoomAt(view, Math.exp(-event.deltaY * 0.01), at, canvas, screen);
			} else {
				moved_ = clampView({ ...view, x: view.x - event.deltaX, y: view.y - event.deltaY }, canvas, screen);
			}
		};
		target.addEventListener('wheel', wheel, { passive: false });
		return () => target.removeEventListener('wheel', wheel);
	});

	function spaceKey(event: KeyboardEvent, down: boolean) {
		if (event.key !== ' ' || (event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]'))) return;
		if (!down || (host && document.activeElement instanceof Node && host.contains(document.activeElement))) {
			spaceHeld = down;
			// Space is Go on a cue sheet: held over a layout it is a hand, and nothing else.
			if (down) event.preventDefault();
		}
	}

	// ── A fixture dropped on the drawing ─────────────────────────────────────

	let dropping = $state(false);

	function over(event: DragEvent) {
		if (!$canEdit || !event.dataTransfer?.types.includes(FIXTURE_MIME)) return;
		event.preventDefault();
		event.dataTransfer.dropEffect = 'copy';
		dropping = true;
	}

	async function drop(event: DragEvent) {
		dropping = false;
		const id = event.dataTransfer?.getData(FIXTURE_MIME);
		if (!id || !$canEdit || !host) return;
		event.preventDefault();
		const fixture = show.fixtures.find((f) => f.id === id);
		if (!fixture) return;
		const bounds = host.getBoundingClientRect();
		const at = toCanvas(view, event.clientX - bounds.left, event.clientY - bounds.top);
		const added = sensorElements([fixture], show.types, at, canvas, layout.grid);
		if (added.length === 0) {
			addToast(`${fixture.name} reports nothing: it has no input parameters to put on a layout.`, 'info');
			return;
		}
		await writeElements(layout.id, [...layout.elements, ...added]);
		if (added.length === 1) selectSubject(refOf(added[0]));
	}

	const GRIPS: { grip: Grip; style: string }[] = [
		{ grip: 'nw', style: 'left:-5px;top:-5px;cursor:nwse-resize' },
		{ grip: 'ne', style: 'right:-5px;top:-5px;cursor:nesw-resize' },
		{ grip: 'sw', style: 'left:-5px;bottom:-5px;cursor:nesw-resize' },
		{ grip: 'se', style: 'right:-5px;bottom:-5px;cursor:nwse-resize' },
		{ grip: 'n', style: 'left:50%;top:-5px;margin-left:-5px;cursor:ns-resize' },
		{ grip: 's', style: 'left:50%;bottom:-5px;margin-left:-5px;cursor:ns-resize' },
		{ grip: 'w', style: 'left:-5px;top:50%;margin-top:-5px;cursor:ew-resize' },
		{ grip: 'e', style: 'right:-5px;top:50%;margin-top:-5px;cursor:ew-resize' }
	];
</script>

<!-- The canvas itself takes keys (delete, nudge) and a press on its empty part, which
     makes the layout the subject: an application surface, as the plan's is. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
	class="desk"
	class:stale={!live}
	class:dropping
	bind:this={host}
	role="application"
	aria-label="Layout {layout.name}"
	tabindex="0"
	onkeydown={key}
	onpointerdown={pressCanvas}
	onclick={background}
	ondragover={over}
	ondragleave={() => (dropping = false)}
	ondrop={drop}
>
	{#if screen.width > 0}
		<LayoutDrawing {layout} {show} {view} {screen} selected={chosen} {overrides} />
		{#each layout.elements as element (element.id)}
			{@const shown = overrides?.get(element.id) ?? element}
			{@const at = onScreen(boxOf(shown))}
			{@const pressable = live && pressOf(element) !== null}
			<div
				class="handle"
				class:editable={$canEdit}
				class:pressable
				class:chosen={chosen.has(element.id)}
				style="left:{at.left}px;top:{at.top}px;width:{at.width}px;height:{at.height}px"
				role="button"
				tabindex="-1"
				aria-label={element.label || element.target.kind}
				onpointerdown={(e) => begin(e, element, 'move')}
				onpointermove={moved}
				onpointerup={end}
				onpointercancel={end}
				onclick={(e) => tap(e, element)}
				onkeydown={() => {}}
				oncontextmenu={(e) => {
					e.preventDefault();
					openMenu(refOf(element), e.clientX, e.clientY);
				}}
				use:longPress={(x, y) => openMenu(refOf(element), x, y)}
			>
				<!-- The subject without a modifier key, for a finger and a pen. -->
				<button
					class="dot"
					aria-label="Inspect"
					title="Inspect"
					onpointerdown={(e) => e.stopPropagation()}
					onclick={(e) => {
						e.stopPropagation();
						selectSubject(refOf(element));
					}}
				></button>
				{#if $canEdit && one?.id === element.id}
					{#each GRIPS as g (g.grip)}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<span
							class="grip"
							style={g.style}
							onpointerdown={(e) => begin(e, element, g.grip)}
							onpointermove={moved}
							onpointerup={end}
							onpointercancel={end}
						></span>
					{/each}
				{/if}
			</div>
		{/each}
		{#if lasso}
			<div class="lasso" style="left:{lasso.left}px;top:{lasso.top}px;width:{lasso.width}px;height:{lasso.height}px"></div>
		{/if}
		{#if moved_}
			<button class="fit" title="Show the whole layout" onclick={() => (moved_ = null)}>Fit</button>
		{/if}
	{/if}
</div>

<svelte:window onkeydown={(e) => spaceKey(e, true)} onkeyup={(e) => spaceKey(e, false)} onblur={() => (spaceHeld = false)} />

<style>
	.desk {
		position: relative;
		flex: 1;
		min-height: 0;
		overflow: hidden;
		outline: none;
		background: #0f1114;
		/* A lasso and a pan are drags, not text selections. */
		user-select: none;
		-webkit-user-select: none;
		touch-action: none;
	}
	.lasso {
		position: absolute;
		border: 1px dashed var(--accent, #4a90d9);
		background: rgba(74, 144, 217, 0.12);
		pointer-events: none;
	}
	.fit {
		position: absolute;
		right: 8px;
		bottom: 8px;
		padding: 2px 8px;
		font-size: var(--font-xs, 11px);
		background: rgba(15, 17, 20, 0.85);
		color: var(--text-dim, #aaa);
		border: 1px solid var(--line, #333);
		border-radius: 4px;
		cursor: pointer;
	}
	.desk.dropping {
		box-shadow: inset 0 0 0 2px var(--accent, #4a90d9);
	}
	/* Disconnected: what it last said, dimmed, and nothing presses. */
	.stale :global(svg) {
		opacity: 0.45;
		filter: grayscale(0.6);
	}
	.handle {
		position: absolute;
		border: 1px solid transparent;
		border-radius: 3px;
	}
	.handle.pressable {
		cursor: pointer;
	}
	.handle.editable {
		cursor: move;
		touch-action: none;
	}
	.handle:hover {
		border-color: rgba(74, 144, 217, 0.45);
	}
	.handle.chosen {
		border-color: transparent;
	}
	.dot {
		position: absolute;
		top: -5px;
		right: -5px;
		width: 12px;
		height: 12px;
		padding: 0;
		border-radius: 50%;
		border: 1px solid #fff;
		background: var(--accent, #4a90d9);
		opacity: 0;
		cursor: pointer;
	}
	.handle:hover .dot {
		opacity: 1;
	}
	@media (pointer: coarse) {
		.dot {
			opacity: 0.8;
			width: 16px;
			height: 16px;
		}
	}
	.grip {
		position: absolute;
		width: 10px;
		height: 10px;
		background: var(--accent, #4a90d9);
		border: 1px solid #fff;
		border-radius: 2px;
		touch-action: none;
	}
</style>
