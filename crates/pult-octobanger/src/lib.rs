//! OctoBanger, read.
//!
//! The OctoBanger is an eight-channel Arduino prop controller from ButtonBanger
//! (2017): a trigger input, eight TTL outputs, and a serial MP3 player, programmed
//! from a Windows app that saves one controller as an `.OBC`. This reads those files so
//! a prop built on one can be moved onto OpenHaunt nodes without being programmed again
//! by hand. A pure format library, in the shape of `pult-gdtf`: `serde`, `zip` and
//! `thiserror`, **no pult crate**, so it is tested against real files with no station
//! anywhere near it. The translation into a show is
//! `crates/pult-backend/src/infra/interop/octobanger/`.
//!
//! **An `.OBC` is the app's own objects, saved with .NET's BinaryFormatter.** [`nrbf`]
//! reads that format, which is published, and builds a tree of class names and values
//! out of it — it never instantiates anything, which is the difference between this
//! and the deserializer. [`Controller`] is the reading of that tree.
//!
//! What the numbers in it mean comes from the firmware (`OctoBanger_TTL.ino`, which
//! ships beside the app) and from the enum definitions in the app's own metadata — not
//! from guessing, which is what the first reading of these files did and got three
//! enum values wrong. Each field says where its meaning came from.

pub mod controller;
pub mod nrbf;
pub mod project;

pub use controller::{Channel, Controller, Media, MediaKind, PinMapping, TimingOffset, TriggerLevel};
pub use project::{Project, ProjectController};

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// The first bytes are not a BinaryFormatter header.
    #[error("this is not an OctoBanger controller file: it does not start the way one does")]
    NotBinaryFormatter,
    /// The app can also save in a format of its own it calls *Document*; nobody has
    /// sent one yet, so this importer has never seen one and says so rather than
    /// guessing at it.
    #[error("this controller was saved in the app's Document format, which this console cannot read yet; save it as a binary .OBC")]
    DocumentFormat,
    #[error("the file ends part way through, at byte {at}")]
    Truncated { at: usize },
    #[error("the file is damaged at byte {at}: {what}")]
    Malformed { at: usize, what: &'static str },
    #[error("the file nests deeper than any controller does, at byte {at}")]
    TooDeep { at: usize },
    #[error("the file holds {record} at byte {at}, which a controller never does")]
    Unsupported { at: usize, record: &'static str },
    /// It read, and what it holds is not a controller.
    #[error("this file holds a {0}, not an OctoBanger controller")]
    NotAController(String),
    #[error("the controller has no {0}")]
    Missing(&'static str),
    #[error("the archive could not be read: {0}")]
    Zip(String),
    #[error("{0} is larger than any file of its kind has a reason to be")]
    TooLarge(String),
    #[error("the archive holds no .OBC file")]
    NoController,
}
