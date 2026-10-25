/**
 * A lasso: drag a rectangle over a list, and what it touches is picked.
 *
 * An action rather than a component, so anything that draws items can have one by
 * marking them — `data-lasso-id` on each item and `use:lasso` on what holds them —
 * with no wrapper around its markup. The Fixtures sheet is the first; a pool of cards
 * or the plan's symbols take it the same way.
 *
 * What it touches is *previewed* while it is dragged and *selected* when it is let
 * go: selecting on every move made a list that filters on the selection — the
 * Fixtures sheet with "all fixtures" off — reflow under the rectangle and flicker.
 *
 * It starts only once the pointer has moved, so a click is still a click, and the
 * click a finished lasso would otherwise end in is swallowed. The rectangle is drawn
 * `fixed` on the body, because a panel's own overflow would clip it at the edge of the
 * tile it started in.
 *
 * **On a touchscreen one finger lassos and two fingers scroll.** The container is
 * `touch-action: none` (set here), because a browser that is allowed to pan claims a
 * one-finger drag for scrolling and cancels the pointer before a lasso can start — so
 * a lasso only worked from where panning had been switched off, and in only the
 * directions that started there. With panning off, a second finger down cancels the
 * lasso and this scrolls the container itself, by the fingers' midpoint.
 *
 * **And it scrolls while it is dragged**, when the pointer is within an edge band of the
 * container — or a lasso could never reach a row that is off screen. The rectangle's
 * start is kept in the container's content coordinates for that reason: it is the rows
 * that stay put, not the screen.
 */

/** Two boxes overlap, edges touching included. */
export function intersects(
	a: { left: number; top: number; right: number; bottom: number },
	b: { left: number; top: number; right: number; bottom: number }
): boolean {
	return a.left <= b.right && b.left <= a.right && a.top <= b.bottom && b.top <= a.bottom;
}

/** The box two corners make, whichever way the drag went. */
export function boxOf(x0: number, y0: number, x1: number, y1: number) {
	return { left: Math.min(x0, x1), top: Math.min(y0, y1), right: Math.max(x0, x1), bottom: Math.max(y0, y1) };
}

export type LassoOptions = {
	/** A lasso began. `additive` when Shift, Ctrl or Cmd was held: add to what is picked. */
	onstart?: (additive: boolean) => void;
	/** What it touches now, in document order, every time that changes — to highlight. */
	onpreview?: (ids: string[]) => void;
	/** What it touched when it was let go: the selection. */
	onselect: (ids: string[], additive: boolean) => void;
};

/** How far the pointer moves before a press becomes a lasso rather than a click. */
const THRESHOLD = 4;

/** How close to an edge, in pixels, a dragging pointer starts the list scrolling. */
const EDGE = 28;

