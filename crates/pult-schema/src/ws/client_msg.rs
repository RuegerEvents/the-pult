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
    },
    /// Invoke a named method (e.g. "sequences.goNext", "session.create").
    Call {
        method: String,
        args: serde_json::Value,
        request_id: String,
    },
    Ping,
}
