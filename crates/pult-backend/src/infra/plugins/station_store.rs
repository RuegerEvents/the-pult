//! What a plugin remembers about *this machine*.
//!
//! The other half of [`super::manifest::StoreScope`]. Show-scoped data is an
//! ordinary entity and gets replication and the showfile for free; this half
//! deliberately gets neither. A cached grammar, a last-used provider, the model
//! that happens to be installed on this console — none of that belongs in a
//! showfile, where it would replicate to machines it is not true of and land in
//! every backup.
//!
//! So it lives beside `preferences.toml`, for the same reason and with the same
//! contract: **never a reason to keep an operator from their show.** A station
//! that cannot open the file logs once and carries on with every store reading
//! empty. A plugin's cache is not worth a failed start.
//!
//! One SQLite file rather than a directory of TOML, because two consoles on one
//! machine can have it open at once, a quota wants `SUM(length(value))` rather
//! than a directory walk, and sqlx is already here.
//!
//! It is keyed by `(plugin_id, store, key)` and knows nothing about which show
//! is open, which is the point: the same station reads back what it wrote after
//! opening a different show.

use std::path::PathBuf;

use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::{info, warn};

/// Where the file lives.
///
/// `PULT_PLUGIN_DATA` names it outright, the way `PULT_PREFERENCES` does, so
/// tests get their own and two stations on one machine can be kept apart.
pub fn path() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PULT_PLUGIN_DATA") {
        return Some(PathBuf::from(named));
    }
    Some(crate::infra::preferences::config_dir()?.join("the-pult").join("plugin-data.db"))
}

/// The station's plugin data, open. `None` when it could not be, which every
/// caller treats as "the stores are empty" rather than as a failure.
#[derive(Clone)]
pub struct StationStore(Option<SqlitePool>);

impl StationStore {
    /// Open it, or log why not and carry on without it.
    pub async fn open() -> StationStore {
        match Self::try_open().await {
            Ok(store) => store,
            Err(e) => {
                warn!("[plugins] station store unavailable, plugin stores will read empty: {e}");
                StationStore(None)
            }
        }
    }

    async fn try_open() -> Result<StationStore, String> {
        let Some(path) = path() else {
            return Err("no configuration directory on this platform".into());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let options = SqliteConnectOptions::new().filename(&path).create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS plugin_data (\n\
             plugin_id TEXT NOT NULL,\n\
             store     TEXT NOT NULL,\n\
             key       TEXT NOT NULL,\n\
             value     TEXT NOT NULL,\n\
             PRIMARY KEY (plugin_id, store, key)\n\
             )",
        )
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

        info!("[plugins] station store at {}", path.display());
        Ok(StationStore(Some(pool)))
    }

    /// An empty one, for a station that has no business writing to disk.
    pub fn none() -> StationStore {
        StationStore(None)
    }

    pub async fn get(&self, plugin_id: &str, store: &str, key: &str) -> Value {
        let Some(pool) = &self.0 else { return Value::Null };
        let row = sqlx::query(
            "SELECT value FROM plugin_data WHERE plugin_id = ?1 AND store = ?2 AND key = ?3",
        )
        .bind(plugin_id)
        .bind(store)
        .bind(key)
        .fetch_optional(pool)
        .await;
        match row {
            Ok(Some(row)) => row
                .try_get::<String, _>("value")
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or(Value::Null),
            Ok(None) => Value::Null,
            Err(e) => {
                warn!("[plugins] station store read failed: {e}");
                Value::Null
            }
        }
    }

