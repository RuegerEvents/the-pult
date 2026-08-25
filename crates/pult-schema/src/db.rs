use anyhow::Result;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::sql::{ColumnGetter, PultSqlRow, SqlLiteral};

// ── ColumnGetter impl for SqliteRow ───────────────────────────────────────────
// ColumnGetter is local to this crate, so implementing it for the external
// sqlx::sqlite::SqliteRow is allowed by the orphan rules.

impl ColumnGetter for sqlx::sqlite::SqliteRow {
    fn get_text(&self, col: &str) -> Option<String> {
        self.try_get::<String, _>(col).ok()
    }
    fn get_int(&self, col: &str) -> Option<i64> {
        self.try_get::<i64, _>(col).ok()
    }
    fn get_real(&self, col: &str) -> Option<f64> {
        self.try_get::<f64, _>(col).ok()
    }
}

// ── Generic query functions ───────────────────────────────────────────────────

pub async fn get_all<T: PultSqlRow>(pool: &SqlitePool) -> Result<Vec<T>> {
    let table = T::table_name().expect("get_all requires an entity with a table name");
    let cols = T::column_names().join(", ");
    let rows = sqlx::query(&format!("SELECT {cols} FROM {table}"))
        .fetch_all(pool)
        .await?;
    rows.iter().map(|r| T::from_columns(r)).collect()
}

pub async fn get_by_id<T: PultSqlRow>(pool: &SqlitePool, id: Uuid) -> Result<Option<T>> {
    let table = T::table_name().expect("get_by_id requires an entity with a table name");
    let pk = T::primary_key_field().expect("get_by_id requires an entity with a primary key");
    let cols = T::column_names().join(", ");
    let id_str = id.to_string();
    let row = sqlx::query(&format!("SELECT {cols} FROM {table} WHERE {pk} = ?1"))
        .bind(&id_str)
        .fetch_optional(pool)
        .await?;
    row.map(|r| T::from_columns(&r)).transpose()
}

pub async fn upsert<T: PultSqlRow>(pool: &SqlitePool, entity: &T) -> Result<()> {
    let table = T::table_name().expect("upsert requires an entity with a table name");
    let pk = T::primary_key_field().expect("upsert requires an entity with a primary key");
    let col_names = T::column_names();
    let values = entity.to_sql_values();

    let placeholders: Vec<String> = (1..=col_names.len()).map(|i| format!("?{i}")).collect();
    let updates: Vec<String> = col_names
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != pk)
        .map(|(i, c)| format!("{c} = ?{}", i + 1))
        .collect();

    let sql = format!(
        "INSERT INTO {table} ({cols}) VALUES ({placeholders}) \
         ON CONFLICT({pk}) DO UPDATE SET {updates}",
        cols = col_names.join(", "),
        placeholders = placeholders.join(", "),
        updates = updates.join(", "),
    );

    let mut query = sqlx::query(&sql);
    for val in values {
        query = match val {
            SqlLiteral::Text(s) => query.bind(s),
            SqlLiteral::Int(i) => query.bind(i),
            SqlLiteral::Real(r) => query.bind(r),
            SqlLiteral::Null => query.bind(Option::<String>::None),
        };
    }
    query.execute(pool).await?;
    Ok(())
}

pub async fn delete<T: PultSqlRow>(pool: &SqlitePool, id: Uuid) -> Result<()> {
    let table = T::table_name().expect("delete requires an entity with a table name");
    let pk = T::primary_key_field().expect("delete requires an entity with a primary key");
    sqlx::query(&format!("DELETE FROM {table} WHERE {pk} = ?1"))
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}
