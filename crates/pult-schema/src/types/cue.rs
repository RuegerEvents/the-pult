use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fixture::{ParameterKind, ParameterValue};
use crate::PultSchema;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FollowMode {
    /// Wait for the operator to press Go.
    Manual,
    /// Auto-fire after the previous cue completes, plus a delay.
    FollowAfter { delay_ms: u32 },
    /// Fire at a specific SMPTE timecode position.
    Timecode { hours: u8, minutes: u8, seconds: u8, frames: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParameterCapture {
    pub fixture_id: Uuid,
    pub parameter_kind: ParameterKind,
    pub value: ParameterValue,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    pub delay_in_ms: u32,
}

/// A single lighting state snapshot with timing information.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "cues")]
pub struct Cue {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// Fractional cue number (1.0, 1.5, 2.0) — allows insertions.
    #[pult(lifecycle = PERSISTED)]
    pub number: f64,
    #[pult(lifecycle = PERSISTED)]
    pub captures: Vec<ParameterCapture>,
    #[pult(lifecycle = PERSISTED)]
    pub follow_mode: FollowMode,
    #[pult(lifecycle = PERSISTED)]
    pub fade_in_ms: u32,
    #[pult(lifecycle = PERSISTED)]
    pub fade_out_ms: u32,
    /// True when this cue is currently being executed (output is active).
    #[pult(lifecycle = SYNCED)]
    pub is_active: bool,
}
