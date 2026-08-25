use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

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
    #[pult(lifecycle = SYNCED)]
    pub live_values: HashMap<String, ParameterValue>,
    #[pult(lifecycle = SYNCED)]
    pub active_preset: Option<Uuid>,
}
