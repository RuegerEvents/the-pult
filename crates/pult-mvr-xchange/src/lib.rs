//! MVR-xchange, as specified in DIN SPEC 15801: what a message says, and how the
//! bytes around it are shaped.
//!
//! The specification defines **two modes**, and they are not two spellings of one
//! thing:
//!
//! - **TCP mode** — discovery by mDNS under `<group>._mvrxchange._tcp.local.`, and
//!   messages in a framing of the protocol's own ([`frame`]): a `778682` magic, a
//!   version, a package number and count, a payload kind, and a 64-bit length, all
//!   big-endian.
//! - **WebSocket mode** — discovery by ordinary DNS to a URL somebody hosts, and
//!   messages as RFC 6455 frames, text for JSON and binary for a file.
//!
//! The message bodies are identical across the two; only the wrapping differs. So
//! [`message`] is shared and [`frame`] is the TCP half alone — in WebSocket mode the
//! frame *is* the framing.
//!
//! # What is not here
//!
//! No socket, no mDNS daemon, no runtime and no clock. Those are a station's, and
//! they live in `pult-backend`'s `infra/interop/xchange/`. What is here can be tested
//! against a hand-written buffer with nothing running, which is the same reason
//! `pult-gdtf` can be tested against other people's files with no station near it.
//!
//! No MVR content either. A commit names a file by uuid and says how large it is; the
//! archive itself crosses as an opaque buffer and is read by `pult-mvr` at the other
//! end. That is why this crate does not depend on that one.
//!
//! # Reading other people's messages
//!
//! The published specification disagrees with its own examples in three places, and
//! this crate takes the lenient side of each — see [`message`]. A parser that refused
//! them would refuse real software.

pub mod frame;
pub mod message;

pub use frame::{Decoder, FrameError, Payload, PACKAGE_HEADER, PACKAGE_VERSION};
pub use message::{Commit, Message, Response};

/// The mDNS service every TCP-mode client registers under.
///
/// The group is the *sub* service — `<group>._mvrxchange._tcp.local.` — which is why
/// a group name is not a field in any message: it is the address.
pub const SERVICE_TYPE: &str = "_mvrxchange._tcp.local.";

/// What a group is called when nobody has said otherwise.
///
/// grandMA3 ships with this and Vectorworks offers it, so two consoles that have each
/// been switched on and nothing else are already in the same group. That is the whole
/// reason to hard-code a string here rather than make somebody type one.
pub const DEFAULT_GROUP: &str = "Default";

/// The version of the MVR *file format* this console writes, as a commit announces
/// it. Not the version of this protocol, which has no field of its own.
pub const MVR_VERSION: (u32, u32) = (1, 6);
