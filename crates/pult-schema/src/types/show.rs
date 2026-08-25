use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// Top-level show metadata.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "show", singleton)]
pub struct Show {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub created_at: DateTime<Utc>,
    /// Whether the show engine is actively running.
    #[pult(lifecycle = SYNCED)]
    pub is_running: bool,
    /// ID of the currently active sequence, if any.
    #[pult(lifecycle = SYNCED)]
    pub active_sequence: Option<Uuid>,
}
