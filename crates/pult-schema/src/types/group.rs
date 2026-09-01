//! Saved groups: a name attached to a question about the rig.
//!
//! Task 30 made a selection a *query* rather than a list of ids — "every mover on the
//! downstage truss" stays true after somebody patches a fifth one, and a list of ids
//! does not. It then had nowhere to keep one: the query types lived in
//! `frontend/src/lib/selection.ts` with a comment saying that if saved groups ever
//! became show data, they would move here. They have, so they did.
//!
//! # Why the query and not the fixtures
//!
//! A group stores the question. Resolving it reads the rig as it is now, so a fixture
//! patched this afternoon is in this morning's group without anybody re-saving
//! anything, and a fixture that was deleted is simply not in the answer.
//!
//! # Why `Manual` carries its order
//!
//! The frontend keeps the order an operator dragged the panel into in a store beside
//! the query, because an in-flight drag is not a fact about the show. A *group* has no
//! store behind it: it is read on a station that never saw the drag. So
//! [`SelectionOrder::Manual`] carries the ids in the order somebody put them in, and a
//! saved group resolves the same way everywhere by construction rather than by
//! everyone remembering to freeze it.
//!
//! # Why this is evaluated twice
//!
//! [`evaluate`] exists here and again in `frontend/src/lib/selection.ts`. Dragging a
//! box or a cone across the rig changes the query every frame, so evaluation is on the
//! interaction path and cannot be a round trip. The two are held together by
//! `testdata/selection-queries.json`, which both test suites read — the same
//! arrangement `model/effects.rs` and `frontend/src/lib/effects.ts` already have.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fixture::{Fixture, Vec3};
use crate::PultSchema;

// ── What a query is ───────────────────────────────────────────────────────────

/// One test a fixture either passes or fails.
///
/// Every geometric term reads a position, so a fixture that has never been placed
/// fails all of them. That is the honest answer — a light nobody has told the console
/// about cannot be "downstage" — and it is why `Everything` and `OfType` exist: they
/// are how you reach an unplaced rig at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum SelectionTerm {
    Everything,
    /// A literal list. What a click and a shift-click build, and how a manual pick
    /// lives in the same shape as everything else.
    Ids { ids: Vec<Uuid> },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    OfType { type_id: Uuid },
    /// Case-insensitive substring of the fixture's name.
    Named { text: String },
    /// Within `radius` metres of a point.
    Sphere { centre: Vec3, radius: f32 },
    /// Inside an axis-aligned region. The two corners may be given either way round.
    Box { from: Vec3, to: Vec3 },
    /// The spec's radial selection: a cone from a point, opening along a direction.
    ///
    /// `angle_deg` is the half-angle — the angle from the axis to the edge — because
    /// that is the number a beam angle is quoted as and the one an operator has in
    /// their head. `reach` caps how far it goes, so a narrow cone does not select the
    /// whole stage behind the fixtures you meant.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Cone { from: Vec3, direction: Vec3, angle_deg: f32, reach: f32 },
}

/// What a clause does to the running set. Read left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SelectionCombine {
    Add,
    Keep,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SelectionClause {
    pub combine: SelectionCombine,
    pub term: SelectionTerm,
}

/// Which way an axial order runs. `x` is stage left to right, `z` is upstage to down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum SelectionAxis {
    X,
    Y,
    Z,
}

/// How the result is ordered.
///
/// An effect spreads across the selection *in order*, so the order is not decoration:
/// it is what makes a chase run left to right rather than in patch order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum SelectionOrder {
    /// Whatever order somebody put them in, with anything the query newly matches
    /// going on the end.
    ///
    /// The list is part of the query because a saved group is resolved by stations
    /// that never saw the drag that produced it. A live selection may leave it empty
    /// and hand `evaluate` the operator's hand order instead.
    Manual {
        #[serde(default)]
        order: Vec<Uuid>,
    },
    ByName,
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ByAxis {
        axis: SelectionAxis,
        /// Optional on the wire so `{ kind: 'ByAxis', axis: 'x' }` stays a whole
        /// order — most of the ones an operator picks from a menu are.
        #[serde(default)]
        #[ts(optional = nullable)]
        descending: Option<bool>,
    },
    /// Outwards from a point, which is what makes a centre-out chase possible.
    ByDistance { from: Vec3 },
}

/// A list of clauses, read left to right, each either adding fixtures, narrowing to
/// them, or removing them. That is how an operator actually builds a selection —
/// "all the movers, of those the downstage ones, but not the broken one" — and it
/// avoids a boolean tree nobody wants to type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SelectionQuery {
    pub clauses: Vec<SelectionClause>,
    pub order: SelectionOrder,
}

impl SelectionQuery {
    /// The query a fresh console has: nothing selected, in the order it was picked.
    pub fn empty() -> Self {
        SelectionQuery { clauses: Vec::new(), order: SelectionOrder::Manual { order: Vec::new() } }
    }
}

