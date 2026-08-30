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
    /// The cue currently being edited, if any.
    ///
    /// Editing is load-tweak-Update rather than live: the cue is read into the
    /// programmer, changed there, and written back on Update. This says which cue is
    /// waiting for that write, and it is SYNCED so a second console shows the same
    /// banner rather than quietly storing over the first one's work.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub editing_cue: Option<Uuid>,
}
