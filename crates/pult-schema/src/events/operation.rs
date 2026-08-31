use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{lifecycle::Lifecycle, path::Path, path::PathSegment};

// Ord is how concurrent writes are broken apart: every node picks the same winner
// because every node compares the same two ids.
//
// TS because a station id reaches the frontend now that an output can name the one
// that sends it. Serde flattens the newtype, so it is a plain uuid string on the
// wire and in TypeScript.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ts_rs::TS,
)]
#[ts(export)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A Lamport-style vector clock for causal ordering of distributed operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorClock(pub HashMap<NodeId, u64>);

impl VectorClock {
    pub fn increment(&mut self, node: NodeId) -> u64 {
        let counter = self.0.entry(node).or_insert(0);
        *counter += 1;
        *counter
    }

    pub fn merge(&mut self, other: &VectorClock) {
        for (node, &counter) in &other.0 {
            let entry = self.0.entry(*node).or_insert(0);
            if counter > *entry {
                *entry = counter;
            }
        }
    }

    pub fn happens_before(&self, other: &VectorClock) -> bool {
        let mut at_least_one_less = false;
        for (node, &other_counter) in &other.0 {
            let self_counter = self.0.get(node).copied().unwrap_or(0);
            if self_counter > other_counter {
                return false;
            }
            if self_counter < other_counter {
                at_least_one_less = true;
            }
        }
        for (node, &self_counter) in &self.0 {
            if !other.0.contains_key(node) && self_counter > 0 {
                return false;
            }
        }
        at_least_one_less
    }
}

/// A single mutation wrapped with distributed metadata.
///
/// The last three fields are what makes the oplog an undo history as well as a
/// replication log. Undo is not a separate stack kept beside this: an undo *is* a
/// write, logged like any other, which means it replicates to peers for free, a
/// second client of the same user sees it, and the history panel is a view of one
/// list rather than a reconciliation of two.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: Uuid,
    pub node_id: NodeId,
    /// Monotonically increasing sequence number on the originating node.
    pub seq: u64,
    pub clock: VectorClock,
    pub lifecycle: Lifecycle,
    pub path: Path,
    pub value: serde_json::Value,
    pub timestamp: DateTime<Utc>,
    /// Who asked for this, where anybody did.
    ///
    /// `None` for the engine's own writes — a fade advancing, a follow cue firing,
    /// a station publishing its memory usage. Nobody pressed anything, so there is
    /// nobody to attribute it to and nothing to undo.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// What was at this path before, so the write can be taken back.
    ///
    /// Captured at write time because the oplog is otherwise a list of destinations
    /// with no record of where anything came from — replaying it forwards works and
    /// running it backwards does not. `Some(Null)` and `None` are different: the
    /// first means the path was empty and undo should empty it again, the second
    /// means nothing was captured and this operation cannot be undone.
    #[serde(default)]
    pub previous: Option<serde_json::Value>,
    /// The gesture this one reverses, if it is an undo or a redo.
    ///
    /// Undo and redo are the same mechanism: each writes a value and points at what
    /// it undid. Redo is undoing an undo, which is why one field covers both and why
    /// the stack does not need to exist anywhere else.
    ///
    /// It names a *gesture*, and an ordinary write is a gesture keyed by its own id
    /// — so a row written before gestures existed still points at exactly what it
    /// always did.
    #[serde(default)]
    pub undoes: Option<Uuid>,
    /// The one thing an operator did that this write was part of.
    ///
    /// A drag across a fader is a few hundred pointer events and, over a selection of
    /// twenty, a few thousand writes — but it is one act, and Ctrl-Z should take back
    /// the act rather than the last frame of it. Only the client knows where a
    /// gesture starts and stops, so it says: every write between a pointer going down
    /// and coming up carries the same id.
    ///
    /// `None` means a write that stands alone, which is most of them, and is what an
    /// older client sends.
    #[serde(default)]
    pub gesture: Option<Uuid>,
}

/// What makes a write undoable: who asked for it, what it replaced, and which
/// gesture it belonged to.
///
/// Together rather than as three parameters because they are always decided
/// together and never used apart, and because four trailing `Option`s in a row is a
/// signature where two of them get swapped and nothing complains.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Authorship {
    /// Who asked. `None` for the engine's own writes — a fade advancing, a station
    /// publishing its memory use — which nobody did and nobody can take back.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// What was at the path before.
    #[serde(default)]
    pub previous: Option<serde_json::Value>,
    /// The gesture this write was part of, if it was part of one.
    #[serde(default)]
    pub gesture: Option<Uuid>,
    /// The gesture this write reverses, if it is an undo or a redo.
    #[serde(default)]
    pub undoes: Option<Uuid>,
}

impl Authorship {
    /// Nobody: the engine's own writes.
    pub fn none() -> Self {
        Self::default()
    }

    /// A write somebody asked for, over what was there.
    pub fn by(user_id: Option<Uuid>, previous: Option<serde_json::Value>) -> Self {
        Self { user_id, previous, gesture: None, undoes: None }
    }

    /// The same, as part of one gesture.
    pub fn during(mut self, gesture: Option<Uuid>) -> Self {
        self.gesture = gesture;
        self
    }

    /// The same, taking back a gesture.
    pub fn reversing(mut self, undone: Uuid) -> Self {
        self.undoes = Some(undone);
        self
    }
}

impl Operation {
    /// Whether this operation is one a person could ask to take back.
    ///
    /// Three things have to hold. Somebody has to have asked for it — the engine's
    /// own writes have no author and no meaning as an undo. Something has to have
    /// been captured to put back. And it has to be a value write rather than a
    /// command: `goNext` has no inverse worth the name, and an operator who pressed
    /// Ctrl-Z expecting to take back an edit would not thank a console that moved
    /// the lights instead. Going back a cue is a different gesture with a different
    /// name.
    pub fn is_undoable(&self) -> bool {
        self.user_id.is_some() && self.previous.is_some() && !is_command_path(&self.path)
    }
}

/// Whether a path names a registered command rather than a field.
///
/// Commands are dispatched by the last segment matching a registration, which is
/// what this asks. Keeping it here rather than in the engine means the oplog can
/// answer "was that an edit or a button press" without the engine's help.
pub fn is_command_path(path: &Path) -> bool {
    let Some(PathSegment::Key(last)) = path.last() else { return false };
    crate::commands::registered_command(last).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_clock_happens_before() {
        let a = NodeId(Uuid::new_v4());
        let b = NodeId(Uuid::new_v4());

        let mut clock1 = VectorClock::default();
        clock1.increment(a);

        let mut clock2 = VectorClock::default();
        clock2.increment(a);
        clock2.increment(b);

        assert!(clock1.happens_before(&clock2));
        assert!(!clock2.happens_before(&clock1));
    }
}
