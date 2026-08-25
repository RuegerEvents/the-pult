//! Turning fixture state into DMX channel values.
//!
//! Shared by every protocol in the DMX family, so Art-Net and sACN differ only in
//! how they put the bytes on the wire.

use std::collections::HashMap;

use pult_schema::types::fixture::{Fixture, FixtureType, ParameterDefinition, ParameterValue};
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
pub fn render(patch: &Patch) -> Vec<Universe> {
    let mut universes: HashMap<u16, Universe> = HashMap::new();

    for fixture in &patch.fixtures {
        let Some(fixture_type) = patch.fixture_type(fixture) else {
            // Patched to a type that is not in the show. Nothing sensible to send.
            continue;
        };
        let universe = universes
            .entry(fixture.universe)
            .or_insert_with(|| Universe::new(fixture.universe));

        for parameter in &fixture_type.parameters {
            let value = fixture
                .live_values
                .get(&parameter_key(&parameter.kind))
                .unwrap_or(&parameter.default_value);
            write_parameter(&mut universe.channels, fixture.dmx_address, parameter, value);
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
    parameter: &ParameterDefinition,
    value: &ParameterValue,
) {
    let base = dmx_address as usize + parameter.dmx_channel as usize - 1;
    let Some(start) = base.checked_sub(1) else { return };

    match value {
        ParameterValue::Color { r, g, b } => {
            for (offset, channel) in [r, g, b].into_iter().enumerate() {
                if let Some(slot) = channels.get_mut(start + offset) {
                    *slot = to_byte(*channel);
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
                    ParameterValue::Color { .. } => unreachable!("handled above"),
                };
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
