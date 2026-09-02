//! Other people's file formats, read and written.
//!
//! GDTF for a fixture type, MVR for a scene. Both are zipped XML, both are read by a
//! crate of their own that knows the format and nothing about this console, and both
//! land in the show through [`apply`] — which is where the rules about *writing* live:
//! one gesture, validate before anything is stored, and undo the lot if a write fails
//! halfway.

pub mod apply;
pub mod gdtf;
pub mod share;
