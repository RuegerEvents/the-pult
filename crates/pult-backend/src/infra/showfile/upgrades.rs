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
use uuid::Uuid;

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
    triggers_become_flows(pool).await?;
    Ok(())
}

/// Redraw every one-row-per-rule trigger as the graph it always was.
///
/// A trigger is a source, a condition, a delay and an action in a row, which is
/// exactly a four-node chain — so nothing about the show changes, only how it is
/// written down. Not an [`Upgrade`] because that decides from a table's *columns*
/// whether it still applies, and what settles this one is whether the table is there
/// at all: `CREATE TABLE` never makes `triggers` again, and the `DROP` at the end
/// means a second open finds nothing to do.
async fn triggers_become_flows(pool: &SqlitePool) -> Result<()> {
    if column_names(pool, "triggers").await?.is_empty() {
        return Ok(());
    }

    let rows = sqlx::query("SELECT id, name, source, condition, action, delay_ms, enabled FROM triggers")
        .fetch_all(pool)
        .await?;
    info!("[showfile] upgrading — {} trigger(s) become flows", rows.len());

    for (row_index, row) in rows.iter().enumerate() {
        let flow_id = Uuid::new_v4();
        let name: String = row.try_get("name").unwrap_or_else(|_| "Trigger".into());
        let enabled: i64 = row.try_get("enabled").unwrap_or(1);
        let delay_ms: i64 = row.try_get("delay_ms").unwrap_or(0);
        let source: String = row.try_get("source")?;
        let condition: String = row.try_get("condition")?;
        let action: String = row.try_get("action")?;

        sqlx::query("INSERT INTO flows (id, name, enabled) VALUES (?, ?, ?)")
            .bind(flow_id.to_string())
            .bind(&name)
            .bind(enabled)
            .execute(pool)
            .await?;

        // Laid out left to right with a row per rule, so a show with a dozen
        // triggers opens as a dozen readable chains rather than a pile at the origin.
        let y = row_index as f64 * 140.0;
        let mut chain: Vec<Uuid> = Vec::new();
        let mut place = |kind: String| -> (Uuid, String, f64) {
            let id = Uuid::new_v4();
            let x = chain.len() as f64 * 220.0;
            chain.push(id);
            (id, kind, x)
        };

        let steps = if delay_ms > 0 {
            vec![
                format!("{{\"Source\":{source}}}"),
                format!("{{\"Condition\":{condition}}}"),
                format!("{{\"Delay\":{{\"ms\":{delay_ms}}}}}"),
                format!("{{\"Action\":{action}}}"),
            ]
        } else {
            vec![
                format!("{{\"Source\":{source}}}"),
                format!("{{\"Condition\":{condition}}}"),
                format!("{{\"Action\":{action}}}"),
            ]
        };

        for kind in steps {
            let (id, kind, x) = place(kind);
            sqlx::query(
                "INSERT INTO flow_nodes (id, flow_id, kind, x, y) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(flow_id.to_string())
            .bind(kind)
            .bind(x)
            .bind(y)
            .execute(pool)
            .await?;
        }

        for pair in chain.windows(2) {
            sqlx::query(
                "INSERT INTO flow_edges (id, flow_id, from_node, from_port, to_node, to_port) \
                 VALUES (?, ?, ?, 0, ?, 0)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(flow_id.to_string())
            .bind(pair[0].to_string())
            .bind(pair[1].to_string())
            .execute(pool)
            .await?;
        }
    }

    sqlx::query("DROP TABLE triggers").execute(pool).await?;
    // The order table outlives the table it ordered, and a stale row there would
    // have `post_load_init` reserving a place for an entity nothing can produce.
    sqlx::query("DELETE FROM collection_order WHERE table_name = 'triggers'")
        .execute(pool)
        .await?;
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