// ── The entity ────────────────────────────────────────────────────────────────

/// A saved selection: a name and the question it stands for.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "groups")]
pub struct Group {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// What the group asks of the rig. Not the fixtures it currently picks out —
    /// those are worked out on demand, which is the whole point.
    #[pult(lifecycle = PERSISTED)]
    pub query: SelectionQuery,
}

// ── Geometry ──────────────────────────────────────────────────────────────────

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn length(v: Vec3) -> f32 {
    dot(v, v).sqrt()
}

/// A unit vector, or `None` for one with no direction to speak of.
pub fn normalise(v: Vec3) -> Option<Vec3> {
    let l = length(v);
    (l > 1e-9).then(|| Vec3 { x: v.x / l, y: v.y / l, z: v.z / l })
}

pub fn distance(a: Vec3, b: Vec3) -> f32 {
    length(sub(a, b))
}

/// Whether a point is inside a cone.
///
/// The angle between the axis and the point, compared against the half-angle. A point
/// exactly at the apex is inside: a cone drawn from a fixture should include that
/// fixture rather than excluding it on a technicality.
pub fn in_cone(point: Vec3, from: Vec3, direction: Vec3, angle_deg: f32, reach: f32) -> bool {
    let Some(axis) = normalise(direction) else { return false };
    let offset = sub(point, from);
    let along = length(offset);
    if along > reach {
        return false;
    }
    if along < 1e-9 {
        return true;
    }
    // Clamped because floating point can put a dot product a hair outside [-1, 1],
    // and `acos` answers NaN rather than 0 when it does.
    let cos = (dot(offset, axis) / along).clamp(-1.0, 1.0);
    cos.acos() <= angle_deg.to_radians()
}

/// Whether a point is inside a box, whichever way round the corners were given.
pub fn in_box(point: Vec3, from: Vec3, to: Vec3) -> bool {
    let within = |v: f32, a: f32, b: f32| v >= a.min(b) && v <= a.max(b);
    within(point.x, from.x, to.x)
        && within(point.y, from.y, to.y)
        && within(point.z, from.z, to.z)
}

// ── Evaluating ────────────────────────────────────────────────────────────────

fn matches(term: &SelectionTerm, fixture: &Fixture) -> bool {
    match term {
        SelectionTerm::Everything => return true,
        SelectionTerm::Ids { ids } => return ids.contains(&fixture.id),
        SelectionTerm::OfType { type_id } => return fixture.fixture_type_id == *type_id,
        SelectionTerm::Named { text } => {
            return fixture.name.to_lowercase().contains(text.trim().to_lowercase().as_str())
        }
        _ => {}
    }

    // Everything below is about where a fixture is, and one that has never been
    // placed is not anywhere.
    let Some(position) = fixture.position else { return false };
    let point = position.position();

    match term {
        SelectionTerm::Sphere { centre, radius } => distance(point, *centre) <= *radius,
        SelectionTerm::Box { from, to } => in_box(point, *from, *to),
        SelectionTerm::Cone { from, direction, angle_deg, reach } => {
            in_cone(point, *from, *direction, *angle_deg, *reach)
        }
        _ => unreachable!("handled above"),
    }
}

/// The fixtures a query picks out, in the order it asks for.
///
/// Pure, and given the whole rig every time: that is what "re-evaluated as the rig
/// changes" means in practice — nothing is cached, so a fixture patched a moment ago
/// is in the answer without anything having to invalidate anything.
///
/// `previous` is a hand order held outside the query, which a live selection has and a
/// saved group does not. When it is `None`, a `Manual` order uses the list it carries.
pub fn evaluate(
    query: &SelectionQuery,
    fixtures: &[Fixture],
    previous: Option<&[Uuid]>,
) -> Vec<Uuid> {
    let mut picked: Vec<Uuid> = Vec::new();

    for SelectionClause { combine, term } in &query.clauses {
        let hits: Vec<Uuid> =
            fixtures.iter().filter(|f| matches(term, f)).map(|f| f.id).collect();
        match combine {
            SelectionCombine::Add => {
                // Order of arrival is kept for `Manual`, so adding twice does not move
                // a fixture to the end of the list.
                for id in hits {
                    if !picked.contains(&id) {
                        picked.push(id);
                    }
                }
            }
            SelectionCombine::Keep => picked.retain(|id| hits.contains(id)),
            SelectionCombine::Drop => picked.retain(|id| !hits.contains(id)),
        }
    }

    sort_selection(&picked, &query.order, fixtures, previous)
}

