import { readFileSync } from 'node:fs';

import { describe, it, expect } from 'vitest';

import type { Fixture, Vec3 } from './generated/index.js';
import {
	describe as describeQuery,
	evaluate,
	EMPTY_QUERY,
	idsQuery,
	inBox,
	inCone,
	isManualList,
	NO_ORDER,
	normalise,
	sortSelection,
	type SelectionQuery
} from './selection.js';
import { aFixtureType } from './test-fixtures.js';

const at = (name: string, x: number, y: number, z: number, typeId = 'par'): Fixture => ({
	id: name,
	name,
	fixture_type_id: typeId,
	address: { Dmx: { mode: 'Default', breaks: [{ universe: 1, address: 1 }] } },
	position: { Point: { x, y, z } },
	sensed_values: {},
	live_effects: {},
	live_fades: {},
	home_values: {}
});

const unplaced = (name: string, typeId = 'par'): Fixture => ({ ...at(name, 0, 0, 0, typeId), position: null });

/** A row of four across the front, one behind, and one nobody has placed. */
const rig: Fixture[] = [
	at('Front 1', -3, 5, 2),
	at('Front 2', -1, 5, 2),
	at('Front 3', 1, 5, 2),
	at('Front 4', 3, 5, 2),
	at('Mover back', 0, 6, -4, 'mover'),
	unplaced('In the case', 'mover')
];

const query = (q: Partial<SelectionQuery>): SelectionQuery => ({ ...EMPTY_QUERY, ...q });

describe('picking fixtures out of a rig', () => {
	it('selects nothing for an empty query', () => {
		expect(evaluate(EMPTY_QUERY, rig)).toEqual([]);
	});

	it('selects everything, placed or not', () => {
		const all = evaluate(query({ clauses: [{ combine: 'Add', term: { kind: 'Everything' } }] }), rig);
		expect(all).toHaveLength(6);
		expect(all).toContain('In the case');
	});

	it('selects by type, which is how you reach an unplaced rig', () => {
		const movers = evaluate(
			query({ clauses: [{ combine: 'Add', term: { kind: 'OfType', typeId: 'mover' } }] }),
			rig
		);
		expect(movers).toEqual(['Mover back', 'In the case']);
	});

	it('selects by name, case-insensitively', () => {
		const front = evaluate(
			query({ clauses: [{ combine: 'Add', term: { kind: 'Named', text: 'front' } }] }),
			rig
		);
		expect(front).toHaveLength(4);
	});

	/**
	 * A hand-picked list is a query like any other, so clicking and a geometric
	 * selection are the same kind of thing and can be combined.
	 */
	it('treats a hand-picked list as a query', () => {
		expect(evaluate(idsQuery(['Front 2', 'Front 3']), rig)).toEqual(['Front 2', 'Front 3']);
		expect(isManualList(idsQuery(['Front 2']))).toBe(true);
		expect(isManualList(query({ clauses: [{ combine: 'Add', term: { kind: 'Everything' } }] }))).toBe(false);
	});
});

