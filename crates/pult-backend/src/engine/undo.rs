//! Taking a change back.
//!
//! # An undo is a write
//!
//! There is no undo stack anywhere. An undo reads the oplog, works out the inverse of
//! the operation it is reversing, and writes it — logging a new operation that points
//! at the one it undid. That has three consequences worth the design:
//!
//! - it replicates to peers like any other write, so a second console is not left
//!   showing a value that has been taken back;
//! - a second client of the same user sees it, which is the whole reason undo is
//!   per-user rather than per-browser;
//! - redo is undoing an undo, so one mechanism covers both and there is no second
//!   list to keep in step with the first.
//!
//! # A gesture, not a write
//!
//! What an operator did and what the log recorded are not the same size. Dragging a
//! fader across a selection of twenty is one act and several thousand writes, and a
//! Ctrl-Z that took back the last frame of it would be useless. So a client stamps
//! every write between a pointer going down and coming up with one gesture id, and
//! everything here reasons in gestures: an ordinary write is a gesture of one, keyed
//! by its own id, so there is no second code path for the common case.
//!
//! Reversing a gesture writes one operation per *path* it touched, back to what was
//! there before its first write at that path. Not one per operation — that would be
//! four hundred rows to undo four hundred rows, and the log would grow faster the
//! more you took back.
//!
//! # What is undoable
//!
//! Somebody has to have asked for it, something has to have been captured to put
//! back, and it has to be an edit rather than a button press. `Operation::is_undoable`
//! is where those three live. The engine's own writes — a fade advancing, a station
//! publishing its memory use — have no author and no meaning as an undo.
//!
//! Commands are excluded deliberately. `goNext` has no inverse worth the name, and an
//! operator who pressed Ctrl-Z expecting to take back an edit would not thank a
//! console that moved the lights instead. Going back a cue is a different gesture and
//! has a different name.

use std::collections::{HashMap, HashSet};

use pult_schema::{
    events::operation::Operation,
    path::{Path, PathSegment},
};
use uuid::Uuid;

/// What to write to reverse an operation.
#[derive(Debug, Clone, PartialEq)]
pub struct Inverse {
    pub path: Path,
    pub value: serde_json::Value,
}

/// Which of the three shapes of write this is.
///
/// The inverse of a field write is the old value at the same path. The inverse of a
/// create is a delete and the inverse of a delete is a create, and neither of those
/// is "the same path with the previous value" — which is why this is a function
/// rather than one line at the call site.
fn collection_and_action(path: &Path) -> Option<(&str, &str)> {
    match path.as_slice() {
        [PathSegment::Key(table), PathSegment::Key(action)] if action == "__create" => {
            Some((table, "__create"))
        }
        [PathSegment::Key(table), _, PathSegment::Key(action)] if action == "__delete" => {
            Some((table, "__delete"))
        }
        _ => None,
    }
}

/// The id an entity value carries, for turning a create into a delete.
fn entity_id(value: &serde_json::Value) -> Option<Uuid> {
    value.get("id")?.as_str().and_then(|id| Uuid::parse_str(id).ok())
}

/// How to undo one operation, or `None` if it cannot be undone.
///
/// `None` is a real answer rather than a failure: an operation from before undo
/// existed captured nothing to go back to, and a create whose value carries no id
/// leaves nothing to delete. Both should refuse rather than guess.
pub fn inverse_of(op: &Operation) -> Option<Inverse> {
    if !op.is_undoable() {
        return None;
    }
    let previous = op.previous.clone()?;

    match collection_and_action(&op.path) {
        // Something was added. Take it away again, by the id it was given.
        Some((table, "__create")) => Some(Inverse {
            path: vec![
                PathSegment::Key(table.into()),
                PathSegment::Id(entity_id(&op.value)?),
                PathSegment::Key("__delete".into()),
            ],
            value: serde_json::Value::Null,
        }),
        // Something was removed. Put it back whole — which is why a delete captures
        // the entity rather than a flag: nothing else knows what was in it.
        Some((table, "__delete")) => {
            if previous.is_null() {
                return None;
            }
            Some(Inverse {
                path: vec![
                    PathSegment::Key(table.into()),
                    PathSegment::Key("__create".into()),
                ],
                value: previous,
            })
        }
        // An ordinary write. The old value, back where it came from.
        _ => Some(Inverse { path: op.path.clone(), value: previous }),
    }
}

