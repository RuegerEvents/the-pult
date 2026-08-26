use anyhow::Result;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;
use tracing::info;

pub mod oplog;
pub mod order;
mod upgrades;

#[cfg(test)]
mod tests;

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

/// Run migrations against an already-open pool. Test-only.
#[cfg(test)]
pub async fn migrate_for_test(pool: &SqlitePool) -> Result<()> {
    run_migrations(pool).await
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

    add_missing_columns(pool).await?;
    // After the additive pass, so a replacement column exists to be filled in.
    upgrades::run(pool).await?;

    info!("showfile migrations applied");
    Ok(())
}

/// Bring an existing showfile's tables up to the current schema.
///
/// `CREATE TABLE IF NOT EXISTS` does nothing to a table that already exists, so a
/// field added to an entity would leave every saved show unopenable: the generated
/// SELECT names a column the file does not have. This walks the registry and adds
/// what is missing, which means adding a field to the schema stays the only edit.
///
/// New columns are always nullable. SQLite cannot add a NOT NULL column without a
/// default, and there is no honest default for a field that did not exist when the
/// existing rows were written.
async fn add_missing_columns(pool: &SqlitePool) -> Result<()> {
    for meta in pult_schema::registry::EntityMeta::all_with_tables() {
        let (Some(table), Some(defs)) = (meta.table_name, (meta.column_defs)()) else { continue };

        let existing: Vec<String> = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await?
            .iter()
            .filter_map(|row| sqlx::Row::try_get::<String, _>(row, "name").ok())
            .collect();
        if existing.is_empty() {
            continue; // brand new table; CREATE TABLE already made it correctly
        }

        for def in defs.split(',') {
            let def = def.trim();
            let Some(column) = def.split_whitespace().next() else { continue };
            if existing.iter().any(|c| c == column) {
                continue;
            }
            let nullable = def
                .replace(" NOT NULL", "")
                .replace(" PRIMARY KEY", "");
            info!("[showfile] adding {table}.{column}");
            sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {nullable}"))
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}
