// Allow generated code that references ::pult_schema::... to work within this crate.
extern crate self as pult_schema;

pub mod lifecycle;
pub mod path;
pub mod handle;
pub mod traits;
pub mod events;
pub mod types;
pub mod ws;
pub mod sql;
pub mod db;
pub mod registry;
pub mod commands;

pub use pult_macros::PultSchema;
pub use pult_macros::pult_commands;

pub mod prelude {
    pub use crate::lifecycle::Lifecycle;
    pub use crate::path::{Path, PathPattern, PathSegment};
    pub use crate::handle::{DataHandle, EntityCollectionAccessor, FieldAccessor, HandleError, ShowDataRoot};
    pub use crate::traits::PultEntity;
    pub use crate::sql::{ColumnGetter, PultSqlRow, SqlLiteral};
    pub use crate::PultSchema;
}

// ── ShowDataRoot extension methods ────────────────────────────────────────────
// These add collection accessors for top-level entity types.

use handle::{DataHandle, EntityCollectionAccessor, ShowDataRoot};
use types::{
    fixture::{FixtureAccessor, FixtureTypeAccessor},
    sequence::SequenceAccessor,
};

impl<H: DataHandle> ShowDataRoot<H> {
    pub fn sequences(&self) -> EntityCollectionAccessor<SequenceAccessor<H>> {
        EntityCollectionAccessor::new(
            vec![path::PathSegment::Key("sequences".into())],
            self.handle.clone(),
        )
    }

    pub fn fixtures(&self) -> EntityCollectionAccessor<FixtureAccessor<H>> {
        EntityCollectionAccessor::new(
            vec![path::PathSegment::Key("fixtures".into())],
            self.handle.clone(),
        )
    }

    pub fn fixture_types(&self) -> EntityCollectionAccessor<FixtureTypeAccessor<H>> {
        EntityCollectionAccessor::new(
            vec![path::PathSegment::Key("fixtureTypes".into())],
            self.handle.clone(),
        )
    }
}
