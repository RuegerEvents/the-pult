//! Bringing an older showfile up to this build's shape, when somebody asks.
//!
//! Until generation 9 a showfile from another build was refused by name and that was
//! the whole policy: nobody was carrying a season's work in one. A haunt running its
//! second weekend is exactly that, so from 9 on every bump of
//! [`super::SCHEMA_GENERATION`] ships a **step** — one Rust function over one SQLite
//! transaction, taking the file from generation `n` to `n + 1` — and opening an older
//! file walks it up the steps. Rust rather than SQL files, because most of what has
//! ever changed is the shape *inside* a JSON column, which SQL cannot rewrite.
//!
//! Four rules, each the user's call.
//!
//! **Nothing is upgraded unless a person asked.** An upgrade is a one-way door — an
//! older build cannot open the result — so [`super::open`] refuses an upgradable file
//! with [`NeedsUpgrade`], and only [`Upgrade::InPlace`] walks it. A station that opens
//! one by itself (a reboot reopening the last show) comes up with no show open, and the
//! welcome screen offers the upgrade.
//!
//! **A version is taken first.** `VACUUM INTO versions/<id>.db` at the old generation,
//! and a row naming it, so the show as it was is in the Versions list — and restoring
//! that version walks the copy up the same steps.
//!
//! **History starts at the upgrade.** The oplog's rows name paths and carry values in
//! the old shape, and an undo replaying one would write it straight back. So the floor
//! is raised through every node's last operation and the rows go, exactly as a prune
//! does: the clock survives in the floor, and a peer that has not caught up is sent a
//! snapshot rather than the old operations.
//!
//! **Generations before [`FIRST_UPGRADABLE`] stay refused.** A step is a promise about
//! a shape, and nobody holds a generation-8 file worth keeping one for.

use anyhow::{Context, Result};
use sqlx::{Connection, SqliteConnection, SqlitePool};
use tracing::info;
use uuid::Uuid;

use super::SCHEMA_GENERATION;

/// The oldest generation a step can be taken from.
pub const FIRST_UPGRADABLE: i64 = 9;

/// The generations a step starts from, in order. A bump of `SCHEMA_GENERATION`
/// without a step here fails `every_generation_since_the_first_has_a_step`.
pub const STEPS: &[i64] = &[9];

/// A showfile that could be upgraded, opened by a caller that did not say to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "this showfile is generation {from} and this build reads {to}. It can be upgraded — \
     open it from the welcome screen and say so — but an older build cannot open it afterwards"
)]
pub struct NeedsUpgrade {
    pub from: i64,
    pub to: i64,
}

/// What opening a file older than this build should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upgrade {
    /// Refuse with [`NeedsUpgrade`].
    No,
    /// A show somebody asked to upgrade: take a version, then walk it up.
    InPlace,
    /// A throwaway copy — a version being read for a restore. Walk it up, and take no
    /// version of a file that is about to be deleted.
    Scratch,
}

/// The generation this build reads.
pub const CURRENT: i64 = SCHEMA_GENERATION;

/// The generation of a showfile on the disk, read without opening it for writing —
/// or `None` where there is no file or it cannot be read.
pub async fn generation_of(db: &std::path::Path) -> Option<i64> {
    use std::str::FromStr;
    if !db.exists() {
        return None;
    }
    let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=ro", db.display()))
        .ok()?
        .read_only(true);
    let mut conn = SqliteConnection::connect_with(&opts).await.ok()?;
    let generation = sqlx::query_scalar::<_, i64>("PRAGMA user_version").fetch_one(&mut conn).await.ok();
    let _ = conn.close().await;
    generation
}

/// Which show a showfile on the disk is, read the way [`generation_of`] reads.
///
/// Every id its `show` table holds, not the first: a follower's file keeps its own
/// original row beside the leader's show it was handed on joining (the HHG audio Mac's
/// held two), and waiting on only one of them waits on a show nobody runs.
pub async fn show_ids_of(db: &std::path::Path) -> Vec<Uuid> {
    use std::str::FromStr;
    let Ok(opts) = sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=ro", db.display())) else {
        return Vec::new();
    };
    let Ok(mut conn) = SqliteConnection::connect_with(&opts.read_only(true)).await else { return Vec::new() };
    let ids = sqlx::query_scalar::<_, String>("SELECT id FROM show").fetch_all(&mut conn).await.unwrap_or_default();
    let _ = conn.close().await;
    ids.into_iter().filter_map(|id| id.trim_matches('"').parse().ok()).collect()
}

/// Whether a file of `generation` can be walked up to this build.
pub fn upgradable(generation: i64) -> bool {
    (FIRST_UPGRADABLE..SCHEMA_GENERATION).contains(&generation)
}

