/**
 * The workspace tree, as maths.
 *
 * A layout is a tree of splits and tab groups — the same shape a tiling window
 * manager has, and the same shape the showfile stores. Everything here takes a tree
 * and returns a new one, so the store is a single assignment and Svelte's own
 * reactivity does the rest; nothing has to notice that a node three levels down was
 * mutated.
 *
 * A **path** is the list of child indices from the root: `[]` is the root, `[0, 1]`
 * is the second child of the first. That is enough to name any node, needs nothing
 * stored on the nodes themselves, and survives the round trip through JSON.
 *
 * Two invariants are kept by {@link tidy}, which every operation ends with:
 *
 * - a split never contains a split running the same way, so a row of four panels is
 *   one row and not three nested pairs — which matters because a gutter drags the
 *   two panels either side of it, and nesting would make some gutters move panels
 *   that are not next to them;
 * - a split never has fewer than two children, and an empty tab group does not exist,
 *   so closing the last panel of a tile removes the tile rather than leaving a hole.
 */

import type { LayoutNode, SplitDirection } from './generated/index.js';

export type Path = number[];
/** Where a dropped panel goes relative to the tile it was dropped on. */
export type DropSide = 'left' | 'right' | 'top' | 'bottom' | 'center';

/** No tile may be squeezed below a tenth of its split, so it stays grabbable. */
const MIN_SHARE = 0.1;

export const tabs = (panels: string[], active = 0): LayoutNode => ({
	type: 'Tabs',
	panels,
	active
});

export const split = (
	direction: SplitDirection,
	children: LayoutNode[],
	sizes?: number[]
): LayoutNode => ({
	type: 'Split',
	direction,
	sizes: normalise(sizes ?? children.map(() => 1)),
	children
});

// ── Reading ───────────────────────────────────────────────────────────────────

/** The node at a path, or null if the path does not lead anywhere. */
export function leafAt(tree: LayoutNode, path: Path): LayoutNode | null {
	let node: LayoutNode = tree;
	for (const index of path) {
		if (node.type !== 'Split') return null;
		const child = node.children[index];
		if (!child) return null;
		node = child;
	}
	return node;
}

/** Every panel the tree holds, in the order it holds them. */
export function panelsIn(tree: LayoutNode): string[] {
	return tree.type === 'Tabs' ? [...tree.panels] : tree.children.flatMap(panelsIn);
}

/**
 * The tab group a panel is in, or null when it is not open.
 *
 * Not used by the operations here — they are told where to act. It is how anything
 * *outside* the tree asks where a panel went, which is what "open this panel" has to
 * know before it can decide between opening one and bringing one forward.
 */
export function findPanel(tree: LayoutNode, panel: string): Path | null {
	if (tree.type === 'Tabs') return tree.panels.includes(panel) ? [] : null;
	for (const [index, child] of tree.children.entries()) {
		const found = findPanel(child, panel);
		if (found) return [index, ...found];
	}
	return null;
}

// ── Changing ──────────────────────────────────────────────────────────────────

/** Put a panel in the tab group at `path`, or bring it forward if it is there. */
export function addTab(tree: LayoutNode, path: Path, panel: string): LayoutNode {
	return tidy(
		replaceAt(tree, path, (node) => {
			if (node.type !== 'Tabs') return node;
			const at = node.panels.indexOf(panel);
			if (at >= 0) return tabs(node.panels, at);
			return tabs([...node.panels, panel], node.panels.length);
		})
	);
}

/** Divide the tile at `path` and put a panel in the new half. */
export function splitLeaf(
	tree: LayoutNode,
	path: Path,
	side: Exclude<DropSide, 'center'>,
	panel: string
): LayoutNode {
	const direction: SplitDirection = side === 'left' || side === 'right' ? 'Row' : 'Column';
	const fresh = tabs([panel]);
	return tidy(
		replaceAt(tree, path, (node) =>
			split(direction, side === 'left' || side === 'top' ? [fresh, node] : [node, fresh], [1, 1])
		)
	);
}

/** Take a panel out of the tab group at `path`. */
export function removePanel(tree: LayoutNode, path: Path, panel: string): LayoutNode {
	return tidy(withoutPanel(tree, path, panel));
}

/**
 * Move a panel from one tile to another.
 *
 * The removal is done first but left untidied, so the target path still means what
 * it meant when the drag started — tidying between the two steps would collapse the
 * tile the panel came from and renumber everything after it.
 */
