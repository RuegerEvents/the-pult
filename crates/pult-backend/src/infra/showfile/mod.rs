use anyhow::Result;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;
use tracing::info;

pub async fn open(path: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{path}?mode=rwc"))?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

/// Open a migrated in-memory showfile. Test-only.
///
/// The pool is capped at one connection: every SQLite `:memory:` connection gets
/// its own private database, so a larger pool would hand out empty ones.
#[cfg(test)]
pub async fn open_in_memory() -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    let sql = include_str!("migrations/001_initial.sql");

    // Split on ';' and execute each non-empty statement individually.
    // Strip comment lines first — the first chunk includes the file header comments
    // followed by the first CREATE TABLE, so checking starts_with("--") on the raw
    // chunk would skip both the comment AND the real statement.
    for chunk in sql.split(';') {
        let stmt: String = chunk
            .lines()
            .filter(|l| !l.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        let stmt = stmt.trim();
        if !stmt.is_empty() {
            sqlx::query(stmt).execute(pool).await?;
        }
    }

    info!("showfile migrations applied");
    Ok(())
}
