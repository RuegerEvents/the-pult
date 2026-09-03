use anyhow::Result;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;
use tracing::info;

pub mod bundle;
pub mod oplog;
pub mod order;
pub mod recent;
pub mod travel;
pub mod versions;

#[cfg(test)]
mod tests;

pub async fn open(path: &std::path::Path) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=rwc", path.display()))?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await?;
    run_migrations(&pool).await?;
    reclaim_if_it_is_worth_it(&pool).await;
    Ok(pool)
}

/// Give the free pages back when there are enough of them to notice.
///
/// SQLite never shrinks a file on its own: a show that imported a rig and had it
/// deleted again keeps the pages, and the only thing that gives them back is a
/// `VACUUM`, which rewrites the whole database and cannot run while anything else is
/// using it. At open there is nothing else using it.
///
/// A quarter is the threshold because a `VACUUM` costs about as long as a copy of the
/// file, and doing it at every start to reclaim a few kilobytes would be a console
/// that takes longer to open the smaller the show gets. Failure is a warning and
/// nothing else — a show that cannot be compacted still opens.
async fn reclaim_if_it_is_worth_it(pool: &SqlitePool) {
    let free: i64 = sqlx::query_scalar("PRAGMA freelist_count").fetch_one(pool).await.unwrap_or(0);
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count").fetch_one(pool).await.unwrap_or(0);
    if pages == 0 || free <= pages / 4 {
        return;
    }
    info!("[showfile] {free} of {pages} pages are free; compacting");
    if let Err(e) = sqlx::query("VACUUM").execute(pool).await {
        tracing::warn!("[showfile] could not compact: {e}");
    }
}

/// A second pool to an already-open showfile, for the writer task and nothing else.
///
/// The showfile is WAL, which allows one writer alongside readers. That is what lets
/// `engine::writer` hold a transaction open for the length of a group commit without
/// a peer's catch-up read queueing behind it — or, worse, landing *inside* it and
/// reading a row that has not committed yet. With a single shared pool capped at one
/// connection, both of those would happen.
///
/// Migrations are not run again: this opens a file that `open` has already brought up
/// to date, and running them twice is work at best and a race at worst.
pub async fn open_for_writing(path: &std::path::Path) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=rwc", path.display()))?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);
    // One, because there is one writer by construction and a second connection here
    // would be a second writer racing it for SQLite's write lock.
    Ok(sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?)
}

/// Open a migrated in-memory showfile.
///
/// The pool is capped at one connection: every SQLite `:memory:` connection gets
/// its own private database, so a larger pool would hand out empty ones.
///
/// Two callers, and they are not both tests. Every in-crate test wants one. So does
/// a console with **no show open** — the engine, the sync layer and the HTTP server
/// all run, because a welcome screen is served over the same socket the show would
/// be, and what they run against is a database that is never written to disk. The
/// asset store is what says no in that state, since nothing else has to.
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
    // Asked before anything is created, because it is the only moment the answer is
    // still available: after `CREATE TABLE IF NOT EXISTS` every file has tables.
    let existed = has_tables(pool).await?;

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
    // After the additive pass, so a column this build has just added is not read as
    // a column an older file was missing.
    refuse_a_file_this_build_cannot_read(pool, existed).await?;

    info!("showfile migrations applied");
    Ok(())
}

/// Which shape of stored data this build reads.
///
/// Bumped by hand whenever a change makes an existing showfile unreadable in a way
/// [`add_missing_columns`] cannot fix: a field whose *shape* changed inside a JSON
/// column, a column that changed meaning, an entity that moved. Not for an added
/// field — that is what the additive pass is for, and bumping on one would refuse
/// files that open perfectly well.
///
/// 1. The first stamp. Everything before it is unstamped and refused on sight.
/// 2. `Fixture::position` became a `Transform` — a position, a rotation and a signed
///    scale, where it had been a point or a point and a direction.
/// 3. A show became a folder, and the assets moved out of the file into `assets/`
///    beside it. The `bytes` column is gone, so a generation 2 file's rows claim
///    assets whose bytes nothing can find.
pub(super) const SCHEMA_GENERATION: i64 = 3;

/// Say plainly that a showfile is from another build, instead of panicking somewhere
/// deep in a generated `from_columns`.
///
/// This console does not migrate showfiles. That is a deliberate decision for as long
/// as it is in development and nobody is carrying a season's work in one: a migration
/// is a promise about every shape the data has ever had, and keeping that promise cost
/// more than reseeding a demo show does. What replaces it is a refusal that names what
/// is wrong.
///
/// Two things can be wrong, and they fail differently, so both are checked.
///
/// **A shape changed.** Nothing about the columns says so — the JSON in them simply
/// means something else — and an `Option` column that fails to parse becomes `None`
/// without an error, which is data loss with no symptom. Only a stamp catches that,
/// and [`SCHEMA_GENERATION`] is it.
///
/// **A non-`Option` field was added.** The additive pass adds its column nullable,
/// because SQLite cannot add a NOT NULL column without a default and most fields have
/// no honest one. Every existing row then holds NULL there, and `from_columns` reads
/// each column on its own and unwraps, so opening the show panics. That needs no stamp
/// to detect: the file says it itself, and the column can be named.
async fn refuse_a_file_this_build_cannot_read(pool: &SqlitePool, existed: bool) -> Result<()> {
    let generation: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(pool).await?;

    if !existed {
        // A file this run created. Stamp it and there is nothing else to say.
        sqlx::query(&format!("PRAGMA user_version = {SCHEMA_GENERATION}"))
            .execute(pool)
            .await?;
        return Ok(());
    }

    if generation != SCHEMA_GENERATION {
        anyhow::bail!(
            "this showfile is generation {generation} and this build reads {SCHEMA_GENERATION}. \
             Showfiles are not migrated while the console is in development: \
             start a fresh one."
        );
    }

    if let Some((table, column)) = a_required_column_nothing_filled_in(pool).await? {
        anyhow::bail!(
            "this showfile predates {table}.{column}, which this build needs a \
             value for in every row. Showfiles are not migrated while the console \
             is in development: start a fresh one."
        );
    }

    Ok(())
}

/// The first NOT NULL column the additive pass had to add nullable and nothing filled.
///
/// One query per table rather than one per column: the common answer is "none", and
/// that costs a table scan nobody notices at open. Only a file that *is* broken pays
/// for the second pass that says which column it was.
async fn a_required_column_nothing_filled_in(
    pool: &SqlitePool,
) -> Result<Option<(String, String)>> {
    for meta in pult_schema::registry::EntityMeta::all_with_tables() {
        let (Some(table), Some(defs)) = (meta.table_name, (meta.column_defs)()) else { continue };

        let required: Vec<String> = defs
            .split(',')
            .filter(|def| def.contains("NOT NULL"))
            .filter_map(|def| def.trim().split_whitespace().next().map(str::to_string))
            .collect();
        if required.is_empty() {
            continue;
        }

        let any_null =
            required.iter().map(|c| format!("{c} IS NULL")).collect::<Vec<_>>().join(" OR ");
        let broken: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE {any_null}"))
                .fetch_one(pool)
                .await?;
        if broken == 0 {
            continue;
        }

        for column in required {
            let count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE {column} IS NULL"
            ))
            .fetch_one(pool)
            .await?;
            if count > 0 {
                return Ok(Some((table.to_string(), column)));
            }
        }
    }
    Ok(None)
}

/// Whether this file has anything in it yet.
async fn has_tables(pool: &SqlitePool) -> Result<bool> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
            .fetch_one(pool)
            .await?;
    Ok(count > 0)
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
