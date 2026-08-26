//! The operation log: replicated writes, kept so a peer that reconnects can be told
//! what it missed instead of being sent the whole show again.

use anyhow::Result;
use chrono::{DateTime, Utc};
use pult_schema::{
    events::operation::{NodeId, Operation, VectorClock},
    lifecycle::Lifecycle,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Append one operation. Called for every write that is replicated, wherever it came
/// from, so any node can serve catch-up for operations that originated elsewhere.
pub async fn append(pool: &SqlitePool, op: &Operation) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO oplog \
         (seq, node_id, op_id, clock_json, path_json, value_json, lifecycle, timestamp) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(op.seq as i64)
    .bind(op.node_id.0.to_string())
    .bind(op.id.to_string())
    .bind(serde_json::to_string(&op.clock)?)
    .bind(serde_json::to_string(&op.path)?)
    .bind(serde_json::to_string(&op.value)?)
    .bind(serde_json::to_string(&op.lifecycle)?)
    .bind(op.timestamp.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Everything the holder of `known` has not seen, oldest first.
///
/// An operation counts as seen when the asking node's clock has reached that
/// operation's sequence number on the node that wrote it.
pub async fn since(pool: &SqlitePool, known: &VectorClock) -> Result<Vec<Operation>> {
    let rows = sqlx::query(
        "SELECT seq, node_id, op_id, clock_json, path_json, value_json, lifecycle, timestamp \
         FROM oplog ORDER BY timestamp, seq",
    )
    .fetch_all(pool)
    .await?;

    let mut missing = Vec::new();
    for row in rows {
        let Some(op) = read_operation(&row) else { continue };
        if known.0.get(&op.node_id).copied().unwrap_or(0) < op.seq {
            missing.push(op);
        }
    }
    Ok(missing)
}

/// How many operations the log holds. Used to decide whether catch-up is cheaper
/// than a snapshot.
pub async fn len(pool: &SqlitePool) -> Result<u64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM oplog").fetch_one(pool).await?;
    Ok(row.try_get::<i64, _>("n").unwrap_or(0).max(0) as u64)
}

/// A row that will not parse is skipped rather than failing the whole catch-up:
/// one unreadable operation should not cost a peer its reconnection.
fn read_operation(row: &sqlx::sqlite::SqliteRow) -> Option<Operation> {
    let timestamp: String = row.try_get("timestamp").ok()?;
    Some(Operation {
        id: Uuid::parse_str(&row.try_get::<String, _>("op_id").ok()?).ok()?,
        node_id: NodeId(Uuid::parse_str(&row.try_get::<String, _>("node_id").ok()?).ok()?),
        seq: row.try_get::<i64, _>("seq").ok()?.max(0) as u64,
        clock: serde_json::from_str(&row.try_get::<String, _>("clock_json").ok()?).ok()?,
        lifecycle: serde_json::from_str::<Lifecycle>(
            &row.try_get::<String, _>("lifecycle").ok()?,
        )
        .ok()?,
        path: serde_json::from_str(&row.try_get::<String, _>("path_json").ok()?).ok()?,
        value: serde_json::from_str(&row.try_get::<String, _>("value_json").ok()?).ok()?,
        timestamp: DateTime::parse_from_rfc3339(&timestamp).ok()?.with_timezone(&Utc),
    })
}

#[cfg(test)]
mod tests;
