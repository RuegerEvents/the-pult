//! Folders for the pools: a timeline, a sequence, a preset, a group — every kind a
//! pool shows — can sit in one.
//!
//! **A folder is a row, and an object names its folder.** A PERSISTED `folders` row
//! carries a name, a parent and the kind of object it holds, and each pooled object
//! carries `folder: Option<Uuid>`. So renaming a folder or moving it under another is
//! one write, however much is in it, and folders nest. A path written on each object
//! instead ("Act 1/Scares") would have made a rename a rewrite of every member.
//!
//! **A folder holds one kind.** `kind` is the registry name the browser's pools use
//! (`"timeline"`, `"preset"`), so the timelines' folders are not offered to a preset.
//!
//! **Deleting a folder deletes nothing in it.** Its objects and its folders move up to
//! its parent, in the same gesture as the delete, so one Ctrl-Z puts the folder back
//! with everything in it. A folder is somewhere to put things, and throwing a scare
//! away because its folder was tidied up is not a thing a console should do.
//!
//! A folder an object names that is not there (deleted on a peer, or never received)
//! is no folder: the object shows at the top level. Nothing cascades.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// One folder in one pool.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "folders")]
pub struct Folder {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The folder this one is in, or `None` at the top of its pool.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub parent: Option<Uuid>,
    /// Which kind of object it holds, by the pools' name for the kind.
    #[pult(lifecycle = PERSISTED)]
    pub kind: String,
}
