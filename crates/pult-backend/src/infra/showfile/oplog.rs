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
///
/// A write inside a gesture *replaces* that gesture's earlier write to the same
/// path rather than adding a row beside it — see [`fold_into_the_gesture`].
pub async fn append(pool: &SqlitePool, op: &Operation) -> Result<()> {
    if fold_into_the_gesture(pool, op).await? {
        return Ok(());
    }
    sqlx::query(
        "INSERT OR REPLACE INTO oplog \
         (seq, node_id, op_id, clock_json, path_json, value_json, lifecycle, timestamp, \
          user_id, previous_json, undoes, gesture) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )
    .bind(op.seq as i64)
    .bind(op.node_id.0.to_string())
    .bind(op.id.to_string())
    .bind(serde_json::to_string(&op.clock)?)
    .bind(serde_json::to_string(&op.path)?)
    .bind(serde_json::to_string(&op.value)?)
    .bind(serde_json::to_string(&op.lifecycle)?)
    .bind(op.timestamp.to_rfc3339())
    .bind(op.user_id.map(|id| id.to_string()))
    // `Some(Null)` and `None` are different and the column has to keep them apart:
    // the first means the path was empty and undo should empty it again, the second
    // means nothing was captured and there is nothing to go back to.
    .bind(op.previous.as_ref().map(serde_json::to_string).transpose()?)
    .bind(op.undoes.map(|id| id.to_string()))
    .bind(op.gesture.map(|id| id.to_string()))
    .execute(pool)
    .await?;
    Ok(())
}

/// Fold a write into the gesture's existing row for that path, if there is one.
///
/// A drag writes the same path forty times a second — two seconds across twenty
/// fixtures is 2,400 rows — and the log needs the last of those, not all of them.
/// Both of the things the log is for want exactly that: a peer catching up on a path
/// only needs where it ended, and undo wants the value from before the drag against
/// the value it finished on. So the row keeps its `previous` and takes the new value.
///
/// **It takes the new `seq` too**, which is the part that makes this safe rather than
/// merely smaller. Catch-up asks for everything past a sequence number, so a row that
/// kept its first `seq` while its value moved would be invisible to a peer that had
/// already caught up mid-drag — it would sit at the value the drag passed through
/// when the two of them last spoke. Sequence numbers only ever go up, so the row is
/// simply moved to the front of the queue.
///
/// **Only inside one gesture.** Two separate edits to the same path are two things
/// somebody did and have to stay two rows, or the second would swallow the first and
/// there would be nothing to take back to. That the boundary can be drawn at all is
/// what gestures bought.
///
/// **Never a create.** Every create in a collection is written to the same
/// `<table>/__create` path, so two fixtures patched in one gesture would fold into
/// one row and the log would forget a fixture. Creates and deletes are written once
/// per entity anyway, so there is nothing there to fold.
async fn fold_into_the_gesture(pool: &SqlitePool, op: &Operation) -> Result<bool> {
    let Some(gesture) = op.gesture else { return Ok(false) };
    if is_a_create(op) {
        return Ok(false);
    }
    let folded = sqlx::query(
        "UPDATE oplog SET seq = ?1, clock_json = ?2, value_json = ?3, timestamp = ?4 \
         WHERE node_id = ?5 AND gesture = ?6 AND path_json = ?7",
    )
    .bind(op.seq as i64)
    .bind(serde_json::to_string(&op.clock)?)
    .bind(serde_json::to_string(&op.value)?)
    .bind(op.timestamp.to_rfc3339())
    .bind(op.node_id.0.to_string())
    .bind(gesture.to_string())
    .bind(serde_json::to_string(&op.path)?)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(folded > 0)
}

/// Whether this operation brings an entity into being, and so shares its path with
/// every other create in its collection.
fn is_a_create(op: &Operation) -> bool {
    use pult_schema::path::PathSegment;
    matches!(op.path.as_slice(), [PathSegment::Key(_), PathSegment::Key(last)] if last == "__create")
}

/// Everything the holder of `known` has not seen, oldest first.
///
/// An operation counts as seen when the asking node's clock has reached that
/// operation's sequence number on the node that wrote it.
pub async fn since(pool: &SqlitePool, known: &VectorClock) -> Result<Vec<Operation>> {
    let rows = sqlx::query(
        "SELECT seq, node_id, op_id, clock_json, path_json, value_json, lifecycle, timestamp, \
                user_id, previous_json, undoes, gesture \
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
        // Absent on a row written before undo existed, and on every engine write.
        // Both read as "not something a person can take back", which is true.
        user_id: row
            .try_get::<Option<String>, _>("user_id")
            .ok()
            .flatten()
            .and_then(|id| Uuid::parse_str(&id).ok()),
        previous: row
            .try_get::<Option<String>, _>("previous_json")
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok()),
        undoes: row
            .try_get::<Option<String>, _>("undoes")
            .ok()
            .flatten()
            .and_then(|id| Uuid::parse_str(&id).ok()),
        gesture: row
            .try_get::<Option<String>, _>("gesture")
            .ok()
            .flatten()
            .and_then(|id| Uuid::parse_str(&id).ok()),
    })
}

/// The operations somebody asked for, newest first, for the history panel and undo.
///
/// Authored rows only, and that is load-bearing rather than tidy. A station writes
/// its own telemetry into the log twice a second, so a window over *every* row is
/// about a quarter of an hour long: an operator who renamed a fixture and then did
/// something else for twenty minutes would find it had quietly stopped being
/// undoable. Counting only what people did makes the window a count of edits, which
/// is what an operator thinks it is.
///
/// `limit` because a long show's log is thousands of rows and nobody scrolls that
/// far — and because undo only ever needs the most recent one that qualifies.
pub async fn recent_by_people(pool: &SqlitePool, limit: u32) -> Result<Vec<Operation>> {
    let rows = sqlx::query(
        "SELECT seq, node_id, op_id, clock_json, path_json, value_json, lifecycle, timestamp, \
                user_id, previous_json, undoes, gesture \
         FROM oplog WHERE user_id IS NOT NULL \
         ORDER BY timestamp DESC, seq DESC LIMIT ?1",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().filter_map(read_operation).collect())
}

#[cfg(test)]
mod tests;
