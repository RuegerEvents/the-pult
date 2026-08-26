//! Triggers: something happening in the rig making something happen in the show.
//!
//! The spec's event system, in its first and flattest form — one row per rule, a
//! source, a condition, an action, and a delay. The node graph it eventually wants
//! is a different way of *drawing* these, not a different thing.
//!
//! `TriggerSource` is an enum with one variant so far. OSC, MIDI, and "a cue
//! finished" all belong beside `Parameter` and none of them changes the shape of
//! the rest.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{types::fixture::{ParameterKind, ParameterValue}, PultSchema};

/// What a trigger watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TriggerSource {
    /// One parameter of one fixture. A contact on an I/O node, a temperature, or
    /// anything else that lands in `live_values`.
    Parameter { fixture_id: Uuid, parameter: ParameterKind },
}

/// When a trigger fires.
///
/// Every one of these is about a *change*: a level that is already above the
/// threshold does not fire again on the next reading, or a warm room would fire a
/// cue forty times a second.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TriggerCondition {
    RisingEdge,
    FallingEdge,
    AnyChange,
    Above(f32),
    Below(f32),
}

/// What a trigger does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TriggerAction {
    GoNext { sequence_id: Uuid },
    GoToCue { sequence_id: Uuid, cue_id: Uuid },
    SetParameter { fixture_id: Uuid, parameter: ParameterKind, value: ParameterValue },
}

/// One rule: watch this, and when it does that, do this.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "triggers")]
pub struct Trigger {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub source: TriggerSource,
    #[pult(lifecycle = PERSISTED)]
    pub condition: TriggerCondition,
    #[pult(lifecycle = PERSISTED)]
    pub action: TriggerAction,
    /// Wait this long after the condition before acting.
    #[pult(lifecycle = PERSISTED)]
    pub delay_ms: u32,
    #[pult(lifecycle = PERSISTED)]
    pub enabled: bool,
    /// The condition has been met and the delay is still running.
    #[pult(lifecycle = SYNCED)]
    pub pending: bool,
    #[pult(lifecycle = SYNCED)]
    pub last_fired_at: Option<DateTime<Utc>>,
}
