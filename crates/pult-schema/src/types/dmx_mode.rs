//! How a fixture type's parameters land on a DMX line, in the mode it is patched in.
//!
//! A mode is a detail of *addressing*, not of what a light can do. The parameter
//! list on the type is the whole of what an operator can set; a mode says which of
//! those parameters occupy which bytes, how wide each one is, and which DMX break it
//! is in. A moving head patched in its 16-bit mode and the same head in its 8-bit
//! mode have the same parameters and different layouts.
//!
//! Two consequences hold this together.
//!
//! **A footprint is a list, not a number.** A fixture with a separate break for its
//! dimmer occupies two spans, and they need not be adjacent or even in the same
//! universe. `channel_count` survives as the first break's, because that is what the
//! patch panel has always meant by it, and [`super::FixtureType::footprint`] is the
//! real answer.
//!
//! **A type with no modes still has one.** Everything patched before this existed —
//! the demo seed, a type derived from an OpenHaunt node, a showfile from last year —
//! carries a `Dmx { channel }` binding per parameter and nothing else. Rather than
//! rewriting those at load, [`super::FixtureType::mode`] computes an implicit
//! `"Default"` from them, so nothing on the read path changed and no showfile had to
//! be migrated to keep working.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One way of addressing a fixture type over DMX.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DmxMode {
    pub name: String,
    /// How many channels this mode occupies in each DMX break, break 1 first.
    ///
    /// Dense: a mode that uses breaks 1 and 3 and not 2 has a zero in the middle, so
    /// the index into this list is always the break number minus one.
    pub breaks: Vec<u16>,
    pub channels: Vec<DmxChannelLayout>,
}

impl DmxMode {
    /// The first break's footprint, which is what a single-break fixture means by
    /// "how many channels".
    pub fn channel_count(&self) -> u16 {
        self.breaks.first().copied().unwrap_or(0)
    }

    /// Whether this mode carries the named parameter at all.
    ///
    /// What the programmer greys a control out on: a head in its basic mode has no
    /// zoom, and offering one that goes nowhere is worse than saying it is not there.
    pub fn has(&self, parameter_key: &str) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.parameter_key == parameter_key)
    }
}

/// Where one parameter sits in one mode.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DmxChannelLayout {
    /// Which parameter of the type this drives, by its
    /// [`super::parameter_key`].
    pub parameter_key: String,
    /// Which DMX break, 0-based into [`DmxMode::breaks`].
    pub break_index: u8,
    /// The bytes this channel occupies, 1-based from the break's start address,
    /// coarse to fine. One to four of them, and they need not be adjacent: a head
    /// commonly puts every coarse byte first and every fine byte after.
    ///
    /// Empty for a parameter the mode declares and does not place — a virtual
    /// channel, which the console can still show and cannot send.
    pub offsets: Vec<u16>,
    /// What the fixture powers up at, at this channel's width.
    pub default: u32,
    /// The named ranges of this channel, in order. Empty for a plain continuous one.
    #[serde(default)]
    pub functions: Vec<ChannelFunctionRange>,
    /// For a colour parameter, which emitter of the type this byte carries.
    ///
    /// A colour is one parameter and several channels — an RGBW head has four — so
    /// the layout entry that says "this byte is the white one" is what lets one
    /// `Color` value become four bytes without the connector guessing an order.
    #[serde(default)]
    pub emitter: Option<String>,
}

impl DmxChannelLayout {
    /// How many bytes wide this channel is. Zero for a virtual one.
    pub fn byte_count(&self) -> u8 {
        self.offsets.len().min(4) as u8
    }

    /// The largest value this channel can hold.
    pub fn max(&self) -> u32 {
        match self.byte_count() {
            0 => 0,
            n if n >= 4 => u32::MAX,
            n => (1u32 << (8 * n as u32)) - 1,
        }
    }
}

/// A named slice of a channel's range: "Open", "Breakup", "Strobe".
///
/// Both ends filled in, unlike GDTF's own form, which gives only the start of each
/// and leaves the reader to infer the end from the next one. Doing that inference
/// once, at import, is what keeps it out of the frame path.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChannelFunctionRange {
    pub name: String,
    /// The GDTF attribute this range is, where the file said one: a shutter channel's
    /// strobe range is a different attribute from its open range.
    #[serde(default)]
    pub attribute: String,
    pub dmx_from: u32,
    pub dmx_to: u32,
    pub physical_from: f32,
    pub physical_to: f32,
}

/// One DMX break's place in the world: which universe, and where in it.
///
/// A break rather than "the address", because a fixture can have more than one and
/// they can be in different universes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DmxBreak {
    pub universe: u16,
    /// 1-based, as a patch sheet writes it.
    pub address: u16,
}
