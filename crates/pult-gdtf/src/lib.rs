//! GDTF: General Device Type Format, read and written.
//!
//! A pure format library — `quick-xml`, `serde`, `zip`, `uuid`, `thiserror`, and no
//! pult crate. Nothing here knows what a `FixtureType` is in the console's sense;
//! the translation between this object model and the schema lives in the backend, so
//! that this crate can be tested against the spec and against other people's files
//! without a station anywhere near it.
//!
//! ```no_run
//! # fn main() -> Result<(), pult_gdtf::Error> {
//! let bytes = std::fs::read("robe@spiider.gdtf").unwrap();
//! let file = pult_gdtf::GdtfFile::parse(&bytes)?;
//! for mode in &file.description.fixture_type.dmx_modes.items {
//!     println!("{}: {:?}", mode.name, pult_gdtf::resolve::footprint(&file.description.fixture_type, mode));
//! }
//! # Ok(()) }
//! ```
//!
//! Writing is the other half and the reason this exists rather than a read-only
//! crate off crates.io: the console exports its own fixture types as GDTF, and a
//! reader alone cannot do that.

pub mod archive;
pub mod canonical;
pub mod minimal;
pub mod model;
pub mod resolve;
pub mod validate;
pub mod values;
pub mod xml;

pub use archive::GdtfFile;
pub use canonical::canonicalize;
pub use model::Gdtf;

/// What can go wrong reading or writing a GDTF file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this is not a zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("a .gdtf must contain a description.xml at its root")]
    NoDescription,
    #[error("the archive entry {0:?} names a path outside the archive")]
    BadEntry(String),
    #[error("{0}")]
    TooLarge(&'static str),
    #[error("reading the XML: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("reading the XML: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("the description.xml does not match the GDTF schema: {0}")]
    De(#[from] quick_xml::DeError),
    #[error("writing the XML: {0}")]
    Se(#[from] quick_xml::SeError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Something the console should tell the operator about, that did not stop the file
/// being read.
///
/// A warning rather than an error throughout: a Share file with a dangling geometry
/// reference is still a fixture somebody needs to patch tonight, and refusing it
/// helps nobody. Every one carries the path it was found at, so the Import report can
/// point at the thing rather than at the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Where in the document, dot-separated: `DMXModes.Mode 1.Pan`.
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
