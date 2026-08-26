//! Changes to an existing showfile that adding a column cannot make.
//!
//! [`super::add_missing_columns`] handles the common case: a new field arrives, it
//! becomes a nullable column, old rows read it as absent. That is enough for
//! anything additive. It is not enough when a field is *replaced* — the old columns
//! are still there and still NOT NULL, so the next write fails the constraint, and
//! the new column is NULL on every existing row, so the next read panics.
//!
//! Each upgrade names itself, decides from `PRAGMA table_info` whether it still
//! applies, and runs statements that are safe to run twice. Opening the same file
//! repeatedly does the work once, and opening a fresh one does none of it, because
//! `CREATE TABLE` already made the table correctly.

use anyhow::Result;
use sqlx::{Row, SqlitePool};
use tracing::info;

/// One irreversible change to an existing showfile.
struct Upgrade {
    /// What it does, for the log.
    name: &'static str,
    /// Which table's columns decide whether it is still needed.
    table: &'static str,
    /// Given that table's column names, is there anything left to do?
    applies: fn(&[String]) -> bool,
    statements: &'static [&'static str],
}

const UPGRADES: &[Upgrade] = &[Upgrade {
    name: "fixtures: universe/dmx_address folded into address",
    table: "fixtures",
    // Both old columns are dropped at the end, so their presence is the flag.
    applies: |columns| columns.iter().any(|c| c == "universe" || c == "dmx_address"),
    statements: &[
        // `address` itself arrived as a nullable column, so every pre-existing row
        // has NULL there and the old pair still holds the truth.
        "UPDATE fixtures \
         SET address = json_object('Dmx', json_object('universe', universe, 'address', dmx_address)) \
         WHERE address IS NULL",
        "ALTER TABLE fixtures DROP COLUMN universe",
        "ALTER TABLE fixtures DROP COLUMN dmx_address",
    ],
}];

pub async fn run(pool: &SqlitePool) -> Result<()> {
    for upgrade in UPGRADES {
        let columns = column_names(pool, upgrade.table).await?;
        if columns.is_empty() || !(upgrade.applies)(&columns) {
            continue;
        }
        info!("[showfile] upgrading — {}", upgrade.name);
        for statement in upgrade.statements {
            sqlx::query(statement).execute(pool).await?;
        }
    }
    Ok(())
}

async fn column_names(pool: &SqlitePool, table: &str) -> Result<Vec<String>> {
    Ok(sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?
        .iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect())
}
