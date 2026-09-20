//! What the host sends, about every 16 ms, for as long as it wants the wing up.
//!
//! Three things go out every round — the LED frame, the fader values the host is
//! asserting, and a rolling [`Sync`] byte — and each is **its own frame**. grandMA3
//! sends all three whether or not anything moved, and since the wing drops back to its
//! bootloader when the host goes quiet, a host that only sends on change is a host
//! whose wing falls over while nobody is touching it.

use crate::chunk::{Chunk, ChunkError};
use crate::input::{FADERS, FADER_BYTES, FADER_FULL};
use serde::{Deserialize, Serialize};

/// Leaf tags inside a real-time container, going out.
pub mod tag {
    pub const SYNC: u16 = 0x0001;
    /// Same tag the wing reports faders on, written the other way.
    pub const FADERS: u16 = 0x0003;
    pub const LEDS: u16 = 0x0005;
    pub const DMX: u16 = 0x000a;
}

/// 253 brightness bytes, one per LED channel.
pub const LED_BYTES: usize = 253;

/// Set in an outbound fader word.
///
/// **Confirmed on the hardware.** grandMA3 sends `0x9000` where the wing reports
/// `0x1000`, and the bit means "the host is driving this fader". The faders are
/// motorised: a word carrying this flag is a position the motor *holds*, against a
/// hand if need be. Sending it on a fader nobody asked for makes the desk feel jammed.
pub const FADER_ASSERTED: u16 = 0x8000;

/// A rolling byte the host sends every frame. It rises by one and wraps.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sync(pub u8);

impl Sync {
    pub fn tick(&mut self) -> u8 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

/// One frame's worth of what the host is driving.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputState {
    pub leds: Vec<u8>,
    /// Per fader: `Some(v)` to assert a position, `None` to leave it to the operator.
    pub faders: Vec<Option<u16>>,
    pub sync: Sync,
    /// A DMX frame, if this host is driving the wing's ports.
    pub dmx: Option<Vec<u8>>,
}

impl Default for OutputState {
    fn default() -> Self {
        OutputState {
            leds: vec![0; LED_BYTES],
            faders: vec![None; FADERS],
            sync: Sync::default(),
            dmx: None,
        }
    }
}

impl OutputState {
    /// Set one LED channel. Out-of-range is ignored rather than a panic: the channel
    /// numbers come from MA's table, and a table from a newer grandMA3 naming a channel
    /// this build has never heard of should dim a light, not stop the console.
    pub fn set_led(&mut self, channel: usize, level: u8) {
        if let Some(c) = self.leds.get_mut(channel) {
            *c = level;
        }
    }

    /// Set an RGB LED from its three channel offsets.
    pub fn set_rgb(&mut self, r: usize, g: usize, b: usize, colour: [u8; 3]) {
        self.set_led(r, colour[0]);
        self.set_led(g, colour[1]);
        self.set_led(b, colour[2]);
    }

    pub fn all_leds(&mut self, level: u8) {
        self.leds.iter_mut().for_each(|c| *c = level);
    }

    /// Assert a fader position, clamped to the wing's own full scale.
    pub fn set_fader(&mut self, index: usize, value: u16) {
        if let Some(f) = self.faders.get_mut(index) {
            *f = Some(value.min(FADER_FULL));
        }
    }

    pub fn release_fader(&mut self, index: usize) {
        if let Some(f) = self.faders.get_mut(index) {
            *f = None;
        }
    }

    /// The containers for one round of output — **one block per frame**.
    ///
    /// grandMA3 never combines them: its capture is a 48-byte fader frame, a 277-byte
    /// LED frame and a 25-byte sync frame, each its own `0x0027`. Sending all three in
    /// one container is a shape the wing has never been given, and it answers by
    /// resetting itself a few milliseconds after it starts.
    pub fn frames(&self) -> Vec<Chunk> {
        let io = crate::proto::tag::IO;
        let mut out = vec![
            Chunk::group(io, vec![Chunk::bytes(tag::FADERS, self.fader_bytes())]),
            Chunk::group(io, vec![Chunk::bytes(tag::LEDS, self.leds.clone())]),
            Chunk::group(io, vec![Chunk::bytes(tag::SYNC, vec![self.sync.0])]),
        ];
        if let Some(dmx) = &self.dmx {
            out.push(Chunk::group(io, vec![Chunk::bytes(tag::DMX, dmx.clone())]));
        }
        out
    }

