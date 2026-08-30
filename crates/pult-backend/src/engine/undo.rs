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

use std::collections::HashMap;

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

/// The next thing `user` would take back.
///
/// See [`in_effect`] and [`depth`] for the two ideas this rests on. Undo looks for
/// the most recent operation still in effect at an even depth: a change, or a redo,
/// both of which are things currently *applied* that pressing Ctrl-Z should remove.
///
/// Only this user's operations are considered, and only this user's reversals count
/// against them: two operators each have their own history, so one pressing Ctrl-Z
/// can never take back work the other is in the middle of.
pub fn next_to_undo<'a>(log: &'a [Operation], user: Uuid) -> Option<&'a Operation> {
    next_at_parity(log, user, 0)
}

/// The next thing `user` would put back: their most recent operation, if it was an
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
pub fn next_to_redo<'a>(log: &'a [Operation], user: Uuid) -> Option<&'a Operation> {
    let newest = log.iter().find(|op| op.user_id == Some(user) && op.is_undoable())?;
    if newest.undoes.is_none() {
        return None;
    }
    next_at_parity(log, user, 1)
}

fn next_at_parity<'a>(log: &'a [Operation], user: Uuid, parity: usize) -> Option<&'a Operation> {
    let by_id: HashMap<Uuid, &Operation> = log.iter().map(|op| (op.id, op)).collect();
    log.iter().find(|op| {
        op.user_id == Some(user)
            && op.is_undoable()
            && in_effect(op, log, user)
            && depth(op, &by_id) % 2 == parity
    })
}

/// Whether an operation is currently applied.
///
/// An operation is reversed when something points at it that is *itself* still in
/// effect — so an undo that has since been redone no longer hides what it undid.
/// Flat set membership is not enough: it would say a change stayed undone after it
/// had been put back, and the second undo of a run would find nothing to do.
fn in_effect(op: &Operation, log: &[Operation], user: Uuid) -> bool {
    !log.iter().any(|other| {
        other.user_id == Some(user) && other.undoes == Some(op.id) && in_effect(other, log, user)
    })
}

/// How far along a chain of reversals an operation sits.
///
/// A change is 0, an undo of it 1, a redo of that 2. Parity is what separates undo
/// from redo, and it is why one field on `Operation` can carry both: what a
/// reversal *means* is decided by what it points at, not by a flag saying which
/// button was pressed.
///
/// Bounded rather than recursive to the end: `undoes` always points backwards in
/// time so a cycle cannot arise, but a log this long-lived should not be one
/// malformed row away from a stack overflow.
fn depth(op: &Operation, by_id: &HashMap<Uuid, &Operation>) -> usize {
    let mut steps = 0;
    let mut here = op;
    while let Some(target) = here.undoes.and_then(|id| by_id.get(&id)) {
        steps += 1;
        if steps > 64 {
            break;
        }
        here = target;
    }
    steps
}

#[cfg(test)]
mod tests;
