//! What the wing reports, and what it means.
//!
//! Four blocks arrive inside a real-time frame, each a whole snapshot sent when
//! something in it moved. Turning a snapshot into events needs the previous one, which
//! is what [`InputState`] is for.
//!
//! The index rules were each confirmed by pressing one control and watching one byte
//! change — see *Four blocks come in, one goes out* in `docs/WING-PROTOCOL.md`.

use crate::chunk::{Chunk, ChunkError};
use serde::{Deserialize, Serialize};

/// Leaf tags inside a real-time container.
pub mod tag {
    pub const KEYS: u16 = 0x0002;
    pub const FADERS: u16 = 0x0003;
    pub const ENCODERS: u16 = 0x0004;
    pub const DIGITAL: u16 = 0x000f;
    pub const TEXT: u16 = 0x0012;
}

/// 56 bytes, one bit per hardkey.
pub const KEY_BYTES: usize = 56;
/// 24 bytes, twelve little-endian `u16`.
pub const FADER_BYTES: usize = 24;
/// 77 bytes, one signed byte per encoder.
pub const ENCODER_BYTES: usize = 77;
/// How many faders that is.
pub const FADERS: usize = FADER_BYTES / 2;
/// How many hardkeys the block can address.
pub const KEYS: usize = KEY_BYTES * 8;
/// Full scale of a fader. The ADC is 12-bit, so this is one past the top.
pub const FADER_FULL: u16 = 0x1000;

/// Detents in one full turn of an encoder.
///
/// A fact about the hardware rather than the protocol — the wire only ever says how
/// many clicks — but anything drawing a knob, or converting clicks to a proportion of
/// a revolution, needs it, and there should be one of it.
pub const ENCODER_DETENTS: u32 = 24;

/// Something a person did.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// A hardkey went down or came up. `index` indexes MA's own hardkey table.
    Key { index: u16, down: bool },
    /// A fader moved. `value` is 0..=4096.
    Fader { index: u8, value: u16 },
    /// An encoder turned. `delta` is how many clicks, signed; several at once when
    /// the spin is faster than the wing reports.
    Encoder { index: u8, delta: i32 },
    /// A digital input changed. `bits` is the whole byte.
    Digital { bits: u8 },
}

/// Where a hardkey's bit lives — a flat bitmap, and nothing more than that.
///
/// ```text
/// hardkey index = byte * 8 + bit
/// ```
///
/// An earlier version of this put a `^ 1` word swap in, and was wrong on the hardware:
/// pressing Hilight reported `GOBACK`. The swap is real, but it belongs to reading
/// grandMA3's `-DEBUGUSBDATA` log, which prints the block as 16-bit groups and so has
/// its bytes the other way round from the wire. Deriving the rule from the log and
/// then applying it to wire bytes swaps twice. Five named keys pressed on a real desk
/// settle it; see the test below.
#[inline]
pub fn key_bit(index: u16) -> (usize, u8) {
    ((index / 8) as usize, (index % 8) as u8)
}

/// The last thing the wing said, so that the next thing can be a difference.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputState {
    pub keys: Vec<u8>,
    pub faders: Vec<u16>,
    pub encoders: Vec<i8>,
    pub digital: u8,
    /// True once a block of each kind has been seen. Before that there is nothing to
    /// difference against, and reporting the first snapshot as a burst of events would
    /// fire every key that happens to be held at connect.
    seen: Seen,
}

#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
struct Seen {
    keys: bool,
    faders: bool,
    encoders: bool,
    digital: bool,
}

impl Default for InputState {
    fn default() -> Self {
        InputState {
            keys: vec![0; KEY_BYTES],
            faders: vec![0; FADERS],
            encoders: vec![0; ENCODER_BYTES],
            digital: 0,
            seen: Seen::default(),
        }
    }
}

impl InputState {
    /// Is this hardkey down?
    pub fn key(&self, index: u16) -> bool {
        let (byte, bit) = key_bit(index);
        self.keys.get(byte).is_some_and(|b| b & (1 << bit) != 0)
    }

    pub fn fader(&self, index: usize) -> u16 {
        self.faders.get(index).copied().unwrap_or(0)
    }

    /// Apply an event, so that a listener and a replayer hold the same thing.
    pub fn apply(&mut self, e: &Event) {
        match *e {
            Event::Key { index, down } => {
                let (byte, bit) = key_bit(index);
                if let Some(b) = self.keys.get_mut(byte) {
                    if down {
                        *b |= 1 << bit;
                    } else {
                        *b &= !(1 << bit);
                    }
                }
            }
            Event::Fader { index, value } => {
                if let Some(f) = self.faders.get_mut(index as usize) {
                    *f = value;
                }
            }
            Event::Encoder { .. } => {}
            Event::Digital { bits } => self.digital = bits,
        }
    }