/// The id of the one act an operation was part of.
///
/// A write with no gesture is an act of one, keyed by itself. That is the whole
/// trick of this module: everything below talks about gestures, and the ordinary
/// single write is a gesture with one operation in it rather than a second case to
/// get wrong.
pub fn gesture_key(op: &Operation) -> Uuid {
    op.gesture.unwrap_or(op.id)
}

/// The next thing `user` would take back, newest write first.
///
/// See [`in_effect`] and [`depth`] for the two ideas this rests on. Undo looks for
/// the most recent gesture still in effect at an even depth: a change, or a redo,
/// both of which are things currently *applied* that pressing Ctrl-Z should remove.
///
/// Only this user's operations are considered, and only this user's reversals count
/// against them: two operators each have their own history, so one pressing Ctrl-Z
/// can never take back work the other is in the middle of.
pub fn next_to_undo<'a>(log: &'a [Operation], user: Uuid) -> Vec<&'a Operation> {
    run_at_parity(log, user, 0)
}

/// The next thing `user` would put back: their most recent gesture, if it was an
/// undo.
///
/// Two rules, and both matter. Odd depth is what tells an undo from a redo —
/// without it a redo would itself be redoable, and redoing twice would quietly take
/// the change away again, an undo wearing the wrong label.
///
/// And the user has to still be *in* an undo run: the last thing they did must have
/// been an undo or a redo, not a fresh change. Every editor discards the redo branch
/// the moment you do something new, for a good reason — putting an old value back on
/// top of work done since is not what anybody means by redo.
pub fn next_to_redo<'a>(log: &'a [Operation], user: Uuid) -> Vec<&'a Operation> {
    let Some(newest) = log.iter().find(|op| op.user_id == Some(user) && op.is_undoable()) else {
        return Vec::new();
    };
    if newest.undoes.is_none() {
        return Vec::new();
    }
    run_at_parity(log, user, 1)
}

/// Every operation of one gesture, newest first, as `oplog::recent_by_people`
/// returns them.
fn ops_of<'a>(log: &'a [Operation], user: Uuid, key: Uuid) -> Vec<&'a Operation> {
    log.iter()
        .filter(|op| op.user_id == Some(user) && op.is_undoable() && gesture_key(op) == key)
        .collect()
}

fn run_at_parity<'a>(log: &'a [Operation], user: Uuid, parity: usize) -> Vec<&'a Operation> {
    let index = Index::of(log, user);
    let mut seen: HashSet<Uuid> = HashSet::new();
    for op in log.iter().filter(|op| op.user_id == Some(user) && op.is_undoable()) {
        let key = gesture_key(op);
        // The log is newest first, so the first sighting of a gesture is the one
        // that decides where it sits in the running order.
        if !seen.insert(key) {
            continue;
        }
        if index.depth(key) % 2 == parity && index.in_effect(key) {
            return ops_of(log, user, key);
        }
    }
    Vec::new()
}

/// How to reverse a whole gesture: one write per thing it touched, newest first.
///
/// Four rules, and each is there for a case that would otherwise be wrong.
///
/// **One write per target, taken from the *earliest* operation on it.** A drag
/// writes the same path four hundred times, and what "before the drag" means is what
/// was there before the first of them. Inverting all four hundred would arrive at
/// the same value by luck of ordering and put four hundred rows in the log doing it.
///
/// **A create is keyed by what it made, not by where it was written.** Two fixtures
/// added in one gesture are two writes to the same `fixtures/__create` path.
/// Collapsing those together would delete the first and leave the second standing,
/// which is a half-undone gesture and the worst of both.
///
/// **A field written under something the gesture created is dropped.** The entity is
/// going away; putting a value back into it first is a row in the log that describes
/// a state nobody will ever see.
///
/// **Newest first.** A gesture that made two fixtures and then moved one inverts to
/// a move and two deletes, and deleting first would leave the move writing into a
/// hole. Reversing the order unpicks a gesture the way it was tied.
pub fn inverses_of_run(run: &[&Operation]) -> Vec<Inverse> {
    let made = made_by(run);

    // `run` arrives newest first, so walking it and keeping the *last* sighting of
    // each target leaves the earliest operation on it, in newest-target-first order.
    let mut earliest: Vec<(Target, &Operation)> = Vec::new();
    for op in run {
        if covered_by_a_delete(op, &made) {
            continue;
        }
        let target = target_of(op);
        match earliest.iter_mut().find(|(seen, _)| *seen == target) {
            Some(seen) => seen.1 = op,
            None => earliest.push((target, op)),
        }
    }
    earliest.iter().filter_map(|(_, op)| inverse_of(op)).collect()
}

