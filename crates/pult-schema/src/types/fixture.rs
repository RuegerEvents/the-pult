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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterKind {
    Intensity,
    ColorRgb,
    Pan,
    Tilt,
    GoboIndex,
    Raw(u8),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "value")]
pub enum ParameterValue {
    Float(f32),
    Int(i32),
    Color { r: f32, g: f32, b: f32 },
    Bool(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParameterDefinition {
    pub kind: ParameterKind,
    pub dmx_channel: u8,
    pub default_value: ParameterValue,
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
    pub universe: u16,
    #[pult(lifecycle = PERSISTED)]
    pub dmx_address: u16,
    /// Where this fixture is in the rig. None until it has been placed.
    #[pult(lifecycle = PERSISTED)]
    pub position: Option<FixturePosition>,
    #[pult(lifecycle = SYNCED)]
    pub live_values: HashMap<String, ParameterValue>,
    #[pult(lifecycle = SYNCED)]
    pub active_preset: Option<Uuid>,
}
