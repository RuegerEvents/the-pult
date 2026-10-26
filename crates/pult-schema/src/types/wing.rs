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
    types::{
        fixture::{ParameterKind, ParameterValue},
        group::{CellSpan, SelectionQuery},
        surface::TileTarget,
    },
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

/// The LOCAL `wing` row: what the wing on *this* station is doing, and who shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingState {
    pub phase: WingPhase,
    /// What it said it is, once it has said.
    pub device: Option<String>,
    /// The short session id of the page showing it, or `None` when no page is.
    ///
    /// The wing works with no page at all: the station runs every control. A page
    /// that claims it is the one that shares its selection with the wing, shows its
    /// command line and encoders, and answers what needs a screen — the Store
    /// dialog, a learned control waiting to be bound.
    pub claimed_by: Option<String>,
    /// The seven digital inputs as the wing last reported them, one bit each, or
    /// `None` before it has. A level rather than a press, so where they rest can be
    /// read before any of them is bound.
    #[serde(default)]
    pub digital: Option<u8>,
    /// What the wing's operator has in hand, kept by the station.
    #[serde(default)]
    pub operator: WingOperator,
}

/// The wing's half of an operator: its command line, its encoders, the calculator,
/// learning, and the selection all of them act on.
///
/// The station's, so the wing does the same thing whether or not a page is open
/// (found at the gig: a page drawing the rig lagged every key behind it). A page that
/// claims the wing reads this to show it, and shares its selection with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingOperator {
    /// What the keypad has typed and PLEASE has not yet run.
    pub line: String,
    /// The calculator a large encoder's press opens, while one is.
    pub calculator: Option<WingCalculator>,
    /// The selection's attributes as the five encoders page through them.
    pub encoders: WingEncoders,
    /// Waiting for the next control touched, to bind it.
    pub learning: bool,
    pub selection: WingSelection,
    /// The sequence GO and GO BACK act on: the claiming page's cue sheet touched last,
    /// or the sequence that went most recently. `None` when there is none.
    pub go_sequence: Option<Uuid>,
    /// The page of bindings the controls act on, from 1: PAGE_UP and PAGE_DOWN turn it.
    /// LOCAL like the rest of this, so a restarted station is back on page 1.
    #[serde(default = "first_page")]
    pub page: u32,
}

fn first_page() -> u32 {
    1
}

impl Default for WingOperator {
    fn default() -> Self {
        WingOperator {
            line: String::new(),
            calculator: None,
            encoders: WingEncoders::default(),
            learning: false,
            selection: WingSelection::default(),
            go_sequence: None,
            page: first_page(),
        }
    }
}

/// One encoder's calculator: which of the five, and what has been typed into it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingCalculator {
    pub slot: u8,
    pub text: String,
}

/// The encoder bar: five slots of the selection's attributes, one page of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingEncoders {
    pub page: u32,
    pub pages: u32,
    /// Always five; `None` past the end of what the selection has.
    pub slots: Vec<Option<WingSlot>>,
}

impl Default for WingEncoders {
    fn default() -> Self {
        WingEncoders { page: 0, pages: 1, slots: vec![None; 5] }
    }
}

/// One position on the encoder bar: a parameter, or one channel of a color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingSlot {
    pub kind: ParameterKind,
    pub component: Option<ColorComponent>,
    pub default_value: ParameterValue,
}

/// One channel of a color, which is what a knob can turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ColorComponent {
    R,
    G,
    B,
}

/// What the wing has selected: the claiming page's selection, or one made on the wing.
///
/// `rev` counts changes, and `from_wing` says the last one was made on the wing — a
/// group key, a typed `fixture 3` — which is the one kind a page takes on as its own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WingSelection {
    pub fixtures: Vec<Uuid>,
    #[serde(default)]
    pub cells: Vec<CellSpan>,
    /// The question, where the selection is one: a group recalled keeps following the
    /// rig on the page that takes it on.
    pub query: Option<SelectionQuery>,
    pub rev: u32,
    pub from_wing: bool,
}

/// What the claiming page tells the station about itself: its selection, its cue
/// sheet, whether a dialog is up, and who is at it. `wing.context`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WingContext {
    pub fixtures: Vec<Uuid>,
    #[serde(default)]
    pub cells: Vec<CellSpan>,
    pub query: Option<SelectionQuery>,
    /// The wing selection's `rev` the page had seen when it said this. A page that had
    /// not yet seen a selection made on the wing does not get to overwrite it.
    pub seen_rev: u32,
    pub focused_sequence: Option<Uuid>,
    /// A dialog is open on the page, so ESC and PLEASE answer it.
    pub dialog: bool,
    pub user_id: Option<Uuid>,
}

/// What the station asks the claiming page to do, on the `wing_asks` path: the parts
/// of a wing's act that need a screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum WingAsk {
    /// Say something: what a command line answered, why a key did nothing.
    Toast { text: String, good: bool },
    /// Learning caught this control; the page binds it to what is touched next.
    Learned { control: SurfaceControl },
    /// Open the Store dialog, with these programmer entries ticked (all when empty),
    /// and onto a playback control where STORE was followed by one.
    Store {
        preselect: Vec<Uuid>,
        #[serde(default)]
        onto: Option<StoreOnto>,
    },
    /// ESC or PLEASE, for the dialog on top.
    Dialog { accept: bool },
}

/// STORE followed by a playback control: a fader's executor key, or an X key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoreOnto {
    /// The control that is the playback: the fader for an executor key, else the key.
    pub control: SurfaceControl,
    /// The page that control was on.
    pub page: u32,
    /// The sequence it plays, or `None` for a control nothing is bound to yet, which
    /// the store makes a new sequence for and binds to it.
    pub sequence_id: Option<Uuid>,
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
    /// One of the wing's seven digital inputs, 0 to 6, on its rear panel. Read as
    /// pressed while its bit is set, which no closed contact has confirmed yet.
    Digital { index: u16 },
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
    /// A key recalls a saved group into the wing's selection.
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
    /// The page it acts on, from 1. `None` is page 1, which is where every binding made
    /// before there were pages is: optional because a required column added to a show
    /// that has rows is a column nothing filled, and such a show refuses to open.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub page: Option<u32>,
}

impl ControlBinding {
    /// The page it acts on, from 1.
    pub fn page(&self) -> u32 {
        self.page.unwrap_or(1)
    }
}

/// A page of bindings, named. A page with bindings and no row is just its number, so
/// only a page somebody has named has one.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "control_pages")]
pub struct ControlPage {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// Which kind of surface: [`COMMAND_WING`].
    #[pult(lifecycle = PERSISTED)]
    pub surface: String,
    /// The page, from 1.
    #[pult(lifecycle = PERSISTED)]
    pub number: u32,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
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
            page: None,
        };
        let json = serde_json::to_value(&binding).unwrap();
        assert_eq!(json["control"], serde_json::json!({ "kind": "Fader", "index": 3 }));
        assert_eq!(json["action"]["kind"], "Master");
        let back: ControlBinding = serde_json::from_value(json).unwrap();
        assert_eq!(back.control, binding.control);
    }
}
