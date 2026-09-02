//! MVR — My Virtual Rig, read and written.
//!
//! A pure format library, like `pult-gdtf` beside it: `quick-xml`, `serde`, `zip`,
//! `uuid`, `thiserror`, and `pult-gdtf` for the fixture definitions inside an archive
//! and the space conversion they share. No pult crate above those, so this can be
//! tested against other people's files with no station anywhere near it. The
//! translation into the console's schema lives in `pult-backend`.
//!
//! ```no_run
//! # fn main() -> Result<(), pult_mvr::Error> {
//! let bytes = std::fs::read("rig.mvr").unwrap();
//! let file = pult_mvr::MvrFile::parse(&bytes)?;
//! for layer in &file.scene.scene.layers.items {
//!     println!("{} ({})", layer.name, layer.uuid);
//! }
//! # Ok(()) }
//! ```
//!
//! Writing is the other half and the reason this exists rather than a reader: a show
//! that can be imported and not exported is a show somebody has to redraw.

pub mod address;
pub mod archive;
pub mod model;
pub mod transform;
pub mod values;

pub use archive::{MvrFile, SpecMatch};
pub use model::GeneralSceneDescription;
pub use transform::Placement;

/// What can go wrong reading or writing an MVR file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this is not a zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("an .mvr must contain a GeneralSceneDescription.xml at its root")]
    NoSceneDescription,
    #[error("the archive entry {0:?} names a path outside the archive")]
    BadEntry(String),
    #[error("{0}")]
    TooLarge(&'static str),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Everything quick-xml and the GDTF reader can say, which is one enum already.
    #[error(transparent)]
    Gdtf(#[from] pult_gdtf::Error),
}

/// Something the console should tell the operator about, that did not stop the file
/// being read.
///
/// Every leniency in this crate produces one. A file that needed forgiving is still a
/// rig somebody has to patch tonight, so it is read — and the operator is told, once,
/// where and what, rather than finding out when a light is in the wrong place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Where in the document or the archive: an object's name, or a file's.
    pub at: String,
    pub message: String,
}

impl Warning {
    pub fn new(at: impl Into<String>, message: impl Into<String>) -> Self {
        Warning {
            at: at.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}
