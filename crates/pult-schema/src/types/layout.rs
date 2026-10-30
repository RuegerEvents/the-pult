//! Layouts: a drawing of somewhere, with what is happening there painted on it.
//!
//! The rig view answers "where is everything"; a layout answers "where is it
//! happening". A maze drawn as a few rectangles with every PIR and contact on it,
//! each lit while somebody is standing on it and fading for half a minute after
//! they have gone, so a glance says where the group is and which way it went. A
//! timeline as a bar beside the room it plays in. Nothing about a maze is written
//! here: a theater's stage door contact and its backstage temperatures are the same
//! drawing with other targets — and like MA's 2D layout views, a show has as many as
//! it has places worth looking at.
//!
//! **A layout's coordinates are its own.** An element sits at an `x`, `y`, `w`, `h`
//! on a canvas of abstract units, not at the fixture's position in the room: a
//! sensor needs no 3D placement to be on a map, the same contact can sit on two
//! layouts in two places, and a schematic is allowed to be a schematic. The canvas
//! scales to whatever screen it is drawn on.
//!
//! **A layout shows; on the desk it may also press.** An element's `press` is the
//! surface's idea — toggle a variable, play a timeline — and it is offered only on
//! the desk. `/layout/<id>`, the page a phone is handed, draws the same layout and
//! presses nothing.
//!
//! The elements are one JSON column, the way a surface's tiles are: a layout is
//! drawn as a whole by one person between shows.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    types::{fixture::ParameterKind, surface::ColorStop, variable::VariableOp},
    PultSchema,
};

/// How long a contact stays warm after it lets go, when nobody said.
pub const DEFAULT_DECAY_MS: u32 = 30_000;
/// The color of a contact somebody is on, when nobody said.
pub const DEFAULT_ACTIVE_COLOR: &str = "#ff9f1c";
/// The color of one nobody has been near, when nobody said.
pub const DEFAULT_IDLE_COLOR: &str = "#2b313a";

/// What an element is a drawing of.
///
/// Tagged by `kind`, as `TileTarget` is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum ElementTarget {
    /// One parameter of one fixture: what a device reported (a contact, a PIR, a
    /// temperature) or what the console is putting out (a traffic light's color).
    /// An on/off reading decays; a number is colored by `stops` and printed.
    Parameter { fixture_id: Uuid, parameter: ParameterKind },
    /// What a variable holds: a flag decays like a contact, a number is colored by
    /// `stops` and printed.
    Variable { variable_id: Uuid },
    /// A timeline: running or not, how far through, and how long is left.
    Timeline { timeline_id: Uuid },
    /// A filled shape and nothing else — a room, a wall, a stage.
    Shape,
    /// A line through points, in canvas units, absolute rather than inside the
    /// element's box: a wall that turns a corner. The box is the points' bounds.
    Line { points: Vec<[f32; 2]> },
    /// Words on the drawing, and nothing behind them.
    Text,
}

/// The outline an element is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub enum ElementShape {
    #[default]
    Rect,
    Ellipse,
}

/// What pressing an element does, on the desk. Never on the shared page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum ElementPress {
    /// Change the element's variable, through the `variable.*` RPCs, unattributed —
    /// exactly as a surface tile does.
    Variable { op: VariableOp },
    /// Play the element's timeline if it is stopped, stop it if it is running.
    Timeline {
        /// Play from the top rather than from where it stopped.
        from_start: bool,
    },
    /// Make the element's fixture the selection.
    Select,
}

/// How a reading that goes on and off is drawn: lit while on, then fading.
///
/// Every field is optional on an element, which falls back to its layout's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct Decay {
    /// `#rrggbb` while on, and the color the fade starts from.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub active_color: Option<String>,
    /// `#rrggbb` once the fade has finished, and for a reading that has never been on.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub idle_color: Option<String>,
    /// How long the fade from active to idle takes, from the moment the reading went
    /// off. Zero is a light that goes straight out.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub decay_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LayoutElement {
    /// Stable across edits, so an element being dragged keeps its identity.
    pub id: Uuid,
    /// Where it is, in canvas units from the top left.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub shape: ElementShape,
    /// What it says. Empty means the name of what it points at.
    #[serde(default)]
    pub label: String,
    /// Whether the label is drawn at all.
    #[serde(default = "yes")]
    pub show_label: bool,
    /// The fill of a `Shape`, the stroke of a `Line`, the ink of `Text`; ignored by
    /// an element whose color comes from what it is showing.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub color: Option<String>,
    pub target: ElementTarget,
    /// This element's own answer to how it decays, over the layout's.
    #[serde(default)]
    pub decay: Decay,
    /// The color for each value of a number, the highest stop not above it winning.
    /// Empty means the layout's idle color with the figure printed on it.
    #[serde(default)]
    pub stops: Vec<ColorStop>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub press: Option<ElementPress>,
}

