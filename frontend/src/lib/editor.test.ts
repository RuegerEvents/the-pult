/**
 * Building a rig: the parts of it that are arithmetic rather than writes.
 *
 * What a delete would take with it, and where a distribute puts things. Both are
 * decisions an operator sees before anything is written — the prompt names counts, and
 * the align strip moves what is on screen — so both are worth being sure of without a
 * socket in the way.
 */
import { describe, expect, it } from 'vitest';

import type { Fixture, SceneObject } from './generated/index.js';
import { IDENTITY } from './scene.js';
import { distributed, fixturesOn, spaced, subtreeOf, whatWouldGo } from './stores/editor.js';
import { aFixture } from './test-fixtures.js';

function object(id: string, parent: string | null = null): SceneObject {
	return {
		id,
		name: id,
		kind: 'Truss',
		transform: IDENTITY,
		parent,
		layer: null,
		class: null,
		geometry: [],
		symbol: null,
		catalogue: null,
		properties: null,
		locked: false
	};
}

// A run: a group with two sections under it, and a section under one of those.
const rig: SceneObject[] = [
	object('run'),
	object('a', 'run'),
	object('b', 'run'),
	object('a-tail', 'a'),
	object('elsewhere')
];

const lights: Fixture[] = [
	aFixture({ id: 'one', parent: 'a' }),
	aFixture({ id: 'two', parent: 'b' }),
	aFixture({ id: 'far', parent: 'elsewhere' }),
	aFixture({ id: 'loose', parent: null })
];

describe('what hangs off what', () => {
	it('finds everything under a run, however deep', () => {
		expect([...subtreeOf(['run'], rig)].sort()).toEqual(['a', 'a-tail', 'b', 'run']);
	});

	it('finds the lights on all of it', () => {
		expect(fixturesOn(subtreeOf(['run'], rig), lights).map((f) => f.id).sort()).toEqual([
			'one',
			'two'
		]);
	});

	it('leaves a piece with nothing on it alone', () => {
		expect([...subtreeOf(['elsewhere'], rig)]).toEqual(['elsewhere']);
	});
});

describe('what deleting would take with it', () => {
	it('names the children and the lights, and not the thing itself', () => {
		const going = whatWouldGo(new Set(['run']), rig, lights);

		expect(going.objects.map((o) => o.id).sort()).toEqual(['a', 'a-tail', 'b']);
		expect(going.fixtures.map((f) => f.id).sort()).toEqual(['one', 'two']);
	});

	it('is empty for a bare piece, which is what lets it go without asking', () => {
		const going = whatWouldGo(new Set(['a-tail']), rig, lights);

		expect(going.objects).toHaveLength(0);
		expect(going.fixtures).toHaveLength(0);
	});
});

describe('lining things up', () => {
	it('spreads evenly and leaves the two ends where they are', () => {
		expect(distributed([0, 1, 9, 3])).toEqual([0, 3, 9, 6]);
	});

	it('leaves two alone, because two are already distributed', () => {
		expect(distributed([4, 1])).toEqual([4, 1]);
	});

	it('spaces from the first one, in the order they are already in', () => {
		expect(spaced([0, 5, 2], 1)).toEqual([0, 2, 1]);
	});
});
