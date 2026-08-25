use crate::lifecycle::Lifecycle;

/// Implemented by every entity type generated via #[derive(PultSchema)].
pub trait PultEntity: Sized {
    /// The SQLite table name for PERSISTED entities. None for non-persisted.
    fn table_name() -> Option<&'static str>;

    /// List of (field_name, lifecycle) for all fields.
    fn field_lifecycles() -> &'static [(&'static str, Lifecycle)];

    /// The field name that is the primary key (UUID), if any.
    fn primary_key_field() -> Option<&'static str>;
}
