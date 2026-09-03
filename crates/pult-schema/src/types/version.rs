//! A saved version of a show: what *Save* means when nothing needs saving.
//!
//! Every PERSISTED write is already on the disk the moment it is acknowledged, so
//! there is no unsaved work to lose and nothing for Save to flush. What an operator
//! wants Save for is the other thing it has always also meant: **a point to come
//! back to**. Take a version before the designer changes their mind about the whole
//! second act, and the previous act is still there afterwards.
//!
//! # A row that replicates, and a file that does not
//!
//! The row is PERSISTED, so every station in the session knows the version exists,
//! who took it and when — and undoing the save undoes it everywhere, which is what
//! Ctrl-Z after an accidental Save should do.
//!
//! The *snapshot* is each station's own file. It has to be: a snapshot is a copy of
//! this station's `show.db` at that instant, and a station that joined the session
//! after the version was taken never held that state and has nothing to copy. So a
//! console can honestly say "not on this station" for a peer's version, and the row
//! is what lets it say anything at all.
//!
//! # Why a whole file rather than a rewind through the oplog
//!
//! The oplog is pruned on its own retention (an hour by default), so yesterday is
//! not reachable through it and never will be. A version has to be a copy. The
//! `clock` is kept all the same: it is what a future *diff between two versions*
//! would anchor on, and it costs a JSON column.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{events::operation::VectorClock, PultSchema};

/// One saved point in a show's history.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "versions")]
pub struct Version {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// What the operator called it, where they called it anything. A quick Save has
    /// no name, and the panel shows the time instead — naming every checkpoint is
    /// work nobody does, and a console that demanded one would be a console nobody
    /// saved on.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub name: Option<String>,
    #[pult(lifecycle = PERSISTED)]
    pub created_at: DateTime<Utc>,
    /// Who took it. `None` for one the console took by itself — an autosave, or the
    /// one taken before a restore.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub user_id: Option<Uuid>,
    /// Whether the console took this one rather than a person.
    ///
    /// Kept apart because the two are pruned differently and read differently: an
    /// operator's saves are theirs to keep, and the automatic ones are a rolling
    /// window the console trims to `autosave_keep`. The panel dims them for the same
    /// reason it dims the console's own writes in the history.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub automatic: bool,
    /// Where the show's clock stood when this was taken.
    ///
    /// Nothing reads it yet. It is here because a version is the only record of a
    /// past state that survives the oplog's retention, and a diff between two of
    /// them has to be able to say which came first on which station — which a
    /// timestamp across machines with unsynchronised clocks cannot.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub clock: VectorClock,
}

impl Version {
    /// What to show for a version nobody named.
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) if !name.trim().is_empty() => name.clone(),
            _ => self.created_at.format("%-d %b %H:%M").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    fn a_version(name: Option<&str>) -> Version {
        Version {
            id: Uuid::new_v4(),
            name: name.map(str::to_string),
            created_at: at("2026-09-03T19:04:00Z"),
            user_id: None,
            automatic: false,
            clock: VectorClock::default(),
        }
    }

    #[test]
    fn a_version_nobody_named_is_shown_by_when_it_was_taken() {
        // Naming every checkpoint is work nobody does, and a console that demanded
        // one would be a console nobody saved on.
        assert_eq!(a_version(None).label(), "3 Sep 19:04");
        assert_eq!(a_version(Some("  ")).label(), "3 Sep 19:04");
        assert_eq!(a_version(Some("Before act two")).label(), "Before act two");
    }
}