fn yes() -> bool {
    true
}

/// One drawing.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "layouts")]
pub struct Layout {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The canvas, in its own units. The drawing keeps this aspect on any screen.
    #[pult(lifecycle = PERSISTED)]
    pub width: f32,
    #[pult(lifecycle = PERSISTED)]
    pub height: f32,
    /// What a dragged element snaps to, in canvas units. Zero is no snapping.
    #[pult(lifecycle = PERSISTED)]
    pub grid: f32,
    /// A drawing somebody was handed, stretched over the canvas: the asset's sha256.
    #[serde(default)]
    #[ts(optional = nullable)]
    #[pult(lifecycle = PERSISTED)]
    pub backdrop: Option<String>,
    /// The layout's answer to how a reading decays; an element may say otherwise.
    #[pult(lifecycle = PERSISTED)]
    pub decay: Decay,
    /// The elements, back to front, as one JSON column.
    #[pult(lifecycle = PERSISTED)]
    pub elements: Vec<LayoutElement>,
    /// The card's color, as `#rrggbb`, or `None` for its panel category's color.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub color: Option<String>,
    /// The pool folder it is in, or `None` for the top level. See [`crate::types::folder`].
    #[serde(default)]
    #[ts(optional = nullable)]
    #[pult(lifecycle = PERSISTED)]
    pub folder: Option<Uuid>,
}

impl Decay {
    /// This answer, with anything it leaves out taken from `under`, and the
    /// defaults under both.
    pub fn over(&self, under: &Decay) -> (String, String, u32) {
        (
            self.active_color
                .clone()
                .or_else(|| under.active_color.clone())
                .unwrap_or_else(|| DEFAULT_ACTIVE_COLOR.into()),
            self.idle_color
                .clone()
                .or_else(|| under.idle_color.clone())
                .unwrap_or_else(|| DEFAULT_IDLE_COLOR.into()),
            self.decay_ms.or(under.decay_ms).unwrap_or(DEFAULT_DECAY_MS),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_round_trips_with_its_target_as_a_field() {
        let element = LayoutElement {
            id: Uuid::nil(),
            x: 10.0,
            y: 20.0,
            w: 40.0,
            h: 40.0,
            shape: ElementShape::Ellipse,
            label: String::new(),
            show_label: true,
            color: None,
            target: ElementTarget::Variable { variable_id: Uuid::nil() },
            decay: Decay::default(),
            stops: Vec::new(),
            press: Some(ElementPress::Timeline { from_start: true }),
        };
        let json = serde_json::to_value(&element).unwrap();
        assert_eq!(json["target"]["kind"], "Variable");
        assert_eq!(json["press"]["kind"], "Timeline");
        let back: LayoutElement = serde_json::from_value(json).unwrap();
        assert_eq!(back, element);
    }

    #[test]
    fn an_element_written_with_only_the_essentials_reads() {
        let element: LayoutElement = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(), "x": 0, "y": 0, "w": 1, "h": 1,
            "target": { "kind": "Text" }
        }))
        .unwrap();
        assert!(element.show_label, "a label is drawn unless somebody said not to");
        assert_eq!(element.shape, ElementShape::Rect);
    }

    #[test]
    fn an_element_says_what_it_says_and_the_layout_the_rest() {
        let layout = Decay { active_color: Some("#ff0000".into()), idle_color: None, decay_ms: Some(5000) };
        let element = Decay { decay_ms: Some(1000), ..Decay::default() };
        assert_eq!(element.over(&layout), ("#ff0000".into(), DEFAULT_IDLE_COLOR.into(), 1000));
        assert_eq!(
            Decay::default().over(&Decay::default()),
            (DEFAULT_ACTIVE_COLOR.into(), DEFAULT_IDLE_COLOR.into(), DEFAULT_DECAY_MS)
        );
    }
}