export function lasso(node: HTMLElement, options: LassoOptions) {
	let opts = options;
	/** Where the press was, in the container's scrolled content coordinates. */
	let start: { x: number; y: number; additive: boolean; pointer: number } | null = null;
	let rect: HTMLDivElement | null = null;
	let last = '';
	let touched: string[] = [];
	/** The latest pointer position, in client coordinates, for the edge scroll. */
	let at = { x: 0, y: 0 };
	let scrolling: number | null = null;
	/** Touch pointers down on the container, for the two-finger scroll. */
	const fingers = new Map<number, { x: number; y: number }>();

	node.style.touchAction = 'none';

	const content = (x: number, y: number) => {
		const box = node.getBoundingClientRect();
		return { x: x - box.left + node.scrollLeft, y: y - box.top + node.scrollTop };
	};
	const client = (x: number, y: number) => {
		const box = node.getBoundingClientRect();
		return { x: x + box.left - node.scrollLeft, y: y + box.top - node.scrollTop };
	};

	function cancel() {
		start = null;
		if (rect) {
			rect.remove();
			rect = null;
			opts.onpreview?.([]);
		}
		stopScrolling();
	}

	function down(event: PointerEvent) {
		if (event.pointerType === 'touch') {
			fingers.set(event.pointerId, { x: event.clientX, y: event.clientY });
			// A second finger is a scroll, never a lasso.
			if (fingers.size >= 2) {
				cancel();
				return;
			}
		}
		if (event.button !== 0) return;
		const target = event.target as HTMLElement;
		if (target.closest('input, select, textarea, [contenteditable]')) return;
		const p = content(event.clientX, event.clientY);
		start = {
			x: p.x,
			y: p.y,
			additive: event.shiftKey || event.ctrlKey || event.metaKey,
			pointer: event.pointerId
		};
		at = { x: event.clientX, y: event.clientY };
	}

	function draw() {
		if (!start || !rect) return;
		const from = client(start.x, start.y);
		const box = boxOf(from.x, from.y, at.x, at.y);
		Object.assign(rect.style, {
			left: `${box.left}px`,
			top: `${box.top}px`,
			width: `${box.right - box.left}px`,
			height: `${box.bottom - box.top}px`
		});
		const ids: string[] = [];
		for (const item of node.querySelectorAll<HTMLElement>('[data-lasso-id]')) {
			if (intersects(box, item.getBoundingClientRect())) ids.push(item.dataset.lassoId!);
		}
		const key = ids.join(',');
		if (key !== last) {
			last = key;
			touched = ids;
			opts.onpreview?.(ids);
		}
	}

	/** Scroll while the pointer sits in an edge band, faster the deeper in it is. */
	function edgeScroll() {
		scrolling = null;
		if (!start || !rect) return;
		const box = node.getBoundingClientRect();
		const speed = (depth: number) => Math.ceil((Math.min(EDGE, depth) / EDGE) * 18);
		let dx = 0;
		let dy = 0;
		if (at.y < box.top + EDGE) dy = -speed(box.top + EDGE - at.y);
		else if (at.y > box.bottom - EDGE) dy = speed(at.y - (box.bottom - EDGE));
		if (at.x < box.left + EDGE) dx = -speed(box.left + EDGE - at.x);
		else if (at.x > box.right - EDGE) dx = speed(at.x - (box.right - EDGE));
		if (dx === 0 && dy === 0) return;
		const before = [node.scrollLeft, node.scrollTop];
		node.scrollBy(dx, dy);
		if (node.scrollLeft === before[0] && node.scrollTop === before[1]) return;
		draw();
		scrolling = requestAnimationFrame(edgeScroll);
	}

	function stopScrolling() {
		if (scrolling !== null) cancelAnimationFrame(scrolling);
		scrolling = null;
	}

	function move(event: PointerEvent) {
		if (event.pointerType === 'touch' && fingers.has(event.pointerId) && fingers.size >= 2) {
			// Two fingers: scroll by how far their midpoint moved.
			const before = midpoint();
			fingers.set(event.pointerId, { x: event.clientX, y: event.clientY });
			const after = midpoint();
			node.scrollBy(before.x - after.x, before.y - after.y);
			return;
		}
		if (event.pointerType === 'touch' && fingers.has(event.pointerId)) {
			fingers.set(event.pointerId, { x: event.clientX, y: event.clientY });
		}
		if (!start || event.pointerId !== start.pointer) return;
		at = { x: event.clientX, y: event.clientY };
		if (!rect) {
			const from = client(start.x, start.y);
			if (Math.hypot(at.x - from.x, at.y - from.y) < THRESHOLD) return;
			// Captured only now: captured from the press, every plain click would land on
			// the container instead of the button that was pressed.
			node.setPointerCapture(event.pointerId);
			rect = document.createElement('div');
			rect.className = 'lasso-rect';
			document.body.appendChild(rect);
			last = '';
			touched = [];
			opts.onstart?.(start.additive);
		}
		draw();
		if (scrolling === null) scrolling = requestAnimationFrame(edgeScroll);
	}

	function midpoint() {
		let x = 0;
		let y = 0;
		for (const p of fingers.values()) {
			x += p.x;
			y += p.y;
		}
		return { x: x / fingers.size, y: y / fingers.size };
	}

	function up(event: PointerEvent) {
		fingers.delete(event.pointerId);
		if (!start || event.pointerId !== start.pointer) return;
		const additive = start.additive;
		start = null;
		stopScrolling();
		if (!rect) return;
		opts.onpreview?.([]);
		opts.onselect(touched, additive);
		rect.remove();
		rect = null;
		if (node.hasPointerCapture(event.pointerId)) node.releasePointerCapture(event.pointerId);
		// The press ended a lasso, not a click on whatever is under the pointer now.
		const swallow = (e: Event) => {
			e.stopPropagation();
			e.preventDefault();
		};
		window.addEventListener('click', swallow, { capture: true, once: true });
		setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0);
	}

	/** The browser took the pointer back, which is never a selection. */
	function lost(event: PointerEvent) {
		fingers.delete(event.pointerId);
		if (start && event.pointerId === start.pointer) cancel();
	}

	node.addEventListener('pointerdown', down);
	node.addEventListener('pointermove', move);
	node.addEventListener('pointerup', up);
	node.addEventListener('pointercancel', lost);
	return {
		update(next: LassoOptions) {
			opts = next;
		},
		destroy() {
			rect?.remove();
			stopScrolling();
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointermove', move);
			node.removeEventListener('pointerup', up);
			node.removeEventListener('pointercancel', lost);
		}
	};
}
