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
 * # Where the types are now
 *
 * In `pult-schema`, since saved groups made a query show data. This file used to
 * declare them and said that if that ever happened they would move and the backend
 * would get an evaluator beside this one. Both have.
 *
 * The evaluator here stayed, because a box or a cone dragged across the rig changes
 * the query every frame and cannot afford a round trip to the station. The two are
 * held to each other by `testdata/selection-queries.json`, which this file's test and
 * `crates/pult-schema/tests/selection_corpus.rs` both read.
 *
 * The *live* selection is still nobody's but this browser's, and still lives in a
 * store. What moved into the show is the saved kind.
 */

import type {
	CellSpan,
	Element,
	Fixture,
	FixtureType,
	SceneObject,
	SelectionClause,
	SelectionCombine,
	SelectionOrder,
	SelectionQuery,
	SelectionTerm,
	Vec3
} from './generated/index.js';
import { byId as objectsById, worldTransform } from './scene.js';

// ── What a query is ───────────────────────────────────────────────────────────

/**
 * The query types, under the names this side of the house has always called them.
 *
 * `Term` and `Order` are fine names in a file about selection and bad ones in a
 * directory of a hundred generated types, so the schema spells them out and the
 * panels go on saying `Term`.
 */
export type Term = SelectionTerm;
export type Combine = SelectionCombine;
export type Clause = SelectionClause;
export type Order = SelectionOrder;
export type { SelectionQuery };

/** The order a query has when nobody has put the fixtures in one. */
export const NO_ORDER: SelectionOrder = { kind: 'Manual', order: [] };

/** The query a fresh console has: nothing selected, in the order it was picked. */
export const EMPTY_QUERY: SelectionQuery = { clauses: [], order: NO_ORDER, cells: [] };

/** The query a plain click builds. Manual picking is a query like any other. */
export function idsQuery(ids: string[], order: Order = NO_ORDER, cells: CellSpan[] = []): SelectionQuery {
	return { clauses: [{ combine: 'Add', term: { kind: 'Ids', ids } }], order, cells };
}

// ── Cells ─────────────────────────────────────────────────────────────────────

export type { CellSpan, Element };

/**
 * The elements of a selection, in order: each fixture in the order the query put
 * them, and within a multi-cell fixture its cells in the order the spans list them.
 * Empty spans are the whole fixture; a fixture with no cells is in whole whatever
 * they say.
 *
 * `elements` in `group.rs`; `testdata/selection-queries.json` holds the two.
 */
export function elements(
	picked: string[],
	spans: CellSpan[],
	cellsOf: (fixtureId: string) => number
): Element[] {
	const out: Element[] = [];
	for (const fixture of picked) {
		const count = cellsOf(fixture);
		if (count === 0) {
			out.push({ fixture, cell: null });
			continue;
		}
		const seen = new Set<number>();
		const push = (cell: number) => {
			if (cell >= 1 && cell <= count && !seen.has(cell)) {
				seen.add(cell);
				out.push({ fixture, cell });
			}
		};
		if (spans.length === 0) {
			for (let cell = 1; cell <= count; cell++) push(cell);
			continue;
		}
		for (const span of spans) {
			if (span.from <= span.to) {
				for (let cell = span.from; cell <= Math.min(span.to, count); cell++) push(cell);
			} else {
				for (let cell = Math.min(span.from, count); cell >= span.to; cell--) push(cell);
			}
		}
	}
	return out;
}

/** `1-20`, `5`, `20-1`, `1-4, 9`: what an operator types for a cell sub-selection. */
export function parseCellSpans(text: string): CellSpan[] | null {
	const trimmed = text.trim();
	if (trimmed === '') return [];
	const spans: CellSpan[] = [];
	for (const part of trimmed.split(',')) {
		const m = part.trim().match(/^(\d+)(?:\s*-\s*(\d+))?$/);
		if (!m) return null;
		const from = Number(m[1]);
		const to = m[2] === undefined ? from : Number(m[2]);
		if (from < 1 || to < 1 || from > 65535 || to > 65535) return null;
		spans.push({ from, to });
	}
	return spans;
}

/** The spans as an operator would type them back. */
export function cellSpansText(spans: CellSpan[]): string {
	return spans.map((s) => (s.from === s.to ? `${s.from}` : `${s.from}-${s.to}`)).join(', ');
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
export function normalize(v: Vec3): Vec3 | null {
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
	const axis = normalize(direction);
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

/** Whether a chain of parents starting at `parent` reaches `object`, however deep. */
export function hangsOff(
	parent: string | null,
	object: string,
	objects: Map<string, SceneObject>
): boolean {
	for (let depth = 0; depth < 64 && parent; depth++) {
		if (parent === object) return true;
		parent = objects.get(parent)?.parent ?? null;
	}
	return false;
}

function matches(term: Term, fixture: Fixture, objects: Map<string, SceneObject>): boolean {
	switch (term.kind) {
		case 'Everything':
			return true;
		case 'Ids':
			return term.ids.includes(fixture.id);
		case 'OfType':
			return fixture.fixture_type_id === term.typeId;
		case 'Named':
			return fixture.name.toLowerCase().includes(term.text.trim().toLowerCase());
		case 'Hanging':
			return hangsOff(fixture.parent, term.object, objects);
		default:
			break;
	}

	// Everything below is about where a fixture is, and one that has never been
	// placed is not anywhere.
	if (!fixture.position) return false;
	const point = worldTransform(fixture.position, fixture.parent, objects).position;

	switch (term.kind) {
		case 'Sphere':
			return distance(point, term.center) <= term.radius;
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
 *
 * `previous` is the hand order this browser is holding outside the query. A live
 * selection has one; a saved group, resolved on a station that never saw the drag,
 * does not — so `null` means "use the order the query itself carries", and an empty
 * array means "there is a hand order and it is empty", which is not the same thing.
 */
export function evaluate(
	query: SelectionQuery,
	fixtures: Fixture[],
	previous: string[] | null = null,
	scene: SceneObject[] = []
): string[] {
	const objects = objectsById(scene);
	let picked: string[] = [];

	for (const { combine, term } of query.clauses) {
		const hits = fixtures.filter((f) => matches(term, f, objects)).map((f) => f.id);
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

	return sortSelection(picked, query.order, fixtures, previous, scene);
}

/** Put a set of ids into the order a query asks for. */
export function sortSelection(
	ids: string[],
	order: Order,
	fixtures: Fixture[],
	previous: string[] | null = null,
	scene: SceneObject[] = []
): string[] {
	const objects = objectsById(scene);
	const byId = new Map(fixtures.map((f) => [f.id, f]));

	if (order.kind === 'Manual') {
		// Whatever somebody dragged into place, with anything new on the end. The hand
		// order this browser is holding wins over the one the query carries, which is
		// what lets a group be recalled and then rearranged without saving.
		const hand = previous ?? order.order;
		const known = hand.filter((id) => ids.includes(id));
		const rest = ids.filter((id) => !hand.includes(id));
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
		const point = worldTransform(fixture.position, fixture.parent, objects).position;
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
export function describe(
	query: SelectionQuery,
	types: FixtureType[] = [],
	objects: SceneObject[] = []
): string {
	if (query.clauses.length === 0) return 'nothing';
	const typeName = (id: string) => types.find((t) => t.id === id)?.name ?? 'a type';
	const objectName = (id: string) => objects.find((o) => o.id === id)?.name ?? 'an object';

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
			case 'Hanging':
				return `${verb}hung off ${objectName(term.object)}`;
		}
	});
	return parts.join(', ');
}
