//! Putting an imported file into the show.
//!
//! Shared by every importer, because the rules about writing are the same whatever
//! was read and getting them subtly different per format is how a half-imported show
//! happens.
//!
//! **Validate everything, then write.** A plan is built by a pure function from the
//! bytes and what the show already holds; nothing is stored until that plan exists.
//! An import that was going to fail therefore fails before it has left anything
//! behind, which is what makes the recovery below acceptable.
//!
//! **One gesture, one Ctrl-Z.** Every write of an import carries one fresh gesture id,
//! so an MVR of two thousand fixtures is one undo — not two thousand.
//!
//! **A write that fails takes the rest back.** The gesture is the operator's most
//! recent, so undoing it is exactly the right thing to undo. That only holds because
//! of the first rule: a failure here is a bug or a disk, not a rejected file.

use pult_schema::lifecycle::Lifecycle;
use pult_schema::path::{Path, PathSegment};
use serde::Serialize;
use uuid::Uuid;

use crate::engine::EngineHandle;
use crate::infra::assets::AssetStore;

/// Everything an import wants to happen, worked out before any of it does.
#[derive(Default)]
pub struct ImportPlan {
    /// Bytes to put in the asset store, as `(mime, bytes)`. Stored first, and
    /// idempotently: an asset that arrives twice is stored once, and one left behind
    /// by a failed import is content-addressed and harmless.
    pub assets: Vec<(String, Vec<u8>)>,
    /// The writes, in the order they must happen.
    ///
    /// Order is the plan's to decide, not this module's, but it is always the same
    /// shape: a thing before the thing that points at it — fixture types before
    /// fixtures, a truss before what hangs off it.
    pub writes: Vec<(Path, Lifecycle, serde_json::Value)>,
    pub report: ImportReport,
}

/// What an import did, in the words the operator sees.
#[derive(Default, Debug, Clone, Serialize)]
pub struct ImportReport {
    pub created: usize,
    pub updated: usize,
    /// Rows an earlier import of this file put in the show that this one does not
    /// mention. **Listed, never deleted**: somebody may have moved that light on
    /// purpose, and an importer that tidied up would take the rig with it.
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl ImportPlan {
    /// A path that creates a new row in a collection.
    pub fn create(collection: &str) -> Path {
        vec![PathSegment::Key(collection.into()), PathSegment::Key("__create".into())]
    }

    /// A path that replaces an existing row.
    pub fn replace(collection: &str, id: Uuid) -> Path {
        vec![PathSegment::Key(collection.into()), PathSegment::Id(id)]
    }

    /// Add a write, counting it as a create or an update.
    pub fn write(&mut self, collection: &str, existing: Option<Uuid>, value: serde_json::Value) {
        match existing {
            Some(id) => {
                self.report.updated += 1;
                self.writes.push((Self::replace(collection, id), Lifecycle::Persisted, value));
            }
            None => {
                self.report.created += 1;
                self.writes.push((Self::create(collection), Lifecycle::Persisted, value));
            }
        }
    }
}

/// What went wrong applying a plan, in words a route can hand to a browser.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("storing {0}: {1}")]
    Asset(String, String),
    #[error("writing {0}: {1}")]
    Write(String, String),
}

/// Store the assets, then make the writes, all under one gesture.
pub async fn apply(
    plan: ImportPlan,
    assets: &AssetStore,
    engine: &EngineHandle,
    user_id: Uuid,
) -> Result<ImportReport, ApplyError> {
    // Assets first, and outside the gesture: they are content-addressed, so storing
    // one twice is a no-op and one left behind by a failed import is bytes nothing
    // points at rather than a row nothing explains.
    for (mime, bytes) in &plan.assets {
        assets
            .put(mime, bytes)
            .await
            .map_err(|error| ApplyError::Asset(mime.clone(), error.to_string()))?;
    }

    let gesture = Uuid::new_v4();
    for (path, lifecycle, value) in plan.writes {
        let named = describe(&path);
        if let Err(error) = engine.set_as(user_id, Some(gesture), path, lifecycle, value).await {
            // Take back what landed. The gesture is this operator's most recent, so
            // this undoes exactly the import and nothing beside it — which holds only
            // because the plan was validated before any of it was written.
            engine.undo(user_id, false).await;
            return Err(ApplyError::Write(named, error.to_string()));
        }
    }

    Ok(plan.report)
}

/// A path in the words an error message should use.
fn describe(path: &Path) -> String {
    path.iter()
        .map(|segment| match segment {
            PathSegment::Key(key) => key.clone(),
            PathSegment::Id(id) => id.to_string(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("/")
}