/// Put a set of ids into the order a query asks for.
pub fn sort_selection(
    ids: &[Uuid],
    order: &SelectionOrder,
    fixtures: &[Fixture],
    previous: Option<&[Uuid]>,
) -> Vec<Uuid> {
    if let SelectionOrder::Manual { order: stored } = order {
        // Whatever somebody dragged into place, with anything new on the end. The
        // hand order given by a live selection wins over the one the query carries.
        let hand: &[Uuid] = previous.unwrap_or(stored);
        let known = hand.iter().filter(|id| ids.contains(id)).copied();
        let rest = ids.iter().filter(|id| !hand.contains(id)).copied();
        return known.chain(rest).collect();
    }

    // An unplaced fixture sorts to the end of a geometric order rather than to the
    // origin, where it would sit in the middle of the rig pretending to be somewhere.
    let key = |id: &Uuid| -> (f32, String) {
        let Some(fixture) = fixtures.iter().find(|f| f.id == *id) else {
            return (f32::INFINITY, id.to_string());
        };
        if matches!(order, SelectionOrder::ByName) {
            return (0.0, fixture.name.to_lowercase());
        }
        let Some(position) = fixture.position else {
            return (f32::INFINITY, fixture.name.clone());
        };
        let point = position.position();
        match order {
            SelectionOrder::ByAxis { axis, .. } => {
                let v = match axis {
                    SelectionAxis::X => point.x,
                    SelectionAxis::Y => point.y,
                    SelectionAxis::Z => point.z,
                };
                (v, fixture.name.clone())
            }
            SelectionOrder::ByDistance { from } => (distance(point, *from), fixture.name.clone()),
            _ => (0.0, fixture.name.clone()),
        }
    };

    let mut sorted: Vec<Uuid> = ids.to_vec();
    sorted.sort_by(|a, b| {
        let (na, sa) = key(a);
        let (nb, sb) = key(b);
        // Name breaks a tie, so two fixtures at the same point have a stable order
        // rather than one that depends on how the rig happened to be listed.
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal).then_with(|| sa.cmp(&sb))
    });

    if matches!(order, SelectionOrder::ByAxis { descending: Some(true), .. }) {
        sorted.reverse();
    }
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes here have to deserialize from the JSON the frontend has been
    /// writing since task 30, because that is the wire this type now defines.
    #[test]
    fn a_query_written_in_the_frontends_shape_loads() {
        let json = serde_json::json!({
            "clauses": [
                { "combine": "Add", "term": { "kind": "Everything" } },
                { "combine": "Keep", "term": { "kind": "OfType", "typeId": "1c8a5b1e-0000-4000-8000-000000000001" } },
                { "combine": "Drop", "term": { "kind": "Named", "text": "broken" } }
            ],
            "order": { "kind": "ByAxis", "axis": "x" }
        });
        let query: SelectionQuery = serde_json::from_value(json).expect("frontend shape");
        assert_eq!(query.clauses.len(), 3);
        assert!(matches!(
            query.order,
            SelectionOrder::ByAxis { axis: SelectionAxis::X, descending: None }
        ));
    }

    #[test]
    fn every_term_kind_round_trips() {
        let v = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        let terms = vec![
            SelectionTerm::Everything,
            SelectionTerm::Ids { ids: vec![Uuid::nil()] },
            SelectionTerm::OfType { type_id: Uuid::nil() },
            SelectionTerm::Named { text: "x".into() },
            SelectionTerm::Sphere { centre: v, radius: 2.0 },
            SelectionTerm::Box { from: v, to: v },
            SelectionTerm::Cone { from: v, direction: v, angle_deg: 15.0, reach: 10.0 },
        ];
        for term in terms {
            let text = serde_json::to_string(&term).unwrap();
            let back: SelectionTerm = serde_json::from_str(&text).unwrap();
            assert_eq!(term, back, "{text}");
            assert!(text.contains("\"kind\""), "{text} is not tagged by kind");
        }
    }

    #[test]
    fn an_empty_manual_order_keeps_its_list() {
        let order = SelectionOrder::Manual { order: Vec::new() };
        assert_eq!(serde_json::to_string(&order).unwrap(), r#"{"kind":"Manual","order":[]}"#);
        // And an older shape with no list at all still loads, so a query written by
        // hand or by a plugin need not know about it.
        let back: SelectionOrder = serde_json::from_str(r#"{"kind":"Manual"}"#).unwrap();
        assert_eq!(back, order);
    }

    #[test]
    fn descending_stays_absent_when_it_is_not_set() {
        let order = SelectionOrder::ByAxis { axis: SelectionAxis::Z, descending: None };
        let text = serde_json::to_string(&order).unwrap();
        let back: SelectionOrder = serde_json::from_str(&text).unwrap();
        assert_eq!(back, order);
        // And the shape the panels have been writing, with the field left out.
        let bare: SelectionOrder =
            serde_json::from_str(r#"{"kind":"ByAxis","axis":"z"}"#).unwrap();
        assert_eq!(bare, order);
    }
}
