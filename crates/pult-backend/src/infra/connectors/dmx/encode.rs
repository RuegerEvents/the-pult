//! Turning one parameter value into the bytes of one DMX channel.
//!
//! Pure, and separate from the render loop, because this is where every arithmetic
//! decision that can be silently wrong lives: how wide a channel is, whether the fine
//! byte follows the coarse one or sits nine slots later, what a named range means for
//! an integer, and which byte of a colour is the white one.
//!
//! Two rules hold throughout.
//!
//! **A value is normalised; a channel has a width.** Everything in the console is
//! 0..1, and a channel of *n* bytes holds `round(v · (2^8ⁿ − 1))` — so full is full at
//! every width, rather than 255 shifted up and eight zero bits short.
//!
//! **Offsets are a list, not a start.** `Offset="1,9"` is a real thing a real head
//! writes, and a writer that took the first offset and wrote the rest after it would
//! put the fine byte on top of somebody's colour.

use pult_render::color::EmitterSpec;
use pult_schema::types::dmx_mode::DmxChannelLayout;
use pult_schema::types::fixture::{Emitter, ParameterValue};

use super::UNIVERSE_SIZE;

/// Write one parameter into a universe.
///
/// `address` is the break's 1-based start address. A channel that would run past
/// slot 512 is dropped rather than wrapping into another fixture, and so is one the
/// value has nothing to say about.
pub fn write_channel(
    channels: &mut [u8; UNIVERSE_SIZE],
    address: u16,
    layout: &DmxChannelLayout,
    value: &ParameterValue,
    emitter: Option<&EmitterSpec>,
) {
    let Some(raw) = raw_for(layout, value, emitter) else { return };
    write_bytes(channels, address, &layout.offsets, raw);
}

/// The number this value comes to on this channel, at this channel's width.
///
/// `None` for a value that has no byte: a line of text on a DMX line, or a colour on
/// a channel that names no emitter and so cannot say which component it carries.
pub fn raw_for(
    layout: &DmxChannelLayout,
    value: &ParameterValue,
    emitter: Option<&EmitterSpec>,
) -> Option<u32> {
    let width = layout.byte_count();
    if width == 0 {
        return None;
    }
    let max = layout.max();

    Some(match value {
        ParameterValue::Float(v) => scale(*v, max),
        // An integer picks a named range where the channel has them — a gobo wheel's
        // third slot is wherever the file said it is, not at 3/255 of full. Where it
        // has none, it is a plain byte and clamps.
        ParameterValue::Int(index) => match layout.functions.get((*index).max(0) as usize) {
            Some(range) => range.dmx_from,
            None => (*index).clamp(0, max as i32) as u32,
        },
        ParameterValue::Bool(true) => max,
        ParameterValue::Bool(false) => 0,
        // A display's text has no byte on a DMX line. Leaving the channel alone is
        // the only honest thing to write.
        ParameterValue::Text(_) => return None,
        ParameterValue::Color { r, g, b, overrides } => {
            // One emitter, worked out directly, from a spec the patch resolved once.
            // Mixing the whole fixture and picking one level out of the answer is the
            // same arithmetic and allocates a vector of names per channel per frame —
            // which on a rig of five hundred was measurably the whole cost of colour.
            scale(pult_render::color::level_from([*r, *g, *b], overrides, emitter?), max)
        }
    })
}

/// One emitter in the shape the mixer wants.
///
/// A conversion rather than a shared type: `pult-render` is compiled for the browser
/// too and cannot depend on the schema, so the schema's `Emitter` and the renderer's
/// `EmitterSpec` are two spellings of one thing and this is the one place that knows
/// they are. Called when a patch arrives, never in a frame.
pub fn spec_of(emitter: &Emitter) -> EmitterSpec {
    EmitterSpec {
        name: emitter.name.clone(),
        rgb: emitter.rgb.map(|rgb| [rgb.x, rgb.y, rgb.z]),
        subtractive: emitter.subtractive,
    }
}

/// Put a number across a channel's offsets, most significant byte first.
///
/// The offsets need not be adjacent and need not be in order; what makes a byte
/// significant is its *place in the list*, which is the spec's rule and the one a
/// reader that sorted them would break.
fn write_bytes(channels: &mut [u8; UNIVERSE_SIZE], address: u16, offsets: &[u16], raw: u32) {
    let width = offsets.len().min(4);
    for (index, offset) in offsets.iter().take(width).enumerate() {
        let shift = 8 * (width - 1 - index) as u32;
        let byte = ((raw >> shift) & 0xff) as u8;
        // 1-based address plus 1-based offset, both counting the same first slot.
        let Some(slot) = (address as usize)
            .checked_add(*offset as usize)
            .and_then(|sum| sum.checked_sub(2))
        else {
            continue;
        };
        if let Some(cell) = channels.get_mut(slot) {
            *cell = byte;
        }
    }
}

/// A 0..1 value at a channel's width. Out of range clamps rather than wrapping, so a
/// bad value dims a light instead of flashing it to full.
fn scale(value: f32, max: u32) -> u32 {
    (value.clamp(0.0, 1.0) * max as f32).round() as u32
}
