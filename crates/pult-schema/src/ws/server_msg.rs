use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::path::Path;

/// Messages sent from the backend to a frontend client.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "payload")]
pub enum ServerMessage {
    /// Response to a Get request.
    GetResult {
        path: Path,
        value: serde_json::Value,
        request_id: String,
    },
    /// Acknowledgement for a Set request.
    SetAck {
        request_id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Response to a Call request.
    CallResult {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// What an Undo actually took back, or nothing when there was nothing to take.
    UndoResult {
        request_id: String,
        /// A short account of what moved, for the toast: "took back Patch → Spot 3".
        #[serde(skip_serializing_if = "Option::is_none")]
        undone: Option<crate::path::Path>,
    },
    /// The recent history, newest first.
    HistoryResult {
        request_id: String,
        entries: Vec<crate::ws::HistoryEntry>,
    },
    /// Push notification for a subscribed path pattern.
    Update {
        path: Path,
        value: serde_json::Value,
    },
    /// Server-initiated error notification.
    Error {
        message: String,
    },
    Pong,
}
