//! One change, as a panel wants to read it.
//!
//! Deliberately not `Operation`. That type carries a vector clock, a node id and a
//! sequence number, which are how replication works and none of which a person
//! reading "who turned the house lights off" needs. Sending it whole would also put
//! the previous value of every write on the wire, which for a stage plan is a few
//! megabytes nobody asked for.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::path::Path;

/// One entry in the history panel.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HistoryEntry {
    pub id: Uuid,
    /// Who did it, or nothing for the console's own doing — a fade advancing, a
    /// follow cue firing. Those appear in no history because nobody did them.
    pub user_id: Option<Uuid>,
    pub path: Path,
    /// ISO 8601, so the panel can say "a moment ago" without a second field.
    pub at: String,
    /// The operation this one reversed, if it was an undo or a redo.
    pub undoes: Option<Uuid>,
    /// Whether this is still something its author could take back.
    ///
    /// Worked out on the backend, where the whole log is, rather than in each
    /// browser — the rule involves walking a chain of reversals and two clients
    /// deciding it separately is two chances to disagree.
    pub undoable: bool,
}
