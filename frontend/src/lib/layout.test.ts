import { describe, it, expect } from 'vitest';
import type { LayoutNode } from './generated/index.js';
import {
	addTab,
	findPanel,
	leafAt,
	movePanel,
	normalise,
	panelsIn,
	removePanel,
	resize,
	split,
	splitLeaf,
	tabs,
	tidy
} from './layout.js';

/** rig | [values / selection] — one row, the right half stacked. */
const nested = (): LayoutNode =>
	split('Row', [tabs(['rig']), split('Column', [tabs(['values']), tabs(['selection'])])], [0.6, 0.4]);

const sizesOf = (node: LayoutNode) => (node.type === 'Split' ? node.sizes : null);

describe('reading a tree', () => {
	it('finds the node a path names', () => {
		expect(leafAt(nested(), [0])).toEqual(tabs(['rig']));
		expect(leafAt(nested(), [1, 1])).toEqual(tabs(['selection']));
		expect(leafAt(nested(), [])!.type).toBe('Split');
	});

	it('has nothing for a path that leads nowhere', () => {
		expect(leafAt(nested(), [5])).toBeNull();
		expect(leafAt(nested(), [0, 0])).toBeNull();
	});

	it('lists panels left to right, top to bottom', () => {
		expect(panelsIn(nested())).toEqual(['rig', 'values', 'selection']);
	});

	it('says which tile a panel is in', () => {
		expect(findPanel(nested(), 'selection')).toEqual([1, 1]);
		expect(findPanel(nested(), 'patch')).toBeNull();
	});
});

describe('adding a panel', () => {
	it('puts a tab in a group and brings it to the front', () => {
		const next = addTab(nested(), [0], 'plan');
		expect(leafAt(next, [0])).toEqual(tabs(['rig', 'plan'], 1));
	});

	it('brings a panel already there forward rather than opening it twice', () => {
		const twice = addTab(addTab(nested(), [0], 'plan'), [0], 'rig');
		expect(leafAt(twice, [0])).toEqual(tabs(['rig', 'plan'], 0));
	});

	it('divides a tile and puts the new panel on the side asked for', () => {
		const next = splitLeaf(tabs(['rig']), [], 'left', 'plan');
		expect(panelsIn(next)).toEqual(['plan', 'rig']);
		expect(next.type === 'Split' && next.direction).toBe('Row');
	});

	it('splits downwards for a drop on the bottom', () => {
		const next = splitLeaf(tabs(['rig']), [], 'bottom', 'playback');
		expect(panelsIn(next)).toEqual(['rig', 'playback']);
		expect(next.type === 'Split' && next.direction).toBe('Column');
	});

	it('keeps a row a row rather than nesting one inside it', () => {
		// Splitting the left tile of a row sideways has to widen the row, or the
		// gutter between the first two tiles would move the wrong pair.
		const next = splitLeaf(nested(), [0], 'right', 'plan');
		expect(next.type === 'Split' && next.children.length).toBe(3);
		expect(panelsIn(next)).toEqual(['rig', 'plan', 'values', 'selection']);
	});

	it('gives the new tile half of what the old one had', () => {
		const next = splitLeaf(nested(), [0], 'right', 'plan');
		const sizes = sizesOf(next)!;
		expect(sizes[0]).toBeCloseTo(0.3, 6);
		expect(sizes[1]).toBeCloseTo(0.3, 6);
		expect(sizes[2]).toBeCloseTo(0.4, 6);
	});
});

describe('removing a panel', () => {
	it('takes a tab out and leaves the rest of the group', () => {
		const three = addTab(nested(), [0], 'plan');
		const next = removePanel(three, [0], 'plan');
		expect(leafAt(next, [0])).toEqual(tabs(['rig'], 0));
	});

	it('removes the tile when its last panel goes, and collapses the split', () => {
		const next = removePanel(nested(), [0], 'rig');
		// Only the stacked pair is left, so the row it was in is gone.
		expect(next.type === 'Split' && next.direction).toBe('Column');
		expect(panelsIn(next)).toEqual(['values', 'selection']);
	});

	it('leaves an empty tree rather than something unrenderable', () => {
		const alone = tabs(['rig']);
		expect(removePanel(alone, [], 'rig')).toEqual(tabs([]));
	});

	it('keeps the active tab pointing at the same panel', () => {
		const group = tabs(['a', 'b', 'c'], 2);
		expect(removePanel(group, [], 'a')).toEqual(tabs(['b', 'c'], 1));
	});

	it('clamps the active tab when the one after it goes', () => {
		const group = tabs(['a', 'b'], 1);
		expect(removePanel(group, [], 'b')).toEqual(tabs(['a'], 0));
	});
});

