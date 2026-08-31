use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::path::{Path, PathPattern};

/// Messages sent from a frontend client to the backend.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "payload")]
pub enum ClientMessage {
    /// Subscribe to path-pattern updates. Server will push updates matching this pattern.
    Subscribe {
        pattern: PathPattern,
    },
    /// Cancel a subscription.
    Unsubscribe {
        pattern: PathPattern,
    },
    /// Request a full snapshot of the value at this path.
    Get {
        path: Path,
        request_id: String,
    },
    /// Set the value at a path.
    Set {
        path: Path,
        value: serde_json::Value,
        request_id: String,
        /// The one act this write is part of, where it is part of one.
        ///
        /// A drag is hundreds of these and should cost one Ctrl-Z, and only the
        /// client knows where the drag started. Defaulted, so a write that stands
        /// alone says nothing and an older client keeps working.
        #[serde(default)]
        #[ts(optional)]
        gesture: Option<uuid::Uuid>,
    },
    /// Say who is at this client.
    ///
    /// Sent once when a browser knows its user and again if that changes. Every
    /// write on this socket is attributed to them afterwards, which is what lets
    /// undo be per-person rather than per-connection — one operator's desktop and
    /// tablet identify as the same user and share one history.
    Identify {
        user_id: Option<uuid::Uuid>,
    },
    /// Take back this client's user's last change, or put back their last undo.
    Undo {
        redo: bool,
        request_id: String,
    },
    /// Ask for the recent history, for the panel that shows who changed what.
    History {
        limit: u32,
        request_id: String,
    },
    /// Invoke a named method (e.g. "sequences.goNext", "session.create").
    Call {
        method: String,
        args: serde_json::Value,
        request_id: String,
    },
    Ping,
}
