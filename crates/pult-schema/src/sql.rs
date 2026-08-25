/// A single SQL-bindable value produced by PultSqlRow::to_sql_values.
pub enum SqlLiteral {
    Text(String),
    Int(i64),
    Real(f64),
    Null,
}

/// Protocol-neutral row reader. Implemented for sqlx::sqlite::SqliteRow in pult-backend.
pub trait ColumnGetter {
    fn get_text(&self, col: &str) -> Option<String>;
    fn get_int(&self, col: &str) -> Option<i64>;
    fn get_real(&self, col: &str) -> Option<f64>;
}

/// Auto-derived by #[derive(PultSchema)] for entities with at least one PERSISTED field.
/// Provides the SQL column schema and typed bind/read logic for the generic query layer.
pub trait PultSqlRow: crate::traits::PultEntity {
    /// Comma-separated column definitions, e.g. "id TEXT NOT NULL, name TEXT NOT NULL".
    fn column_defs() -> &'static str;

    /// Column names for PERSISTED fields in declaration order.
    fn column_names() -> &'static [&'static str];

    /// Values in column_names() order for INSERT binding.
    fn to_sql_values(&self) -> Vec<SqlLiteral>;

    /// Reconstruct entity from a SQL row; LOCAL/SYNCED fields get Default::default().
    fn from_columns(row: &dyn ColumnGetter) -> anyhow::Result<Self>;

    /// Full CREATE TABLE IF NOT EXISTS statement, derived from column_defs() and table_name().
    fn create_table_sql() -> String {
        format!(
            "CREATE TABLE IF NOT EXISTS {} (\n    {}\n);",
            Self::table_name().expect("PultSqlRow requires a table name"),
            Self::column_defs()
        )
    }
}
