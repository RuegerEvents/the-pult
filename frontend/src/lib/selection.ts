/**
 * Selection as a question about the rig, rather than a list of answers.
 *
 * The spec is explicit about this and about why: a selection should be *generated*
 * from the rig by geometric functions and re-evaluated as the rig changes, "useful
 * for festivals, changing fixtures". A list of ids is a photograph of a rig that has
 * since been rebuilt. "The four movers on the downstage truss" is still true after
 * somebody adds a fifth, and a list of ids is not.
 *
 * # How a query is built
 *
 * A list of clauses, read left to right, each either adding fixtures, narrowing to
 * them, or removing them. That is how an operator actually builds a selection —
 * "all the movers, of those the downstage ones, but not the broken one" — and it
 * avoids a boolean tree nobody wants to type. A tree can come later if a query ever
 * needs one; nothing here forecloses it.
 *
 * # Order is part of the selection
 *
 * An effect spreads across the selection *in order*, so the order is not decoration:
 * it is what makes a chase run left to right rather than in patch order. A query
 * carries how to sort, and `Manual` is the escape hatch for an operator who has
 * dragged the panel into the order they want.
 *
 * # Why this is not in the schema
 *
 * Selection is the operator's, not the show's — `CLAUDE.md` puts frontend-only state
 * in a store, and two people at two consoles plainly hold different fixtures. If
 * saved groups ever become show data, the types move to `pult-schema` then and the
 * backend gets an evaluator beside this one. Until something needs that, one
 * implementation is better than two that can disagree.
 */

import type { Fixture, FixtureType, Vec3 } from './generated/index.js';
import { splitPosition } from './stage.js';

// ── What a query is ───────────────────────────────────────────────────────────

/**
 * One test a fixture either passes or fails.
 *
 * Every geometric term reads a position, so a fixture that has never been placed
 * fails all of them. That is the honest answer — a light nobody has told the console
 * about cannot be "downstage" — and it is why `Everything` and `OfType` exist: they
 * are how you reach an unplaced rig at all.
 */
export type Term =
	| { kind: 'Everything' }
	/** A literal list. What a click and a shift-click build, and how a manual pick lives in the same shape as everything else. */
	| { kind: 'Ids'; ids: string[] }
	| { kind: 'OfType'; typeId: string }
	/** Case-insensitive substring of the fixture's name. */
	| { kind: 'Named'; text: string }
	/** Within `radius` metres of a point. */
	| { kind: 'Sphere'; centre: Vec3; radius: number }
	/** Inside an axis-aligned region. The two corners may be given either way round. */
	| { kind: 'Box'; from: Vec3; to: Vec3 }
	/**
	 * The spec's radial selection: a cone from a point, opening along a direction.
	 *
	 * `angleDeg` is the half-angle — the angle from the axis to the edge — because
	 * that is the number a beam angle is quoted as and the one an operator has in
	 * their head. `reach` caps how far it goes, so a narrow cone does not select the
	 * whole stage behind the fixtures you meant.
	 */
	| { kind: 'Cone'; from: Vec3; direction: Vec3; angleDeg: number; reach: number };

/** What a clause does to the running set. Read left to right. */
export type Combine = 'Add' | 'Keep' | 'Drop';

export type Clause = { combine: Combine; term: Term };

/**
 * How the result is ordered.
 *
 * `Manual` keeps whatever order the ids were already in, for an operator who has
 * dragged the panel into the order they want; anything the query then adds goes on
 * the end. The rest are the orders a chase is usually asked to run in.
 */
export type Order =
	| { kind: 'Manual' }
	| { kind: 'ByName' }
	/** Along an axis. `x` is stage left to right, `z` is upstage to down. */
	| { kind: 'ByAxis'; axis: 'x' | 'y' | 'z'; descending?: boolean }
	/** Outwards from a point, which is what makes a centre-out chase possible. */
	| { kind: 'ByDistance'; from: Vec3 };

export type SelectionQuery = { clauses: Clause[]; order: Order };

/** The query a fresh console has: nothing selected, in the order it was picked. */
export const EMPTY_QUERY: SelectionQuery = { clauses: [], order: { kind: 'Manual' } };

/** The query a plain click builds. Manual picking is a query like any other. */
export function idsQuery(ids: string[], order: Order = { kind: 'Manual' }): SelectionQuery {
	return { clauses: [{ combine: 'Add', term: { kind: 'Ids', ids } }], order };
}

/** Whether this query is just a hand-picked list, which the panel says differently. */
export function isManualList(query: SelectionQuery): boolean {
	return query.clauses.every((c) => c.combine === 'Add' && c.term.kind === 'Ids');
}

// ── Geometry ──────────────────────────────────────────────────────────────────

const sub = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x - b.x, y: a.y - b.y, z: a.z - b.z });
const dot = (a: Vec3, b: Vec3): number => a.x * b.x + a.y * b.y + a.z * b.z;
const length = (v: Vec3): number => Math.sqrt(dot(v, v));

/** A unit vector, or null for one with no direction to speak of. */
export function normalise(v: Vec3): Vec3 | null {
	const l = length(v);
	return l > 1e-9 ? { x: v.x / l, y: v.y / l, z: v.z / l } : null;
}

export const distance = (a: Vec3, b: Vec3): number => length(sub(a, b));

/**
 * Whether a point is inside a cone.
 *
 * The angle between the axis and the point, compared against the half-angle. A point
 * exactly at the apex is inside: a cone drawn from a fixture should include that
 * fixture rather than excluding it on a technicality.
 */
