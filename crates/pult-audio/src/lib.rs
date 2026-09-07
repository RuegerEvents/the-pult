//! Sound, as numbers.
//!
//! The audio half of the timeline, in the shape [`pult_gdtf`](https://docs.rs/pult-gdtf)
//! is in: **no OS, no socket, no clock, and no pult crate**. Everything here takes f32
//! samples and a sample rate and answers with numbers, so all of it can be tested with
//! no sound card in the machine and no station running — which is what the console's
//! own suite does, and it is why an LTC decoder can be exercised against every frame
//! rate at four sample rates in a few milliseconds.
//!
//! Five modules, and each is one question:
//!
//! - [`peaks`] — what does this file look like, small enough to send to a tablet?
//! - [`ltc`] — what does this cable say the time is?
//! - [`beats`] — where are the beats, and which of them are the "one"?
//! - [`lock`] — how do we chase a clock that is not ours without the show jumping?
//! - [`resample`] — the two rate changes the other four need.
//!
//! What is *not* here is anything that talks to hardware. The device, the decoding of
//! an mp3, the file the peaks are stored as and the transport they are written into
//! all live in the station: `crates/pult-backend/src/infra/audio/`. That split is the
//! same one `pult-gdtf` and `infra/interop/gdtf/` have, and it exists for the same
//! reason — a format library that has never met a station can be pointed at other
//! people's files.

pub mod beats;
pub mod lock;
pub mod ltc;
pub mod peaks;
pub mod resample;

pub use beats::{grid_from_beats, Detected, Detector, GridSegment};
pub use lock::{chase, Chase};
pub use ltc::{Decoder, Lock, LtcFrame, LtcRate};
pub use peaks::Peaks;

use thiserror::Error;

/// What can be wrong with a peaks file.
#[derive(Debug, Error, PartialEq)]
pub enum PeaksError {
    #[error("this is not a peaks file")]
    NotPeaks,
    #[error("peaks file version {0}, which this console cannot read")]
    Version(u16),
    #[error("the peaks file stops in the middle of its bins")]
    Truncated,
}

/// What can go wrong running the detector.
///
/// Three cases rather than one string, because they mean different things to whoever
/// is looking: a model that will not load is an installation, a shape that is wrong is
/// a model that is not the one this console was built against, and a run that fails is
/// a machine that has run out of something.
#[derive(Debug, Error)]
pub enum BeatsError {
    #[error("the beat model could not be loaded: {0}")]
    Model(String),
    #[error("the beat model is not shaped the way this console expects: {0}")]
    Shape(String),
    #[error("the beat model could not be run: {0}")]
    Run(String),
}
