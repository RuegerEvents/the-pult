use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// A point in the rig, in metres, from whatever origin the show uses.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Where a fixture is, and for a moving one, where it points.
///
/// The spec asks for positions to be either positional (XYZ) or axial (a position
/// and a direction vector). Nothing forces a position to be accurate: a rig can be
/// laid out roughly and corrected later, or updated from tracking data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FixturePosition {
    /// Just where it hangs.
    Point(Vec3),
    /// Where it hangs and the direction it faces at rest.
    Axial { position: Vec3, direction: Vec3 },
}

impl FixturePosition {
    pub fn position(&self) -> Vec3 {
        match self {
            FixturePosition::Point(p) => *p,
            FixturePosition::Axial { position, .. } => *position,
        }
    }
}

/// How output reaches a fixture.
///
/// Not every fixture is on a DMX line. An OpenHaunt node is addressed by the serial
/// of the node it lives on, and only a DMX gateway module also carries a universe —
/// for a relay or a sensor there is no universe to speak of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FixtureAddress {
    Dmx { universe: u16, address: u16 },
    OpenHaunt { serial: String, universe: Option<u16> },
}

impl Default for FixtureAddress {
    fn default() -> Self {
        FixtureAddress::Dmx { universe: 1, address: 1 }
    }
}

impl FixtureAddress {
    /// Universe and start address, for the fixtures that have them.
    pub fn dmx(&self) -> Option<(u16, u16)> {
        match self {
            FixtureAddress::Dmx { universe, address } => Some((*universe, *address)),
            // A gateway module carries a universe but its own address is the node,
            // not a slot in that universe: it owns all 512 channels.
            FixtureAddress::OpenHaunt { .. } => None,
        }
    }

    /// The node serial, for fixtures that live on one.
    pub fn serial(&self) -> Option<&str> {
        match self {
            FixtureAddress::OpenHaunt { serial, .. } => Some(serial),
            FixtureAddress::Dmx { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterKind {
    Intensity,
    ColorRgb,
    Pan,
    Tilt,
    GoboIndex,
    Raw(u8),
    /// A switched output: a relay, a dry contact the console closes.
    Switch(u8),
    /// A switch or button the console reads.
    Contact(u8),
    Temperature,
    Humidity,
    AirQuality,
    /// A line of text, for a display module.
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "value")]
pub enum ParameterValue {
    Float(f32),
    Int(i32),
    Color { r: f32, g: f32, b: f32 },
    Bool(bool),
    Text(String),
}

/// Which way a parameter flows.
///
/// Everything the console has driven so far is an output. A sensor node reverses
/// that: the device writes the value and the show reads it, which is what lets a
/// contact closure be an ordinary fixture parameter rather than a separate concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterDirection {
    #[default]
    Output,
    Input,
}

/// Where a parameter sits on the thing that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterBinding {
    /// An offset from the fixture's DMX start address, 1-based.
    Dmx { channel: u8 },
    /// A port on an I/O module, 0-based, as the module numbers its own terminals.
    Port { index: u8 },
}

impl ParameterBinding {
    pub fn dmx_channel(&self) -> Option<u8> {
        match self {
            ParameterBinding::Dmx { channel } => Some(*channel),
            ParameterBinding::Port { .. } => None,
        }
    }

    pub fn port(&self) -> Option<u8> {
        match self {
            ParameterBinding::Port { index } => Some(*index),
            ParameterBinding::Dmx { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ParameterDefinition {
    pub kind: ParameterKind,
    #[serde(default)]
    pub direction: ParameterDirection,
    pub binding: ParameterBinding,
    pub default_value: ParameterValue,
}

/// `fixture_types.parameters` is one JSON column, so a showfile written before
/// bindings existed holds `dmx_channel` where `binding` now goes. Reading it back
/// through this shape is the whole migration for that field — there is no column to
/// alter, and a show that has never been reopened stays readable.
///
/// Written out by hand rather than through `#[serde(from = ...)]`, because that
/// attribute is one more thing for ts-rs to fail to parse and warn about.
#[derive(Deserialize)]
struct ParameterDefinitionWire {
    kind: ParameterKind,
    #[serde(default)]
    direction: ParameterDirection,
    binding: Option<ParameterBinding>,
    dmx_channel: Option<u8>,
    default_value: ParameterValue,
}

impl<'de> Deserialize<'de> for ParameterDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ParameterDefinitionWire::deserialize(deserializer)?;
        let binding = wire
            .binding
            .or_else(|| wire.dmx_channel.map(|channel| ParameterBinding::Dmx { channel }))
            .unwrap_or(ParameterBinding::Dmx { channel: 1 });
        Ok(ParameterDefinition {
            kind: wire.kind,
            direction: wire.direction,
            binding,
            default_value: wire.default_value,
        })
    }
}

/// Template describing what parameters a fixture type has.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "fixture_types")]
pub struct FixtureType {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub manufacturer: String,
    #[pult(lifecycle = PERSISTED)]
    pub channel_count: u16,
    #[pult(lifecycle = PERSISTED)]
    pub parameters: Vec<ParameterDefinition>,
}

/// A patched fixture instance — a specific unit in the rig.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "fixtures")]
pub struct Fixture {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub fixture_type_id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub address: FixtureAddress,
    /// Where this fixture is in the rig. None until it has been placed.
    #[pult(lifecycle = PERSISTED)]
    pub position: Option<FixturePosition>,
    #[pult(lifecycle = SYNCED)]
    pub live_values: HashMap<String, ParameterValue>,
    #[pult(lifecycle = SYNCED)]
    pub active_preset: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_written_before_bindings_existed_still_loads() {
        let legacy = serde_json::json!({
            "kind": "Intensity",
            "dmx_channel": 4,
            "default_value": { "type": "Float", "value": 0.0 }
        });

        let parsed: ParameterDefinition = serde_json::from_value(legacy).unwrap();

        assert_eq!(parsed.binding, ParameterBinding::Dmx { channel: 4 });
        assert_eq!(parsed.direction, ParameterDirection::Output);
    }

    #[test]
    fn a_parameter_round_trips_through_its_current_shape() {
        let definition = ParameterDefinition {
            kind: ParameterKind::Contact(3),
            direction: ParameterDirection::Input,
            binding: ParameterBinding::Port { index: 3 },
            default_value: ParameterValue::Bool(false),
        };

        let json = serde_json::to_value(&definition).unwrap();
        let back: ParameterDefinition = serde_json::from_value(json).unwrap();

        assert_eq!(back, definition);
    }

    #[test]
    fn an_address_answers_only_for_the_kind_it_is() {
        let dmx = FixtureAddress::Dmx { universe: 2, address: 17 };
        assert_eq!(dmx.dmx(), Some((2, 17)));
        assert_eq!(dmx.serial(), None);

        let node = FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: Some(5) };
        assert_eq!(node.dmx(), None, "a node fixture has no slot in a universe");
        assert_eq!(node.serial(), Some("1a2b3c"));
    }
}