export function inCone(
	point: Vec3,
	from: Vec3,
	direction: Vec3,
	angleDeg: number,
	reach: number
): boolean {
	const axis = normalise(direction);
	if (!axis) return false;
	const offset = sub(point, from);
	const along = length(offset);
	if (along > reach) return false;
	if (along < 1e-9) return true;
	const cos = dot(offset, axis) / along;
	// Clamped because floating point can put a dot product a hair outside [-1, 1],
	// and `Math.acos` answers NaN rather than 0 when it does.
	return Math.acos(Math.min(1, Math.max(-1, cos))) <= (angleDeg * Math.PI) / 180;
}

/** Whether a point is inside a box, whichever way round the corners were given. */
export function inBox(point: Vec3, from: Vec3, to: Vec3): boolean {
	const within = (v: number, a: number, b: number) => v >= Math.min(a, b) && v <= Math.max(a, b);
	return (
		within(point.x, from.x, to.x) &&
		within(point.y, from.y, to.y) &&
		within(point.z, from.z, to.z)
	);
}

// ── Evaluating ────────────────────────────────────────────────────────────────

function matches(term: Term, fixture: Fixture): boolean {
	switch (term.kind) {
		case 'Everything':
			return true;
		case 'Ids':
			return term.ids.includes(fixture.id);
		case 'OfType':
			return fixture.fixture_type_id === term.typeId;
		case 'Named':
			return fixture.name.toLowerCase().includes(term.text.trim().toLowerCase());
		default:
			break;
	}

	// Everything below is about where a fixture is, and one that has never been
	// placed is not anywhere.
	if (!fixture.position) return false;
	const { point } = splitPosition(fixture.position);

	switch (term.kind) {
		case 'Sphere':
			return distance(point, term.centre) <= term.radius;
		case 'Box':
			return inBox(point, term.from, term.to);
		case 'Cone':
			return inCone(point, term.from, term.direction, term.angleDeg, term.reach);
	}
}

/**
 * The fixtures a query picks out, in the order it asks for.
 *
 * Pure, and given the whole rig every time: that is what "re-evaluated as the rig
 * changes" means in practice — nothing is cached, so a fixture patched a moment ago
 * is in the answer without anything having to invalidate anything.
 */
export function evaluate(
	query: SelectionQuery,
	fixtures: Fixture[],
	previous: string[] = []
): string[] {
	let picked: string[] = [];

	for (const { combine, term } of query.clauses) {
		const hits = fixtures.filter((f) => matches(term, f)).map((f) => f.id);
		if (combine === 'Add') {
			// Order of arrival is kept for `Manual`, so adding twice does not move a
			// fixture to the end of the list.
			const seen = new Set(picked);
			picked = [...picked, ...hits.filter((id) => !seen.has(id))];
		} else if (combine === 'Keep') {
			const keep = new Set(hits);
			picked = picked.filter((id) => keep.has(id));
		} else {
			const drop = new Set(hits);
			picked = picked.filter((id) => !drop.has(id));
		}
	}

	return sortSelection(picked, query.order, fixtures, previous);
}

/** Put a set of ids into the order a query asks for. */
export function sortSelection(
	ids: string[],
	order: Order,
	fixtures: Fixture[],
	previous: string[] = []
): string[] {
	const byId = new Map(fixtures.map((f) => [f.id, f]));

	if (order.kind === 'Manual') {
		// Whatever the operator dragged into place, with anything new on the end.
		const known = previous.filter((id) => ids.includes(id));
		const rest = ids.filter((id) => !previous.includes(id));
		return [...known, ...rest];
	}

	const key = (id: string): [number, string] => {
		const fixture = byId.get(id);
		if (!fixture) return [Number.POSITIVE_INFINITY, id];
		if (order.kind === 'ByName') return [0, fixture.name.toLowerCase()];
		// An unplaced fixture sorts to the end of a geometric order rather than to
		// the origin, where it would sit in the middle of the rig pretending to be
		// somewhere.
		if (!fixture.position) return [Number.POSITIVE_INFINITY, fixture.name];
		const { point } = splitPosition(fixture.position);
		if (order.kind === 'ByAxis') return [point[order.axis], fixture.name];
		return [distance(point, order.from), fixture.name];
	};

	const sorted = [...ids].sort((a, b) => {
		const [na, sa] = key(a);
		const [nb, sb] = key(b);
		// Name breaks a tie, so two fixtures at the same point have a stable order
		// rather than one that depends on how the rig happened to be listed.
		if (na !== nb) return na - nb;
		return String(sa).localeCompare(String(sb));
	});

	return order.kind === 'ByAxis' && order.descending ? sorted.reverse() : sorted;
}

/** A short account of what a query selects, for the panel to show. */
export function describe(query: SelectionQuery, types: FixtureType[] = []): string {
	if (query.clauses.length === 0) return 'nothing';
	const typeName = (id: string) => types.find((t) => t.id === id)?.name ?? 'a type';

	const parts = query.clauses.map(({ combine, term }, i) => {
		const verb = i === 0 ? '' : combine === 'Add' ? 'plus ' : combine === 'Keep' ? 'of those, ' : 'except ';
		switch (term.kind) {
			case 'Everything':
				return `${verb}everything`;
			case 'Ids':
				return `${verb}${term.ids.length} picked`;
			case 'OfType':
				return `${verb}every ${typeName(term.typeId)}`;
			case 'Named':
				return `${verb}named “${term.text}”`;
			case 'Sphere':
				return `${verb}within ${term.radius} m`;
			case 'Box':
				return `${verb}in a region`;
			case 'Cone':
				return `${verb}in a ${term.angleDeg * 2}° beam`;
		}
	});
	return parts.join(', ');
}