describe('geometry', () => {
	it('finds what is inside a sphere and nothing outside it', () => {
		const near = evaluate(
			query({
				clauses: [
					{ combine: 'Add', term: { kind: 'Sphere', centre: { x: 0, y: 5, z: 2 }, radius: 1.5 } }
				]
			}),
			rig
		);
		expect(near).toEqual(['Front 2', 'Front 3']);
	});

	it('finds what is inside a box, whichever way round the corners were given', () => {
		const from: Vec3 = { x: -4, y: 4, z: 1 };
		const to: Vec3 = { x: 0, y: 6, z: 3 };
		const forwards = evaluate(query({ clauses: [{ combine: 'Add', term: { kind: 'Box', from, to } }] }), rig);
		const backwards = evaluate(query({ clauses: [{ combine: 'Add', term: { kind: 'Box', from: to, to: from } }] }), rig);

		expect(forwards).toEqual(['Front 1', 'Front 2']);
		expect(backwards).toEqual(forwards);
	});

	/**
	 * The spec's radial selection. `angleDeg` is the half-angle, because that is
	 * what a beam angle is quoted as and what an operator has in their head.
	 */
	it('finds what is inside a cone from a point', () => {
		// Straight down the middle of the room from front of house, narrow.
		const narrow = evaluate(
			query({
				clauses: [
					{
						combine: 'Add',
						term: {
							kind: 'Cone',
							from: { x: 0, y: 5, z: 10 },
							direction: { x: 0, y: 0, z: -1 },
							angleDeg: 12,
							reach: 30
						}
					}
				]
			}),
			rig
		);
		expect(narrow).toEqual(['Front 2', 'Front 3', 'Mover back']);
	});

	it('a wider cone reaches further across', () => {
		const term = (angleDeg: number) => ({
			kind: 'Cone' as const,
			from: { x: 0, y: 5, z: 10 },
			direction: { x: 0, y: 0, z: -1 },
			angleDeg,
			reach: 30
		});
		const narrow = evaluate(query({ clauses: [{ combine: 'Add', term: term(12) }] }), rig);
		const wide = evaluate(query({ clauses: [{ combine: 'Add', term: term(35) }] }), rig);
		expect(wide.length).toBeGreaterThan(narrow.length);
		for (const id of narrow) expect(wide).toContain(id);
	});

	/**
	 * Reach is what stops a wide cone selecting the whole room behind the rig. The
	 * front row is about 8 m from this apex, so a reach of 5 excludes it however
	 * wide the angle is.
	 */
	it('a cone stops at its reach, however wide the angle', () => {
		const cone = (reach: number) => ({
			kind: 'Cone' as const,
			from: { x: 0, y: 5, z: 10 },
			direction: { x: 0, y: 0, z: -1 },
			angleDeg: 60,
			reach
		});
		expect(evaluate(query({ clauses: [{ combine: 'Add', term: cone(5) }] }), rig)).toEqual([]);
		expect(
			evaluate(query({ clauses: [{ combine: 'Add', term: cone(9) }] }), rig).length
		).toBeGreaterThan(0);
	});

	/** A cone drawn from a fixture should include that fixture, not miss it by a hair. */
	it('includes a point sitting exactly at the apex', () => {
		expect(inCone({ x: 1, y: 2, z: 3 }, { x: 1, y: 2, z: 3 }, { x: 0, y: -1, z: 0 }, 5, 10)).toBe(true);
	});

	it('a direction of nothing selects nothing rather than everything', () => {
		expect(inCone({ x: 0, y: 0, z: 1 }, { x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: 0 }, 90, 10)).toBe(false);
		expect(normalise({ x: 0, y: 0, z: 0 })).toBeNull();
	});

	it('a box with no volume still matches a point exactly on it', () => {
		const p = { x: 1, y: 1, z: 1 };
		expect(inBox(p, p, p)).toBe(true);
	});

	/**
	 * A light nobody has told the console about cannot be "downstage". Failing every
	 * geometric term is the honest answer, and it is why `Everything` and `OfType`
	 * exist — they are how an unplaced rig is reachable at all.
	 */
	it('an unplaced fixture fails every geometric term', () => {
		const huge = evaluate(
			query({
				clauses: [
					{ combine: 'Add', term: { kind: 'Sphere', centre: { x: 0, y: 0, z: 0 }, radius: 1e6 } }
				]
			}),
			rig
		);
		expect(huge).not.toContain('In the case');
		expect(huge).toHaveLength(5);
	});
});

describe('building a selection up', () => {
	/**
	 * Read left to right, which is how an operator says it: "all the movers, of
	 * those the downstage ones, but not the broken one."
	 */
	it('adds, narrows and removes in order', () => {
		const picked = evaluate(
			query({
				clauses: [
					{ combine: 'Add', term: { kind: 'Named', text: 'front' } },
					{ combine: 'Drop', term: { kind: 'Ids', ids: ['Front 1'] } }
				]
			}),
			rig
		);
		expect(picked).toEqual(['Front 2', 'Front 3', 'Front 4']);
	});

	it('narrows rather than unions with Keep', () => {
		const downstageMovers = evaluate(
			query({
				clauses: [
					{ combine: 'Add', term: { kind: 'Everything' } },
					{ combine: 'Keep', term: { kind: 'OfType', typeId: 'par' } },
					{ combine: 'Keep', term: { kind: 'Box', from: { x: 0, y: 0, z: 0 }, to: { x: 5, y: 9, z: 5 } } }
				]
			}),
			rig
		);
		expect(downstageMovers).toEqual(['Front 3', 'Front 4']);
	});

	it('adding the same fixture twice does not move it to the end', () => {
		const picked = evaluate(
			query({
				clauses: [
					{ combine: 'Add', term: { kind: 'Ids', ids: ['Front 1', 'Front 2'] } },
					{ combine: 'Add', term: { kind: 'Ids', ids: ['Front 1', 'Front 3'] } }
				]
			}),
			rig
		);
		expect(picked).toEqual(['Front 1', 'Front 2', 'Front 3']);
	});

	it('a Keep with nothing to keep leaves nothing', () => {
		const picked = evaluate(
			query({
				clauses: [
					{ combine: 'Add', term: { kind: 'Named', text: 'front' } },
					{ combine: 'Keep', term: { kind: 'OfType', typeId: 'mover' } }
				]
			}),
			rig
		);
		expect(picked).toEqual([]);
	});
});

