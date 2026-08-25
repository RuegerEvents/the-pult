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
