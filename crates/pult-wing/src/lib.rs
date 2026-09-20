//! The grandMA3 command wing, on paper.
//!
//! `docs/WING-PROTOCOL.md` is the prose; this is the same thing as code. Four layers,
//! each of which can be read without the one above it:
//!
//! * [`chunk`] — the grammar. Every frame is one chunk, and a chunk is a tag, a length
//!   and a body that is either bytes or more chunks.
//! * [`proto`] — what the tags mean, and [`Message`], which is a frame with its
//!   container recognised.
//! * [`input`] / [`output`] — the blocks inside a real-time frame, decoded into events
//!   and encoded from state.
//! * [`map`] — which index is which control, read out of MA's own table.
//!
//! Nothing here opens a device. A transport hands [`Message::decode`] the bytes it read
//! and puts the bytes of [`Message::encode`] on the wire.

pub mod chunk;
pub mod input;
pub mod map;
pub mod output;
pub mod proto;

pub use chunk::{Chunk, ChunkError};
pub use input::{Event, InputState};
pub use map::{Control, MapError, WingMap, COMMAND_WING};
pub use output::{OutputState, Sync};
pub use proto::{DeviceState, Message, NodeId, Capabilities};