describe('the order the selection comes out in', () => {
	const everything = query({ clauses: [{ combine: 'Add', term: { kind: 'Named', text: 'front' } }] });

	/**
	 * The order is not decoration: an effect spreads across the selection in order,
	 * so this is what makes a chase run left to right rather than in patch order.
	 */
	it('sorts along an axis', () => {
		const left = evaluate({ ...everything, order: { kind: 'ByAxis', axis: 'x' } }, rig);
		expect(left).toEqual(['Front 1', 'Front 2', 'Front 3', 'Front 4']);

		const right = evaluate({ ...everything, order: { kind: 'ByAxis', axis: 'x', descending: true } }, rig);
		expect(right).toEqual(['Front 4', 'Front 3', 'Front 2', 'Front 1']);
	});

	/** Outwards from a point, which is what makes a centre-out chase possible. */
	it('sorts outwards from a point', () => {
		const out = evaluate(
			{ ...everything, order: { kind: 'ByDistance', from: { x: 0, y: 5, z: 2 } } },
			rig
		);
		expect(out).toEqual(['Front 2', 'Front 3', 'Front 1', 'Front 4']);
	});

	it('sorts by name', () => {
		const named = evaluate({ ...everything, order: { kind: 'ByName' } }, rig);
		expect(named).toEqual(['Front 1', 'Front 2', 'Front 3', 'Front 4']);
	});

	/**
	 * Two fixtures at the same point get a stable order rather than one that depends
	 * on how the rig happened to be listed — otherwise a chase across a symmetric rig
	 * would come out differently on two consoles.
	 */
	it('breaks a tie by name, so the answer does not depend on patch order', () => {
		const twins = [at('B', 0, 0, 0), at('A', 0, 0, 0)];
		const q = query({
			clauses: [{ combine: 'Add', term: { kind: 'Everything' } }],
			order: { kind: 'ByAxis', axis: 'x' }
		});
		expect(evaluate(q, twins)).toEqual(['A', 'B']);
		expect(evaluate(q, [...twins].reverse())).toEqual(['A', 'B']);
	});

	/** An unplaced fixture sorts to the end rather than to the origin, where it would
	 * sit in the middle of the rig pretending to be somewhere. */
	it('puts unplaced fixtures last in a geometric order', () => {
		const all = evaluate(
			query({
				clauses: [{ combine: 'Add', term: { kind: 'Everything' } }],
				order: { kind: 'ByAxis', axis: 'x' }
			}),
			rig
		);
		expect(all[all.length - 1]).toBe('In the case');
	});

	/**
	 * Manual keeps what the operator dragged into place, and puts anything the query
	 * has newly picked up on the end rather than reshuffling the lot.
	 */
	it('keeps a hand-made order and appends what is new', () => {
		const dragged = ['Front 3', 'Front 1'];
		const out = sortSelection(['Front 1', 'Front 2', 'Front 3'], NO_ORDER, rig, dragged);
		expect(out).toEqual(['Front 3', 'Front 1', 'Front 2']);
	});

	it('drops a hand-ordered fixture that the query no longer picks', () => {
		const out = sortSelection(['Front 1'], NO_ORDER, rig, ['Front 3', 'Front 1']);
		expect(out).toEqual(['Front 1']);
	});
});

describe('saying what a query selects', () => {
	it('reads as a sentence', () => {
		const q = query({
			clauses: [
				{ combine: 'Add', term: { kind: 'OfType', typeId: 'mover' } },
				{ combine: 'Keep', term: { kind: 'Box', from: { x: 0, y: 0, z: 0 }, to: { x: 1, y: 1, z: 1 } } },
				{ combine: 'Drop', term: { kind: 'Ids', ids: ['x'] } }
			]
		});
		const types = [aFixtureType({ id: 'mover', name: 'Mac Aura' })];
		expect(describeQuery(q, types)).toBe('every Mac Aura, of those, in a region, except 1 picked');
	});

	it('says so when there is nothing', () => {
		expect(describeQuery(EMPTY_QUERY)).toBe('nothing');
	});
});

/**
 * The corpus, from this side.
 *
 * `testdata/selection-queries.json` is read here and by
 * `crates/pult-schema/tests/selection_corpus.rs`. A query has to pick the same
 * fixtures in the same order in a browser as on a station — a group saved on one
 * console and resolved on another is the whole feature — and two evaluators is the
 * price of not putting a round trip inside a drag. This is how the price is paid.
 *
 * A case with no `previous` is a saved group: nothing outside the query holds an
 * order. A case whose `previous` is a list — an empty one included — is a live
 * selection handing over the order somebody dragged the panel into.
 */
describe('the corpus the station agrees with', () => {
	type Case = {
		name: string;
		query: SelectionQuery;
		previous?: string[];
		expected: string[];
	};
	const corpus: { rig: Fixture[]; cases: Case[] } = JSON.parse(
		readFileSync(new URL('../../../testdata/selection-queries.json', import.meta.url), 'utf8')
	);

	const named = (ids: string[]) =>
		ids.map((id) => corpus.rig.find((f) => f.id === id)?.name ?? id);

	for (const c of corpus.cases) {
		it(c.name, () => {
			const got = evaluate(c.query, corpus.rig, c.previous ?? null);
			expect(named(got)).toEqual(named(c.expected));
		});
	}

	it('is still worth reading', () => {
		// A corpus that quietly emptied itself would pass every test above.
		expect(corpus.rig.length).toBeGreaterThanOrEqual(5);
		expect(corpus.cases.length).toBeGreaterThanOrEqual(15);
		expect(corpus.rig.some((f) => f.position === null)).toBe(true);
	});
});
