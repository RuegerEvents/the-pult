//! Turning fixture state into DMX channel values.
//!
//! Shared by every protocol in the DMX family, so Art-Net and sACN differ only in
//! how they put the bytes on the wire.

use std::collections::HashMap;

use pult_schema::types::fixture::{Fixture, FixtureType, ParameterDirection, ParameterValue};
use uuid::Uuid;

use crate::model::playback::parameter_key;

/// A DMX universe: 512 channels, indexed from 0 for channel 1.
pub const UNIVERSE_SIZE: usize = 512;

/// One universe of channel data.
#[derive(Clone)]
pub struct Universe {
    pub number: u16,
    pub channels: [u8; UNIVERSE_SIZE],
}

impl Universe {
    pub fn new(number: u16) -> Self {
        Self { number, channels: [0; UNIVERSE_SIZE] }
    }
}

/// The patch: what a plugin needs to place a fixture's values on a wire.
pub struct Patch {
    pub fixtures: Vec<Fixture>,
    pub fixture_types: HashMap<Uuid, FixtureType>,
}

impl Patch {
    pub fn fixture_type(&self, fixture: &Fixture) -> Option<&FixtureType> {
        self.fixture_types.get(&fixture.fixture_type_id)
    }
}

/// Render the whole patch into universes, one per universe number in use.
///
/// Only fixtures with a DMX address take part. A fixture on an OpenHaunt node has
/// no slot in a universe, and neither does a parameter bound to a port or one the
/// device writes rather than reads — none of those have a channel to occupy.
pub fn render(patch: &Patch) -> Vec<Universe> {
    let mut universes: HashMap<u16, Universe> = HashMap::new();

    for fixture in &patch.fixtures {
        let Some((number, address)) = fixture.address.dmx() else { continue };
        let Some(fixture_type) = patch.fixture_type(fixture) else {
            // Patched to a type that is not in the show. Nothing sensible to send.
            continue;
        };
        let universe = universes.entry(number).or_insert_with(|| Universe::new(number));

        for parameter in &fixture_type.parameters {
            if parameter.direction != ParameterDirection::Output {
                continue;
            }
            let Some(channel) = parameter.binding.dmx_channel() else { continue };
            let value = fixture
                .live_values
                .get(&parameter_key(&parameter.kind))
                .unwrap_or(&parameter.default_value);
            write_parameter(&mut universe.channels, address, channel, value);
        }
    }

    let mut out: Vec<Universe> = universes.into_values().collect();
    out.sort_by_key(|u| u.number);
    out
}

/// Write one parameter into the universe at the fixture's address.
///
/// Addresses are 1-based on the outside and 0-based in the buffer. A parameter that
/// would run past channel 512 is dropped rather than wrapping into another fixture.
fn write_parameter(
    channels: &mut [u8; UNIVERSE_SIZE],
    dmx_address: u16,
    channel: u8,
    value: &ParameterValue,
) {
    let base = dmx_address as usize + channel as usize - 1;
    let Some(start) = base.checked_sub(1) else { return };

    match value {
        ParameterValue::Color { r, g, b } => {
            for (offset, component) in [r, g, b].into_iter().enumerate() {
                if let Some(slot) = channels.get_mut(start + offset) {
                    *slot = to_byte(*component);
                }
            }
        }
        other => {
            if let Some(slot) = channels.get_mut(start) {
                *slot = match other {
                    ParameterValue::Float(f) => to_byte(*f),
                    ParameterValue::Int(i) => (*i).clamp(0, 255) as u8,
                    ParameterValue::Bool(true) => 255,
                    ParameterValue::Bool(false) => 0,
                    // A display's text has no byte on a DMX line. Leaving the channel
                    // alone is the only honest thing to write.
                    ParameterValue::Text(_) => return,
                    ParameterValue::Color { .. } => unreachable!("handled above"),
                };
            }
        }
    }
}

// ── Not sending what has not changed ──────────────────────────────────────────

/// A DMX-family protocol expects a receiver to hear from a controller regularly.
/// Re-sending every universe about once a second keeps one from deciding the
/// controller is gone, without putting an idle rig's full output on the wire 40
/// times a second.
pub const REFRESH_AFTER: std::time::Duration = std::time::Duration::from_millis(800);

/// Remembers what was last sent per universe, so an unchanged one is skipped.
///
/// Shared by every protocol that carries whole universes — Art-Net, sACN, and the
/// unicast the OpenHaunt gateway wants — because the rule is the same for all of
/// them and getting it subtly different per protocol is how an idle rig starts
/// flooding one wire and not another.
#[derive(Default)]
pub struct UniverseCache {
    sent: Vec<(u16, [u8; UNIVERSE_SIZE], std::time::Instant)>,
}

impl UniverseCache {
    /// True if this universe has changed, or has gone long enough without a refresh.
    /// Records the universe as sent, so calling it twice for one frame is wrong.
    pub fn needs_send(
        &mut self,
        universe: &Universe,
        now: std::time::Instant,
        refresh_after: std::time::Duration,
    ) -> bool {
        match self.sent.iter_mut().find(|(n, _, _)| *n == universe.number) {
            Some((_, channels, last)) => {
                let changed = *channels != universe.channels;
                if changed || now.duration_since(*last) >= refresh_after {
                    *channels = universe.channels;
                    *last = now;
                    true
                } else {
                    false
                }
            }
            None => {
                self.sent.push((universe.number, universe.channels, now));
                true
            }
        }
    }
}

/// A sequence counter per universe, for the protocols that carry one.
///
/// Zero means "sequence not implemented" in both Art-Net and E1.31, so it wraps
/// through 1..=255 rather than through zero.
#[derive(Default)]
pub struct SequenceCounter {
    counters: Vec<(u16, u8)>,
}

impl SequenceCounter {
    pub fn next(&mut self, universe: u16) -> u8 {
        match self.counters.iter_mut().find(|(u, _)| *u == universe) {
            Some((_, seq)) => {
                *seq = if *seq >= 255 { 1 } else { *seq + 1 };
                *seq
            }
            None => {
                self.counters.push((universe, 1));
                1
            }
        }
    }
}

/// A 0.0 to 1.0 parameter as a DMX byte. Out-of-range values clamp rather than wrap,
/// so a bad value dims a light instead of flashing it to full.
fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests;
