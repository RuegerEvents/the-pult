/**
 * Where each flow was last looked at from, on this browser.
 *
 * A view is a screen's business, like a layout: kept in `localStorage`, never in the
 * show, so one console zoomed into a corner of a graph does not move anybody else's.
 * Keyed by flow id, so switching back to a flow puts it where it was left.
 */

export type FlowView = { x: number; y: number; zoom: number };

const KEY = 'pult.flowViews';

function all(): Record<string, FlowView> {
	try {
		return JSON.parse(localStorage.getItem(KEY) ?? '{}') ?? {};
	} catch {
		return {};
	}
}

export function viewOf(flowId: string): FlowView | null {
	const view = all()[flowId];
	return view && Number.isFinite(view.x) && Number.isFinite(view.y) && view.zoom > 0 ? view : null;
}

export function keepView(flowId: string, view: FlowView): void {
	try {
		localStorage.setItem(KEY, JSON.stringify({ ...all(), [flowId]: view }));
	} catch {
		// A private window with no storage forgets views, which is all this costs.
	}
}

/**
 * Whether any of `nodes` is on screen in `view`, for a board `width` × `height`. A
 * view that shows none of the flow is a view nobody wants back.
 */
export function showsAny(
	view: FlowView,
	nodes: { x: number; y: number }[],
	width: number,
	height: number,
	size = { w: 184, h: 90 }
): boolean {
	return nodes.some((n) => {
		const left = n.x * view.zoom + view.x;
		const top = n.y * view.zoom + view.y;
		return left + size.w * view.zoom > 0 && top + size.h * view.zoom > 0 && left < width && top < height;
	});
}
