//! A control surface on a desk: what one is doing, and what its controls are bound to.
//!
//! Two lifecycles, and they must not be confused. Which surface is plugged into which
//! station, and which page is driving it, is a fact about one machine and is LOCAL —
//! opening the show on a laptop must not claim a wing that is in another building. What
//! a control is *bound to* is the show's and PERSISTED, so a spare console and the
//! showfile carry it.
//!
//! The first surface is a grandMA3 onPC command wing; the binding is named by surface
//! kind and control rather than by anything about that wing, so a MIDI box later is
//! another `surface` string and not another collection.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    types::{fixture::ParameterKind, surface::TileTarget},
    PultSchema,
};

/// The surface kind a grandMA3 onPC command wing binds under.
pub const COMMAND_WING: &str = "ma3-command-wing";

// ── What the wing is doing (LOCAL) ────────────────────────────────────────────

/// Where the wing on this station has got to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum WingPhase {
    /// This station was started without a wing, which is what every test is.
    #[default]
    Off,
    /// Looking, and nothing is plugged in.
    Absent,
    /// It is on the bus and the boot conversation is under way.
    Booting,
    /// Taking its application: `sent` of `total` bytes.
    Loading { sent: u32, total: u32 },
    /// Up, and reporting what is pressed.
    Running,
    /// It cannot be run here, and why — no grandMA3 install to find the image in, or
    /// another program holding the interface.
    Fault { reason: String },
}

/// The LOCAL `wing` row: what the wing on *this* station is doing, and who drives it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingState {
    pub phase: WingPhase,
    /// What it said it is, once it has said.
    pub device: Option<String>,
    /// The short session id of the page driving it, or `None` when no page is.
    ///
    /// A wing acts on one operator's selection, command line and gestures, and those
    /// live in a page — so a page claims it, and with none claiming it the wing is
    /// dark and does nothing.
    pub claimed_by: Option<String>,
}

// ── What its controls do (PERSISTED) ──────────────────────────────────────────

/// One physical control, as the surface numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum SurfaceControl {
    /// A key, by the index the surface reports it under.
    Key { index: u16 },
    Fader { index: u16 },
    Encoder { index: u16 },
}

/// What a bound control does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum BindingAction {
    /// A key presses something the show already has, exactly as a phone surface's
    /// tile presses it — so a binding adds no acts of its own.
    Press { target: TileTarget },
    /// A key types this line into the command line and runs it.
    CommandLine { line: String },
    /// A key puts one parameter at full while it is held, and lets go on release.
    Flash { fixture_id: Uuid, parameter: ParameterKind },
    /// A key recalls a saved group into the driving page's selection.
    SelectGroup { group_id: Uuid },
    /// A fader or an encoder moves one parameter of one fixture, in the programmer.
    Parameter { fixture_id: Uuid, parameter: ParameterKind },
    /// A fader is a sequence's level; its executor keys are that sequence's Go and Off.
    Master { sequence_id: Uuid },
}

/// One control, bound.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "control_bindings")]
pub struct ControlBinding {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// Which kind of surface: [`COMMAND_WING`].
    #[pult(lifecycle = PERSISTED)]
    pub surface: String,
    #[pult(lifecycle = PERSISTED)]
    pub control: SurfaceControl,
    #[pult(lifecycle = PERSISTED)]
    pub action: BindingAction,
    /// What the panel calls it. Empty means the name of what it is bound to.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_round_trips_with_its_kinds_as_fields() {
        let binding = ControlBinding {
            id: Uuid::nil(),
            surface: COMMAND_WING.into(),
            control: SurfaceControl::Fader { index: 3 },
            action: BindingAction::Master { sequence_id: Uuid::nil() },
            label: String::new(),
        };
        let json = serde_json::to_value(&binding).unwrap();
        assert_eq!(json["control"], serde_json::json!({ "kind": "Fader", "index": 3 }));
        assert_eq!(json["action"]["kind"], "Master");
        let back: ControlBinding = serde_json::from_value(json).unwrap();
        assert_eq!(back.control, binding.control);
    }
}