/// What an operation's inverse acts on, which is not always its path.
#[derive(PartialEq)]
enum Target<'a> {
    /// An entity this gesture brought into being. Keyed by the entity, because every
    /// create in a collection is written to the same path.
    Made(Uuid),
    /// Anything else: the path is the target.
    At(&'a Path),
}

fn target_of(op: &Operation) -> Target<'_> {
    match collection_and_action(&op.path) {
        Some((_, "__create")) => match entity_id(&op.value) {
            Some(id) => Target::Made(id),
            // A create with no id cannot be inverted anyway; `inverse_of` will drop
            // it. Keyed by its path so it does not swallow a sibling on the way out.
            None => Target::At(&op.path),
        },
        _ => Target::At(&op.path),
    }
}

/// The entities this gesture created.
fn made_by(run: &[&Operation]) -> Vec<Uuid> {
    run.iter()
        .filter(|op| matches!(collection_and_action(&op.path), Some((_, "__create"))))
        .filter_map(|op| entity_id(&op.value))
        .collect()
}

/// Whether an operation writes into something the same gesture created, and so is
/// about to be deleted out from under any value put back into it.
fn covered_by_a_delete(op: &Operation, made: &[Uuid]) -> bool {
    if collection_and_action(&op.path).is_some() {
        return false;
    }
    matches!(op.path.get(1), Some(PathSegment::Id(id)) if made.contains(id))
}

/// One user's log, arranged so the two questions below are cheap and terminate.
///
/// Both are recursive over chains of reversals, and both are asked once per gesture
/// while scanning a five-hundred row window. Walking the log for each would be
/// quadratic in the window and exponential in the length of an undo run; two maps
/// built once are neither.
struct Index {
    /// What each gesture reverses, if anything. Every operation of a gesture carries
    /// the same `undoes`, so any one of them answers for the whole gesture.
    reverses: HashMap<Uuid, Option<Uuid>>,
    /// Which gestures reverse this one, deduplicated. A gesture that touched three
    /// paths writes three reversing operations and they must not be walked thrice.
    reversed_by: HashMap<Uuid, Vec<Uuid>>,
}

impl Index {
    fn of(log: &[Operation], user: Uuid) -> Self {
        let mut reverses: HashMap<Uuid, Option<Uuid>> = HashMap::new();
        let mut reversed_by: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for op in log.iter().filter(|op| op.user_id == Some(user) && op.is_undoable()) {
            let key = gesture_key(op);
            reverses.entry(key).or_insert(op.undoes);
            if let Some(target) = op.undoes {
                let reversers = reversed_by.entry(target).or_default();
                if !reversers.contains(&key) {
                    reversers.push(key);
                }
            }
        }
        Index { reverses, reversed_by }
    }

    /// Whether a gesture is currently applied.
    ///
    /// A gesture is reversed when something points at it that is *itself* still in
    /// effect — so an undo that has since been redone no longer hides what it undid.
    /// Flat set membership is not enough: it would say a change stayed undone after
    /// it had been put back, and the second undo of a run would find nothing to do.
    fn in_effect(&self, key: Uuid) -> bool {
        !self
            .reversed_by
            .get(&key)
            .is_some_and(|reversers| reversers.iter().any(|&r| self.in_effect(r)))
    }

    /// How far along a chain of reversals a gesture sits.
    ///
    /// A change is 0, an undo of it 1, a redo of that 2. Parity is what separates
    /// undo from redo, and it is why one field on `Operation` can carry both: what a
    /// reversal *means* is decided by what it points at, not by a flag saying which
    /// button was pressed.
    ///
    /// Bounded rather than followed to the end: `undoes` always points backwards in
    /// time so a cycle cannot arise, but a log this long-lived should not be one
    /// malformed row away from a runaway loop.
    fn depth(&self, key: Uuid) -> usize {
        let mut steps = 0;
        let mut here = key;
        while let Some(Some(target)) = self.reverses.get(&here).copied() {
            steps += 1;
            if steps > 64 {
                break;
            }
            here = target;
        }
        steps
    }
}

#[cfg(test)]
mod tests;
