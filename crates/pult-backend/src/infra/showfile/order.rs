//! Persisted display order for entity collections.
//!
//! A collection is stored as an object keyed by id, which has no order of its own, and
//! SQLite hands rows back in whatever order it likes. Without this, a show reopened
//! after a restart listed its sequences in UUID order rather than the order the
//! operator built them in.
//!
//! One table for every collection rather than a column per entity: the order belongs
//! to the collection, not to the entity, and a shared table means adding an entity
//! type still needs no work here.

use std::collections::HashMap;

use anyhow::Result;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Every collection's order, keyed by table name.
pub async fn load_all(pool: &SqlitePool) -> Result<HashMap<String, Vec<Uuid>>> {
    let rows = sqlx::query(
        "SELECT table_name, entity_id FROM collection_order ORDER BY table_name, position",
    )
    .fetch_all(pool)
    .await?;

    let mut order: HashMap<String, Vec<Uuid>> = HashMap::new();
    for row in rows {
        let table: String = row.try_get("table_name")?;
        let id: String = row.try_get("entity_id")?;
        if let Ok(id) = Uuid::parse_str(&id) {
            order.entry(table).or_default().push(id);
        }
    }
    Ok(order)
}

/// Add one entity at the end of a collection's order.
///
/// The cheap half of [`save`], and it exists because the assumption that comment used
/// to make — that creates are human-paced — is false exactly when it matters. An MVR
/// import brings a rig in one create at a time, and rewriting the whole collection
/// after each of them made patching quadratic: seeding 2000 fixtures took 21 seconds
/// and 5000 took over two minutes, of which about three quarters was this table.
///
/// Safe against the thing the full rewrite was protecting: `MAX(position) + 1` is read
/// inside the same statement that inserts, so two appends cannot land on one position,
/// and the primary key would refuse a repeat of the same id in any case.
pub async fn append(pool: &SqlitePool, table: &str, id: Uuid) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO collection_order (table_name, entity_id, position) \
         VALUES (?1, ?2, \
           COALESCE((SELECT MAX(position) + 1 FROM collection_order WHERE table_name = ?1), 0))",
    )
    .bind(table)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace one collection's order.
///
/// Rewrites the whole collection, which is what a reorder or a delete needs: neither
/// can be expressed as one row. A *create* has [`append`] instead, because it can.
pub async fn save(pool: &SqlitePool, table: &str, ids: &[Uuid]) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM collection_order WHERE table_name = ?1")
        .bind(table)
        .execute(&mut *tx)
        .await?;

    for (position, id) in ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO collection_order (table_name, entity_id, position) VALUES (?1, ?2, ?3)",
        )
        .bind(table)
        .bind(id.to_string())
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