    fn fader_bytes(&self) -> Vec<u8> {
        let mut faders = vec![0u8; FADER_BYTES];
        for (i, f) in self.faders.iter().enumerate().take(FADERS) {
            // `None` sends a plain zero, not the flag: these faders are *motorised*,
            // and a word with the flag in it is a position the motor holds against a
            // hand. The first version set the flag on every fader every frame, which
            // pinned all twelve to the bottom and made the desk feel jammed.
            let word = match f {
                Some(v) => FADER_ASSERTED | (v & 0x7fff),
                None => 0,
            };
            faders[i * 2..i * 2 + 2].copy_from_slice(&word.to_le_bytes());
        }
        faders
    }

    /// Everything in one container. Kept for reading a capture back; **not** what goes
    /// on the wire — see [`OutputState::frames`].
    pub fn container(&self) -> Chunk {
        let mut children = vec![
            Chunk::bytes(tag::FADERS, self.fader_bytes()),
            Chunk::bytes(tag::LEDS, self.leds.clone()),
            Chunk::bytes(tag::SYNC, vec![self.sync.0]),
        ];
        if let Some(dmx) = &self.dmx {
            children.push(Chunk::bytes(tag::DMX, dmx.clone()));
        }
        Chunk::group(crate::proto::tag::IO, children)
    }

    /// Read one back, which is what makes a capture replayable.
    pub fn decode(io: &Chunk) -> Result<OutputState, ChunkError> {
        let mut out = OutputState::default();
        if let Ok(b) = io.child_block(tag::LEDS, LED_BYTES) {
            out.leds = b.to_vec();
        }
        if let Ok(b) = io.child_block(tag::FADERS, FADER_BYTES) {
            for i in 0..FADERS {
                let word = u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
                let value = word & 0x7fff;
                out.faders[i] = (value != 0).then_some(value);
            }
        }
        if let Ok(b) = io.child_block(tag::SYNC, 1) {
            out.sync = Sync(b[0]);
        }
        if let Some(dmx) = io.child(tag::DMX).and_then(|d| d.as_bytes().ok()) {
            out.dmx = Some(dmx.to_vec());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_led_frame_is_two_hundred_and_fifty_three_bytes() {
        let mut o = OutputState::default();
        o.set_led(252, 0xff); // the highest offset MA's own table uses
        let c = o.container();
        assert_eq!(c.child_block(tag::LEDS, LED_BYTES).unwrap()[252], 0xff);
    }

    /// One block per frame, the way grandMA3 sends them.
    #[test]
    fn output_goes_out_as_one_block_per_frame() {
        let o = OutputState::default();
        let sizes: Vec<usize> = o.frames().iter().map(|f| f.encode().len() + 16).collect();
        // +16 for the root and the address chunk a frame wraps them in: 48, 277, 25,
        // which are exactly the three sizes in grandMA3's capture.
        assert_eq!(sizes, vec![48, 277, 25]);
    }

    #[test]
    fn a_fader_round_trips_through_a_frame() {
        let mut o = OutputState::default();
        o.set_fader(1, 2048);
        let back = OutputState::decode(&o.container()).unwrap();
        assert_eq!(back.faders[1], Some(2048));
        assert_eq!(back.faders[0], None);
    }

    /// A fader cannot be asked for more than the wing's own full scale; sending 0xffff
    /// would set the flag bit and mean something else entirely.
    #[test]
    fn a_fader_is_clamped_to_full_scale() {
        let mut o = OutputState::default();
        o.set_fader(0, u16::MAX);
        assert_eq!(o.faders[0], Some(FADER_FULL));
        let back = OutputState::decode(&o.container()).unwrap();
        assert_eq!(back.faders[0], Some(FADER_FULL));
    }

    #[test]
    fn sync_wraps() {
        let mut s = Sync(254);
        assert_eq!(s.tick(), 255);
        assert_eq!(s.tick(), 0);
    }

    #[test]
    fn a_channel_past_the_end_is_ignored_rather_than_fatal() {
        let mut o = OutputState::default();
        o.set_led(9999, 0xff);
        assert_eq!(o.leds.len(), LED_BYTES);
    }
}