/// Walk an open showfile from `from` up to this build's generation.
///
/// `db` is the file's path, for the version taken first; `None` takes none.
pub(super) async fn walk(pool: &SqlitePool, from: i64, version_beside: Option<&std::path::Path>) -> Result<()> {
    let version = match version_beside {
        Some(db) => Some(take_a_version_first(pool, db).await?),
        None => None,
    };

    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;
    for generation in from..SCHEMA_GENERATION {
        info!("[showfile] upgrading from generation {generation} to {}", generation + 1);
        step(&mut tx, generation)
            .await
            .with_context(|| format!("upgrading from generation {generation}"))?;
    }
    start_history_here(&mut tx).await?;
    if let Some(version) = version {
        record_the_version(&mut tx, version, from).await?;
    }
    sqlx::query(&format!("PRAGMA user_version = {SCHEMA_GENERATION}")).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// One step, from `generation` to the next.
async fn step(conn: &mut SqliteConnection, generation: i64) -> Result<()> {
    match generation {
        9 => to_10(conn).await,
        other => anyhow::bail!("there is no step from generation {other}"),
    }
}

/// 9 → 10: the arrangements of the panels were `layouts`; they are `workspaces`, and
/// `layouts` is now the 2D drawings. The rows are the same shape under the new name.
async fn to_10(conn: &mut SqliteConnection) -> Result<()> {
    let has = |name: &'static str| {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(name)
    };
    if has("layouts").fetch_one(&mut *conn).await? == 0 {
        return Ok(());
    }
    // A file this build touched before the stamp was read could already hold an empty
    // `workspaces`; the rows are what matter.
    if has("workspaces").fetch_one(&mut *conn).await? > 0 {
        sqlx::query("DROP TABLE workspaces").execute(&mut *conn).await?;
    }
    sqlx::query("ALTER TABLE layouts RENAME TO workspaces").execute(&mut *conn).await?;
    sqlx::query("UPDATE collection_order SET table_name = 'workspaces' WHERE table_name = 'layouts'")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Raise every node's floor through its last operation, and let the operations go.
async fn start_history_here(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        "INSERT INTO oplog_floor (node_id, pruned_through_seq) \
         SELECT node_id, MAX(seq) FROM oplog WHERE true GROUP BY node_id \
         ON CONFLICT(node_id) DO UPDATE SET pruned_through_seq = \
           MAX(pruned_through_seq, excluded.pruned_through_seq)",
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM oplog").execute(&mut *conn).await?;
    Ok(())
}

/// The file as it was, as a version beside it.
async fn take_a_version_first(pool: &SqlitePool, db: &std::path::Path) -> Result<Uuid> {
    let dir = db.parent().map(|bundle| bundle.join("versions")).context("a showfile with no folder")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
    let id = Uuid::new_v4();
    let path = dir.join(format!("{id}.db"));
    sqlx::query(&format!("VACUUM INTO '{}'", path.display().to_string().replace('\'', "''")))
        .execute(pool)
        .await
        .with_context(|| format!("keeping the show as it was in {}", path.display()))?;
    info!("[showfile] kept the show as it was as version {id}");
    Ok(id)
}

/// The row for the version taken first, written in the upgraded file so the Versions
/// list shows it. Shaped as the generated columns store a `Version`: the optional
/// fields as JSON.
async fn record_the_version(conn: &mut SqliteConnection, id: Uuid, from: i64) -> Result<()> {
    let clock: String = clock_json(conn).await?;
    let name = serde_json::to_string(&format!("Before upgrading from generation {from}"))?;
    sqlx::query(
        "INSERT INTO versions (id, name, created_at, user_id, automatic, clock) VALUES (?, ?, ?, NULL, 1, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(clock)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The clock the floor now holds, as a version records one.
async fn clock_json(conn: &mut SqliteConnection) -> Result<String> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT node_id, pruned_through_seq FROM oplog_floor")
        .fetch_all(&mut *conn)
        .await?;
    let clock: serde_json::Map<String, serde_json::Value> =
        rows.into_iter().map(|(node, seq)| (node, serde_json::json!(seq))).collect();
    Ok(serde_json::Value::Object(clock).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generation_since_the_first_has_a_step() {
        let wanted: Vec<i64> = (FIRST_UPGRADABLE..SCHEMA_GENERATION).collect();
        assert_eq!(STEPS, wanted.as_slice(), "a bump of SCHEMA_GENERATION ships a step");
    }

    #[test]
    fn only_what_a_step_reaches_is_upgradable() {
        assert!(!upgradable(FIRST_UPGRADABLE - 1));
        assert!(upgradable(FIRST_UPGRADABLE));
        assert!(!upgradable(SCHEMA_GENERATION), "the current generation needs nothing");
        assert!(!upgradable(SCHEMA_GENERATION + 1), "a newer file is never walked down");
    }
}