    /// Difference one real-time container against what is held, and update it.
    pub fn absorb(&mut self, io: &Chunk) -> Result<Vec<Event>, ChunkError> {
        let mut out = Vec::new();

        if let Ok(b) = io.child_block(tag::KEYS, KEY_BYTES) {
            if self.seen.keys {
                for (i, (old, new)) in self.keys.iter().zip(b).enumerate() {
                    let changed = old ^ new;
                    for bit in 0..8u8 {
                        if changed & (1 << bit) != 0 {
                            out.push(Event::Key {
                                index: index_of(i, bit),
                                down: new & (1 << bit) != 0,
                            });
                        }
                    }
                }
            }
            self.keys = b.to_vec();
            self.seen.keys = true;
        }

        if let Ok(b) = io.child_block(tag::FADERS, FADER_BYTES) {
            for i in 0..FADERS {
                let v = u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
                if self.seen.faders && self.faders[i] != v {
                    out.push(Event::Fader { index: i as u8, value: v });
                }
                self.faders[i] = v;
            }
            self.seen.faders = true;
        }

        if let Ok(b) = io.child_block(tag::ENCODERS, ENCODER_BYTES) {
            for (i, &raw) in b.iter().enumerate() {
                let new = raw as i8;
                // **The byte is the turn, not a position**, and *every* non-zero
                // reading is a turn — including one identical to the last.
                //
                // The wing sends this block when something moved, so two unhurried
                // clicks one way arrive as `1` and then `1` again, with no zero
                // between them to tell them apart. An earlier version also required
                // the value to have *changed*, which swallowed the second and every
                // one after it: turning slowly did nothing at all, while turning fast
                // worked, because a quick spin lands as `2`, `3`, `5` and those
                // differ. The only safe reading is that a block carrying a non-zero
                // byte is a turn of that many clicks.
                if self.seen.encoders && new != 0 {
                    out.push(Event::Encoder { index: i as u8, delta: new as i32 });
                }
                self.encoders[i] = new;
            }
            self.seen.encoders = true;
        }

        if let Ok(b) = io.child_block(tag::DIGITAL, 1) {
            if self.seen.digital && self.digital != b[0] {
                out.push(Event::Digital { bits: b[0] });
            }
            self.digital = b[0];
            self.seen.digital = true;
        }

        Ok(out)
    }
}

/// The inverse of [`key_bit`].
#[inline]
fn index_of(byte: usize, bit: u8) -> u16 {
    (byte * 8 + bit as usize) as u16
}

