//! The programmer: the scratch buffer an operator works in before anything is stored.
//!
//! A console has two sources of truth about what a light is doing. Playback says what
//! the cue asks for; the programmer says what the operator is asking for *right now*,
//! and the programmer wins. Nothing is written to the show until it is stored, and
//! clearing puts every touched parameter back where playback had it.
//!
//! # Why this is a collection and not a field on anything
//!
//! One value one operator is holding is one row. That makes two consoles working the
//! same rig converge without arbitration, the same way [`crate::types::station`] does
//! — as long as they agree on the id. They do: the frontend derives the id from the
//! fixture and the parameter key rather than minting a fresh one, so two people
//! grabbing the same fader write the same row instead of two rows that fight.
//!
//! SYNCED rather than PERSISTED. A programmer buffer is what is in the operator's
//! hands, not what is in the show; a showfile that reopened with somebody's
//! half-finished look asserted over playback would be a fault, not a feature.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    types::effect::EffectSpec,
    types::fixture::{ParameterKind, ParameterValue},
    PultSchema,
};

/// One parameter of one fixture, held by the programmer.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "programmer_values")]
pub struct ProgrammerValue {
    /// Derived from `fixture_id` and the parameter key rather than minted, so two
    /// consoles writing the same parameter converge on one entry.
    #[pult(lifecycle = SYNCED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = SYNCED)]
    pub fixture_id: Uuid,
    #[pult(lifecycle = SYNCED)]
    pub parameter_kind: ParameterKind,
    #[pult(lifecycle = SYNCED)]
    pub value: ParameterValue,
    /// A shape held instead of a value.
    ///
    /// An entry asserts either its value or its effect for the key, never both, so
    /// the id derivation is unchanged: grabbing a fader and putting a sine on it are
    /// the same act of taking hold of one parameter.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub effect: Option<EffectSpec>,
    /// Parked: survives Clear and Store, so one value can go into several cues.
    ///
    /// The spec calls this the parking function and asks for it explicitly — a value
    /// held "without saving, to be saved in multiple sequences without the need of a
    /// store menu".
    #[pult(lifecycle = SYNCED)]
    pub locked: bool,
}