describe('moving a panel', () => {
	it('drops one into another tile as a tab', () => {
		const next = movePanel(nested(), { path: [1, 1], panel: 'selection' }, [0], 'center');
		expect(leafAt(next, [0])).toEqual(tabs(['rig', 'selection'], 1));
		expect(panelsIn(next)).toEqual(['rig', 'selection', 'values']);
	});

	it('divides the tile it was dropped on the edge of', () => {
		const next = movePanel(nested(), { path: [1, 1], panel: 'selection' }, [0], 'bottom');
		expect(panelsIn(next)).toEqual(['rig', 'selection', 'values']);
		expect(findPanel(next, 'selection')![0]).toBe(0);
	});

	it('collapses the tile the panel came from once it is empty', () => {
		const next = movePanel(nested(), { path: [0], panel: 'rig' }, [1, 0], 'center');
		expect(next.type === 'Split' && next.direction).toBe('Column');
		expect(panelsIn(next)).toEqual(['values', 'rig', 'selection']);
	});

	it('does nothing when a lone panel is dropped on the edge of its own tile', () => {
		const tree = nested();
		expect(movePanel(tree, { path: [0], panel: 'rig' }, [0], 'left')).toBe(tree);
	});

	it('sends a tab to the back of its own group when dropped on its centre', () => {
		const tree = split('Row', [tabs(['a', 'b'], 0), tabs(['c'])]);
		const next = movePanel(tree, { path: [0], panel: 'a' }, [0], 'center');
		expect(leafAt(next, [0])).toEqual(tabs(['b', 'a'], 1));
	});

	it('refuses a drop on something that is not a tile', () => {
		const tree = nested();
		expect(movePanel(tree, { path: [0], panel: 'rig' }, [1], 'center')).toBe(tree);
	});
});

describe('resizing', () => {
	it('takes from one tile and gives to the one before it', () => {
		const sizes = sizesOf(resize(nested(), [], 0, 0.1))!;
		expect(sizes[0]).toBeCloseTo(0.7, 6);
		expect(sizes[1]).toBeCloseTo(0.3, 6);
	});

	it('will not squeeze a tile out of existence', () => {
		const next = resize(nested(), [], 0, 0.9);
		const sizes = sizesOf(next)!;
		expect(sizes[1]).toBeCloseTo(0.1, 6);
		expect(sizes[0]).toBeCloseTo(0.9, 6);
	});

	it('will not drag a tile below the minimum from the other side either', () => {
		const next = resize(nested(), [], 0, -0.9);
		expect(sizesOf(next)![0]).toBeCloseTo(0.1, 6);
	});

	it('leaves a gutter that is not there alone', () => {
		const tree = nested();
		expect(resize(tree, [], 4, 0.1)).toBe(tree);
	});

	it('resizes a split further down the tree', () => {
		const next = resize(nested(), [1], 0, 0.2);
		expect(sizesOf(leafAt(next, [1])!)![0]).toBeCloseTo(0.7, 6);
	});
});

describe('tidying', () => {
	it('turns a split with one child into that child', () => {
		expect(tidy(split('Row', [tabs(['rig'])]))).toEqual(tabs(['rig']));
	});

	it('flattens a split running the same way as its parent', () => {
		const messy = split('Row', [tabs(['a']), split('Row', [tabs(['b']), tabs(['c'])])], [0.5, 0.5]);
		const clean = tidy(messy);
		expect(clean.type === 'Split' && clean.children.length).toBe(3);
		expect(sizesOf(clean)).toEqual([0.5, 0.25, 0.25]);
	});

	it('leaves a split running the other way nested', () => {
		const clean = tidy(nested());
		expect(clean.type === 'Split' && clean.children.length).toBe(2);
	});

	it('normalises shares that do not add up', () => {
		expect(normalise([2, 2])).toEqual([0.5, 0.5]);
		expect(normalise([0, 0])).toEqual([0.5, 0.5]);
	});
});