export function movePanel(
	tree: LayoutNode,
	from: { path: Path; panel: string },
	to: Path,
	side: DropSide
): LayoutNode {
	const target = leafAt(tree, to);
	if (!target || target.type !== 'Tabs') return tree;

	// Dropping a lone panel back onto the edge of its own tile would divide the tile
	// in two and then collapse it again: a lot of tree for no change.
	const source = leafAt(tree, from.path);
	if (
		source?.type === 'Tabs' &&
		source.panels.length === 1 &&
		samePath(from.path, to) &&
		side !== 'center'
	) {
		return tree;
	}

	const emptied = withoutPanel(tree, from.path, from.panel);
	return side === 'center'
		? addTab(emptied, to, from.panel)
		: splitLeaf(emptied, to, side, from.panel);
}

/**
 * Move a gutter.
 *
 * `delta` is a fraction of the whole split, taken from the tile after the gutter and
 * given to the one before it. Neither may fall below {@link MIN_SHARE}, so a tile
 * cannot be dragged out of existence — the way to close one is to close it.
 */
export function resize(
	tree: LayoutNode,
	path: Path,
	gutter: number,
	delta: number
): LayoutNode {
	return replaceAt(tree, path, (node) => {
		if (node.type !== 'Split') return node;
		const before = node.sizes[gutter];
		const after = node.sizes[gutter + 1];
		if (before === undefined || after === undefined) return node;
		const room = Math.min(after - MIN_SHARE, Math.max(MIN_SHARE - before, delta));
		if (room === 0) return node;
		const sizes = [...node.sizes];
		sizes[gutter] = before + room;
		sizes[gutter + 1] = after - room;
		return { ...node, sizes };
	});
}

// ── Machinery ─────────────────────────────────────────────────────────────────

function replaceAt(
	tree: LayoutNode,
	path: Path,
	change: (node: LayoutNode) => LayoutNode
): LayoutNode {
	if (path.length === 0) return change(tree);
	if (tree.type !== 'Split') return tree;
	const [index, ...rest] = path;
	const child = tree.children[index];
	if (!child) return tree;
	const children = [...tree.children];
	children[index] = replaceAt(child, rest, change);
	return { ...tree, children };
}

/** Take a panel out, leaving the tree the shape it was — empty tiles and all. */
function withoutPanel(tree: LayoutNode, path: Path, panel: string): LayoutNode {
	return replaceAt(tree, path, (node) => {
		if (node.type !== 'Tabs') return node;
		const at = node.panels.indexOf(panel);
		if (at < 0) return node;
		const panels = node.panels.filter((p) => p !== panel);
		return tabs(panels, Math.min(node.active > at ? node.active - 1 : node.active, Math.max(panels.length - 1, 0)));
	});
}

/**
 * Restore the tree's two invariants, bottom up.
 *
 * Every operation ends here rather than each one worrying about it, which is why
 * they can all be written as "put this node there".
 */
export function tidy(node: LayoutNode): LayoutNode {
	if (node.type === 'Tabs') return node;

	const children: LayoutNode[] = [];
	const sizes: number[] = [];
	const even = 1 / Math.max(node.children.length, 1);

	node.children.forEach((raw, index) => {
		const child = tidy(raw);
		const share = node.sizes[index] ?? even;
		if (child.type === 'Tabs') {
			if (child.panels.length === 0) return; // a tile with nothing in it is no tile
			children.push(child);
			sizes.push(share);
			return;
		}
		if (child.direction !== node.direction) {
			children.push(child);
			sizes.push(share);
			return;
		}
		// A row inside a row is one row: hand the grandchildren up, each keeping its
		// share of the share their parent had.
		const total = child.sizes.reduce((a, b) => a + b, 0) || 1;
		child.children.forEach((grandchild, j) => {
			children.push(grandchild);
			sizes.push((share * (child.sizes[j] ?? 1 / child.children.length)) / total);
		});
	});

	if (children.length === 0) return tabs([]);
	if (children.length === 1) return children[0];
	return { type: 'Split', direction: node.direction, sizes: normalise(sizes), children };
}

/** Shares that sum to one, and never to nothing. */
export function normalise(sizes: number[]): number[] {
	const total = sizes.reduce((a, b) => a + (b > 0 ? b : 0), 0);
	if (!(total > 0)) return sizes.map(() => 1 / Math.max(sizes.length, 1));
	return sizes.map((s) => (s > 0 ? s : 0) / total);
}

const samePath = (a: Path, b: Path) => a.length === b.length && a.every((v, i) => v === b[i]);