    /// Every key of one store, and what it holds — for the quota check, which
    /// needs the sizes, and for `keys`, which needs the names.
    pub async fn rows(&self, plugin_id: &str, store: &str) -> Vec<(String, Value)> {
        let Some(pool) = &self.0 else { return Vec::new() };
        let rows = sqlx::query(
            "SELECT key, value FROM plugin_data WHERE plugin_id = ?1 AND store = ?2 ORDER BY key",
        )
        .bind(plugin_id)
        .bind(store)
        .fetch_all(pool)
        .await;
        match rows {
            Ok(rows) => rows
                .iter()
                .filter_map(|row| {
                    let key = row.try_get::<String, _>("key").ok()?;
                    let text = row.try_get::<String, _>("value").ok()?;
                    Some((key, serde_json::from_str(&text).unwrap_or(Value::Null)))
                })
                .collect(),
            Err(e) => {
                warn!("[plugins] station store read failed: {e}");
                Vec::new()
            }
        }
    }

    pub async fn set(
        &self,
        plugin_id: &str,
        store: &str,
        key: &str,
        value: &Value,
    ) -> Result<(), String> {
        let Some(pool) = &self.0 else {
            return Err("this station has no plugin data store".into());
        };
        let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO plugin_data (plugin_id, store, key, value) VALUES (?1, ?2, ?3, ?4)\n\
             ON CONFLICT (plugin_id, store, key) DO UPDATE SET value = excluded.value",
        )
        .bind(plugin_id)
        .bind(store)
        .bind(key)
        .bind(text)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(&self, plugin_id: &str, store: &str, key: &str) -> Result<(), String> {
        let Some(pool) = &self.0 else { return Ok(()) };
        sqlx::query("DELETE FROM plugin_data WHERE plugin_id = ?1 AND store = ?2 AND key = ?3")
            .bind(plugin_id)
            .bind(store)
            .bind(key)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn a_store() -> (StationStore, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("pult-station-store-{}.db", uuid::Uuid::new_v4()));
        std::env::set_var("PULT_PLUGIN_DATA", &path);
        let store = StationStore::open().await;
        (store, path)
    }

    #[tokio::test]
    async fn what_was_written_reads_back() {
        let (store, path) = a_store().await;

        assert_eq!(store.get("nl", "prefs", "provider").await, Value::Null, "nothing yet");
        store.set("nl", "prefs", "provider", &json!("ollama")).await.unwrap();
        assert_eq!(store.get("nl", "prefs", "provider").await, json!("ollama"));

        // Writing again replaces rather than duplicating: the key is the key.
        store.set("nl", "prefs", "provider", &json!("openrouter")).await.unwrap();
        assert_eq!(store.get("nl", "prefs", "provider").await, json!("openrouter"));
        assert_eq!(store.rows("nl", "prefs").await.len(), 1);

        store.delete("nl", "prefs", "provider").await.unwrap();
        assert_eq!(store.get("nl", "prefs", "provider").await, Value::Null);
        // Forgetting what is not there is not an error.
        store.delete("nl", "prefs", "provider").await.unwrap();

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn one_plugin_cannot_see_anothers() {
        let (store, path) = a_store().await;

        store.set("a", "cache", "shared-name", &json!(1)).await.unwrap();
        store.set("b", "cache", "shared-name", &json!(2)).await.unwrap();

        assert_eq!(store.get("a", "cache", "shared-name").await, json!(1));
        assert_eq!(store.get("b", "cache", "shared-name").await, json!(2));
        assert_eq!(store.rows("a", "cache").await.len(), 1, "and neither sees the other's");

        let _ = std::fs::remove_file(path);
    }

    /// The contract this file shares with `preferences.rs`: a station that
    /// cannot open its store still starts, with the stores reading empty.
    #[tokio::test]
    async fn a_store_that_cannot_be_opened_reads_empty_rather_than_failing() {
        let store = StationStore::none();

        assert_eq!(store.get("nl", "prefs", "provider").await, Value::Null);
        assert!(store.rows("nl", "prefs").await.is_empty());
        // A write says so — a plugin asking to remember deserves an answer —
        // but a read, a listing and a delete are all quietly empty.
        assert!(store.set("nl", "prefs", "provider", &json!("x")).await.is_err());
        assert!(store.delete("nl", "prefs", "provider").await.is_ok());
    }
}