/// Difference a container against a **fresh** state.
///
/// Only useful for asking "what does this one frame assert", e.g. when reading a
/// capture. It is not how a wing is followed: a release shows up as a bit going to
/// zero, which against a zeroed baseline is no difference and therefore no event. Use
/// [`InputState::absorb`] and keep the state.
pub fn decode(io: &Chunk) -> Result<Vec<Event>, ChunkError> {
    let mut s = InputState::default();
    s.seen = Seen { keys: true, faders: true, encoders: true, digital: true };
    s.absorb(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five named keys, pressed one at a time on a grandMA3 onPC command wing, with the
    /// raw byte and bit read off the wire. This is the gate on the index rule: it was
    /// wrong once, in a way that looked plausible and named the wrong key on every
    /// press.
    #[test]
    fn a_pressed_key_lands_on_the_index_ma_uses() {
        for (byte, bit, index, what) in [
            (25usize, 3u8, 203u16, "HIGHLIGHT"),
            (25, 2, 202, "SOLO"),
            (27, 4, 220, "BLIND"),
            (19, 3, 155, "STORE"),
            (17, 2, 138, "DEF_GO"),
        ] {
            assert_eq!(index_of(byte, bit), index, "{what}");
            assert_eq!(key_bit(index), (byte, bit), "{what}");
        }
    }

    #[test]
    fn every_key_index_round_trips() {
        for i in 0..KEYS as u16 {
            let (byte, bit) = key_bit(i);
            assert_eq!(index_of(byte, bit), i, "hardkey {i}");
        }
    }

    /// Little-endian, like everything else in this protocol. grandMA3's *log* prints
    /// the block as big-endian 16-bit groups, which is where an earlier reading of this
    /// went wrong; the bytes on the wire say otherwise.
    #[test]
    fn a_fader_is_little_endian_and_twelve_bits() {
        let mut b = vec![0u8; FADER_BYTES];
        b[3] = 0x10; // fader 1 at full: 00 10 little-endian
        let io = Chunk::group(0x0027, vec![Chunk::bytes(tag::FADERS, b)]);
        let events = decode(&io).unwrap();
        assert!(events.contains(&Event::Fader { index: 1, value: FADER_FULL }));
    }

    /// The encoder byte is the turn itself: every non-zero reading is a turn, and
    /// going back to zero is not one.
    ///
    /// Two failures live here, both found on the desk. Read as a counter, every click
    /// is followed by an equal and opposite event when the byte clears, and the
    /// encoder never leaves where it started. Read as a turn but only when the value
    /// *changes*, two unhurried clicks the same way arrive as `1` and `1` and the
    /// second is swallowed — slow turning does nothing while fast turning works.
    #[test]
    fn an_encoder_reports_the_turn_and_not_the_release() {
        let mut s = InputState::default();
        let frame = |v: u8| {
            let mut b = vec![0u8; ENCODER_BYTES];
            b[12] = v;
            Chunk::group(0x0027, vec![Chunk::bytes(tag::ENCODERS, b)])
        };
        s.absorb(&frame(0)).unwrap(); // first sight, no events
        assert_eq!(s.absorb(&frame(1)).unwrap(), vec![Event::Encoder { index: 12, delta: 1 }]);
        assert_eq!(s.absorb(&frame(0)).unwrap(), vec![], "clearing is not a turn");
        assert_eq!(s.absorb(&frame(255)).unwrap(), vec![Event::Encoder { index: 12, delta: -1 }]);
        assert_eq!(s.absorb(&frame(0)).unwrap(), vec![], "nor the other way");
        // A spin faster than the reporting rate arrives as several clicks at once.
        assert_eq!(s.absorb(&frame(5)).unwrap(), vec![Event::Encoder { index: 12, delta: 5 }]);

        // And one click, then another the same way, with no zero between them: both
        // are turns, however alike they look.
        let mut s = InputState::default();
        s.absorb(&frame(0)).unwrap();
        let one = vec![Event::Encoder { index: 12, delta: 1 }];
        assert_eq!(s.absorb(&frame(1)).unwrap(), one, "first click");
        assert_eq!(s.absorb(&frame(1)).unwrap(), one, "second click, identical byte");
        assert_eq!(s.absorb(&frame(1)).unwrap(), one, "and a third");
    }

    /// The first block of each kind sets the baseline and fires nothing. Otherwise a
    /// key held at connect would arrive as a press nobody made.
    #[test]
    fn the_first_snapshot_is_not_a_burst_of_events() {
        let mut s = InputState::default();
        let mut b = vec![0u8; KEY_BYTES];
        b[25] = 0x08; // Hilight
        let io = Chunk::group(0x0027, vec![Chunk::bytes(tag::KEYS, b)]);
        assert_eq!(s.absorb(&io).unwrap(), vec![]);
        assert!(s.key(203));
    }

    /// A key must go off as well as on. Differencing a frame against a fresh state —
    /// which is what the message decoder used to do — reports the press and loses the
    /// release, and every key on the desk latches on and stays on.
    #[test]
    fn a_key_goes_off_as_well_as_on() {
        let frame = |set: bool| {
            let mut b = vec![0u8; KEY_BYTES];
            if set {
                b[19] = 0x08; // Store
            }
            Chunk::group(0x0027, vec![Chunk::bytes(tag::KEYS, b)])
        };
        let mut s = InputState::default();
        s.absorb(&frame(false)).unwrap(); // baseline
        assert_eq!(
            s.absorb(&frame(true)).unwrap(),
            vec![Event::Key { index: 155, down: true }]
        );
        assert_eq!(
            s.absorb(&frame(false)).unwrap(),
            vec![Event::Key { index: 155, down: false }]
        );

        // And the stateless helper is exactly the trap: it sees the press and not the
        // release, which is why it is not on the decode path.
        assert_eq!(decode(&frame(true)).unwrap().len(), 1);
        assert_eq!(decode(&frame(false)).unwrap().len(), 0);
    }

    #[test]
    fn a_block_of_the_wrong_size_is_not_decoded() {
        let io = Chunk::group(0x0027, vec![Chunk::bytes(tag::KEYS, vec![0u8; 10])]);
        assert_eq!(decode(&io).unwrap(), vec![]);
    }
}
