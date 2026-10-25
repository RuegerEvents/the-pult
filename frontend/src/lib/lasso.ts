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

export function lasso(node: HTMLElement, options: LassoOptions) {
	let opts = options;
	let start: { x: number; y: number; additive: boolean; pointer: number } | null = null;
	let rect: HTMLDivElement | null = null;
	let last = '';
	let touched: string[] = [];

	function down(event: PointerEvent) {
		if (event.button !== 0) return;
		const target = event.target as HTMLElement;
		if (target.closest('input, select, textarea, [contenteditable]')) return;
		start = {
			x: event.clientX,
			y: event.clientY,
			additive: event.shiftKey || event.ctrlKey || event.metaKey,
			pointer: event.pointerId
		};
	}

	function move(event: PointerEvent) {
		if (!start || event.pointerId !== start.pointer) return;
		if (!rect) {
			if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < THRESHOLD) return;
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
		const box = boxOf(start.x, start.y, event.clientX, event.clientY);
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

	function up(event: PointerEvent) {
		if (!start || event.pointerId !== start.pointer) return;
		const additive = start.additive;
		start = null;
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

	node.addEventListener('pointerdown', down);
	node.addEventListener('pointermove', move);
	node.addEventListener('pointerup', up);
	node.addEventListener('pointercancel', up);
	return {
		update(next: LassoOptions) {
			opts = next;
		},
		destroy() {
			rect?.remove();
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointermove', move);
			node.removeEventListener('pointerup', up);
			node.removeEventListener('pointercancel', up);
		}
	};
}
