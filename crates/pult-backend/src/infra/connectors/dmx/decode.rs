//! Turning the bytes of one DMX channel back into a parameter value.
//!
//! The inverse of [`super::encode`], and it exists so that **a recording is decoded
//! values against time, never bytes**. A capture is `(fixture, parameter, value)`
//! wherever it comes from, so anything arriving on a wire that is going to land in the
//! programmer or in a track goes through the patch on the way — byte to value through
//! the mode the fixture is patched in, which is the only place that knows what the
//! byte meant.
//!
//! The same two rules as going out, read backwards. **Significance is a channel's
//! place in the offset list**, not its slot number, so a head with `Offset="1,9"` is
//! read coarse-then-fine and not in slot order. And **a channel has a width**, so a
//! 16-bit intensity divides by 65535 rather than by 255 and comes back as the same
//! 0..1 it went out as.
//!
//! **A colour is assembled per parameter, not per channel.** Every emitter channel of
//! one colour is read, turned into a level, and handed to
//! [`pult_render::color::unmix`] together — which is the only way the round trip can
//! be exact on a fixture whose emitters the mix cannot derive. An RGBW head comes back
//! as a colour with its white pinned where the mix would not have put it; a CMY head
//! comes back as three overrides. Re-encoding either gives the bytes that arrived,
//! and the corpus asserts exactly that.

use pult_render::color::EmitterSpec;
use pult_schema::types::dmx_mode::DmxChannelLayout;
use pult_schema::types::fixture::ParameterValue;

use super::UNIVERSE_SIZE;

/// Read a number back off a channel's offsets, most significant byte first.
///
/// The mirror of `encode::write_bytes`, and it has to stay one: an offset list this
/// read sorted, or walked the other way round, would come back with the fine byte in
/// the coarse byte's place and a head at 1% would read as a head at 99%.
pub fn read_bytes(channels: &[u8; UNIVERSE_SIZE], address: u16, offsets: &[u16]) -> u32 {
    let width = offsets.len().min(4);
    let mut raw: u32 = 0;
    for (index, offset) in offsets.iter().take(width).enumerate() {
        let shift = 8 * (width - 1 - index) as u32;
        let Some(slot) = (address as usize)
            .checked_add(*offset as usize)
            .and_then(|sum| sum.checked_sub(2))
        else {
            continue;
        };
        let byte = channels.get(slot).copied().unwrap_or(0);
        raw |= (byte as u32) << shift;
    }
    raw
}

/// What a raw channel value means, given what kind of thing this parameter is.
///
/// `default` is the type's own default value and is here as the *kind*: a wire carries
/// a number and nothing else, so the only thing that can say whether 128 is a half, a
/// gobo or a closed relay is the parameter it landed on.
///
/// `None` for the two that have no byte to read: a colour, which is assembled from
/// every emitter channel at once by [`crate::infra::connectors::dmx::Patch::decode`],
/// and a line of text, which was never written to a slot in the first place.
pub fn value_for(
    layout: &DmxChannelLayout,
    raw: u32,
    default: &ParameterValue,
) -> Option<ParameterValue> {
    let max = layout.max();
    if max == 0 {
        return None;
    }
    Some(match default {
        ParameterValue::Float(_) => ParameterValue::Float(raw as f32 / max as f32),
        // The inverse of picking a named range: a gobo wheel's byte comes back as the
        // *index* of the range it falls in, so what the console holds is "the third
        // gobo" and not a number that happens to be inside it. A byte in no declared
        // range, and a channel with no ranges at all, is a plain number — which is
        // what `raw_for` writes for an index it has no range for.
        ParameterValue::Int(_) => ParameterValue::Int(
            match layout
                .functions
                .iter()
                .position(|range| raw >= range.dmx_from && raw <= range.dmx_to)
            {
                Some(index) => index as i32,
                None => raw as i32,
            },
        ),
        // Half way. A relay is written as 0 or full and a real console in the middle
        // of the range meant it to be on.
        ParameterValue::Bool(_) => ParameterValue::Bool(raw >= max / 2),
        ParameterValue::Color { .. } | ParameterValue::Text(_) => return None,
    })
}

/// One emitter's level, from the bytes its channel carries.
pub fn level_of(channels: &[u8; UNIVERSE_SIZE], address: u16, layout: &DmxChannelLayout) -> f32 {
    let max = layout.max();
    if max == 0 {
        return 0.0;
    }
    read_bytes(channels, address, &layout.offsets) as f32 / max as f32
}

/// A colour from every emitter channel of one parameter that a universe was read for.
///
/// `levels` is the type's own emitter order, which is the order [`pult_render::mix`]
/// writes them in — so the primary each RGB component comes from is the same emitter
/// both ways, and the round trip is exact rather than nearly.
pub fn color_from(levels: &[(EmitterSpec, f32)]) -> ParameterValue {
    let color = pult_render::color::unmix(levels);
    ParameterValue::Color {
        r: color.r,
        g: color.g,
        b: color.b,
        overrides: color.overrides,
    }
}
