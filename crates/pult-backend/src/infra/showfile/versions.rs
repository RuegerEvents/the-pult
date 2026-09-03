//! Taking a version, and putting one back.
//!
//! A [`Version`] row says a version exists; this is what makes one. The row
//! replicates and the file does not, which is the whole shape of the thing:
//!
//! - **The snapshot is each station's own.** It is a copy of *this* station's
//!   `show.db` at that instant, so a station that joined the session afterwards
//!   never held that state and has nothing to copy. Which is why the LOCAL
//!   `versions_here` exists: the panel can say "not on this station" about a row
//!   only because the station publishes which files it actually has.
//! - **The snapshot contains its own row.** The checkpointer waits for the version's
//!   own write to be durable before copying, so restoring to a version and then
//!   looking at the list finds the version you restored to still in it. Getting this
//!   backwards would make every restore quietly forget the point it restored to.
//! - **The assets are shared.** They are files in `assets/` beside the database, so
//!   fifty versions of a show with a rig full of meshes hold one copy of each.
//!
//! `VACUUM INTO` rather than a file copy: it is safe on an open WAL database, needs
//! no transaction and no lock held across the copy, and what it writes is a
//! compacted database rather than a page-for-page image of one.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::bundle::Bundle;
use crate::engine::EngineHandle;

/// What the engine asks for after a `versions` row lands.
enum Job {
    /// Take a snapshot, once this receipt says the row is on the disk.
    Take { id: Uuid, when_durable: oneshot::Receiver<Result<(), String>> },
    /// A version was deleted — by an operator, by an undo, or by a peer.
    Forget { id: Uuid },
}

/// The handle the engine holds. Cheap to clone.
#[derive(Clone)]
pub struct Checkpointer(mpsc::Sender<Job>);

impl Checkpointer {
    /// Take a snapshot for this version once its own row is durable.
    ///
    /// Never awaited by the engine: a `VACUUM INTO` over a large show takes as long
    /// as writing the show does, and the actor holding still for it would stop
    /// playback.
    pub fn take(&self, id: Uuid, when_durable: oneshot::Receiver<Result<(), String>>) {
        let _ = self.0.try_send(Job::Take { id, when_durable });
    }

    /// The row has gone, so the file should go with it.
    pub fn forget(&self, id: Uuid) {
        let _ = self.0.try_send(Job::Forget { id });
    }
}

/// Start the checkpointer for one open show.
///
/// `read_pool` is the engine's own pool rather than the writer's: `VACUUM INTO` is a
/// read of the whole database, and putting it on the writer's single connection would
/// put it in front of every commit.
pub fn start(
    bundle: Bundle,
    read_pool: Arc<SqlitePool>,
    engine: EngineHandle,
) -> (Checkpointer, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<Job>(16);
    let task = tokio::spawn(async move {
        // What this station holds, said once at the start so a panel opened before
        // anybody saves anything shows the truth rather than nothing.
        publish_what_is_here(&engine, &bundle).await;

        while let Some(job) = rx.recv().await {
            match job {
                Job::Take { id, when_durable } => {
                    // The row first, so the snapshot contains it.
                    match when_durable.await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            warn!("[versions] {id} was not written, so nothing was copied: {e}");
                            continue;
                        }
                        Err(_) => {
                            warn!("[versions] the writer stopped before {id} landed");
                            continue;
                        }
                    }
                    match snapshot(&read_pool, &bundle, id).await {
                        Ok(bytes) => info!("[versions] saved {id} ({bytes} bytes)"),
                        Err(e) => {
                            warn!("[versions] could not save {id}: {e:#}");
                            continue;
                        }
                    }
                    publish_what_is_here(&engine, &bundle).await;
                    mirror_to_backup(&bundle, id);
                }
                Job::Forget { id } => {
                    let path = bundle.version_path(id);
                    if path.exists() {
                        if let Err(e) = std::fs::remove_file(&path) {
                            warn!("[versions] could not remove {}: {e}", path.display());
                        }
                    }
                    publish_what_is_here(&engine, &bundle).await;
                }
            }
        }
    });
    (Checkpointer(tx), task)
}

/// Copy the show as it stands into `versions/<id>.db`.
async fn snapshot(pool: &SqlitePool, bundle: &Bundle, id: Uuid) -> Result<u64> {
    let path = bundle.version_path(id);
    // A leftover from a run that was killed between the copy and the rename. `VACUUM
    // INTO` refuses to write a file that exists, which is the right refusal for
    // everything except this.
    let _ = std::fs::remove_file(&path);
    std::fs::create_dir_all(bundle.versions_dir())?;

    // The path is a uuid under a directory this console made, so nothing an operator
    // typed reaches this string.
    sqlx::query(&format!("VACUUM INTO '{}'", path.display()))
        .execute(pool)
        .await
        .with_context(|| format!("copying the show into {}", path.display()))?;
    Ok(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0))
}

