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
        ///
        /// One path even when a gesture touched many, because a message naming
        /// twenty of them is not a message anybody reads. `changed` says how many
        /// there were.
        #[serde(skip_serializing_if = "Option::is_none")]
        undone: Option<crate::path::Path>,
        /// How many paths the gesture moved. One for an ordinary write.
        #[serde(default)]
        changed: u32,
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
    /// The other half of the clock exchange: what the station's show clock said when
    /// it answered, and the client's own stamp handed straight back.
    ClockSync {
        sent_at: f64,
        /// The station's show clock, in console unix milliseconds — the same clock
        /// every fade and every effect in the show is anchored in.
        station_ms: f64,
    },
    Pong,
}