/// Tell this station's browsers which versions it can actually restore.
///
/// LOCAL, because it is a fact about this machine's disk and not about the show. A
/// peer's row with no file here is not an error — it is a version taken before this
/// station was in the room.
async fn publish_what_is_here(engine: &EngineHandle, bundle: &Bundle) {
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment};

    let here = bundle.versions_here();
    let path = vec![PathSegment::Key(VERSIONS_HERE.into())];
    let value = serde_json::to_value(here).unwrap_or_default();
    if let Err(e) = engine.set(path, Lifecycle::Local, value).await {
        debug!("[versions] could not publish what is here: {e}");
    }
}

/// The LOCAL path carrying which snapshots this station holds.
pub const VERSIONS_HERE: &str = "versions_here";

/// Copy a snapshot, and anything it points at, somewhere else.
///
/// The `backup_dir` preference and nothing more: a second copy on another volume, so
/// a disk that dies between the get-in and the last night is not the whole season.
/// Failure is a warning in the system log and never an error — a backup that cannot
/// be written must not make Save fail.
///
/// The assets go too, because a snapshot without them is a rig with no drawings and
/// no fixture definitions in it. Only the ones the backup has not already got, which
/// is what makes the second version cost almost nothing.
fn mirror_to_backup(bundle: &Bundle, id: Uuid) {
    let Some(dir) = crate::infra::preferences::load().backup_dir else { return };
    let name = bundle.path().file_name().unwrap_or_default();
    let into = dir.join(name);

    let copy = || -> Result<()> {
        std::fs::create_dir_all(into.join("versions"))?;
        std::fs::create_dir_all(into.join("assets"))?;
        std::fs::copy(bundle.version_path(id), into.join("versions").join(format!("{id}.db")))?;
        std::fs::copy(bundle.path().join("bundle.toml"), into.join("bundle.toml"))?;
        for entry in std::fs::read_dir(bundle.assets_dir())?.flatten() {
            let there = into.join("assets").join(entry.file_name());
            if !there.exists() && entry.path().is_file() {
                std::fs::copy(entry.path(), there)?;
            }
        }
        Ok(())
    };
    match copy() {
        Ok(()) => debug!("[versions] {id} mirrored to {}", into.display()),
        Err(e) => warn!("[versions] could not mirror {id} to {}: {e}", into.display()),
    }
}

/// Put a snapshot back, as the show.
///
/// Only safe with the station down, which is why the console does it between stopping
/// one station and starting the next. The `-wal` and `-shm` go with the old file: a
/// journal belonging to a database that is no longer there is how a perfectly good
/// snapshot gets read as a corrupt show.
pub fn restore(bundle: &Bundle, version: Uuid) -> Result<()> {
    let from = bundle.version_path(version);
    if !from.exists() {
        anyhow::bail!(
            "this station does not hold {version}. A version's row travels and its \
             snapshot does not — open it on the station that took it."
        );
    }
    let to = bundle.db_path();
    for journal in ["show.db-wal", "show.db-shm"] {
        let _ = std::fs::remove_file(bundle.path().join(journal));
    }
    std::fs::copy(&from, &to)
        .with_context(|| format!("putting {} back as the show", from.display()))?;
    info!("[versions] restored {version}");
    Ok(())
}

/// Rows for the snapshots this station holds that the show has forgotten.
///
/// One case makes this necessary rather than tidy. Restoring puts back a database
/// that was written *before* the "Before restoring…" version was taken, so that
/// version's own row is not in it — and the file would sit in `versions/` for ever
/// with nothing naming it, which is exactly the safety net an operator reaches for
/// when the restore turns out to have been a mistake.
///
/// A snapshot carries its own `versions` table, so the row can be read back out of
/// the file it belongs to. Re-created unattributed: whoever took it is recorded in
/// the row, but this station is not in a position to claim the act was theirs.
pub async fn reconcile(bundle: &Bundle, engine: &EngineHandle) {
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment, types::Version};

    let known: Vec<Version> = engine
        .get(vec![PathSegment::Key("versions".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    for id in bundle.versions_here() {
        if known.iter().any(|version| version.id == id) {
            continue;
        }
        let Some(row) = row_inside(&bundle.version_path(id), id).await else {
            warn!("[versions] {id} is here but says nothing about itself; leaving it alone");
            continue;
        };
        info!("[versions] {id} is on this disk and not in the show; putting the row back");
        let path = vec![
            PathSegment::Key("versions".into()),
            PathSegment::Key("__create".into()),
        ];
        let value = serde_json::to_value(&row).unwrap_or_default();
        if let Err(e) = engine.set(path, Lifecycle::Persisted, value).await {
            warn!("[versions] could not put {id} back: {e}");
        }
    }
}

/// A snapshot's own row about itself.
async fn row_inside(file: &Path, id: Uuid) -> Option<pult_schema::types::Version> {
    use std::str::FromStr;

    if !file.exists() {
        return None;
    }
    let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
        "sqlite:{}?mode=ro",
        file.display()
    ))
    .ok()?
    .read_only(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .ok()?;
    let rows: Vec<pult_schema::types::Version> =
        pult_schema::db::get_all(&pool).await.ok().unwrap_or_default();
    pool.close().await;
    rows.into_iter().find(|row| row.id == id)
}
