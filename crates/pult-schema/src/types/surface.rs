//! Surfaces: a page of big buttons and readouts, for running a show from a phone.
//!
//! The workspace is a desk: tiles of panels, each one an instrument for building
//! the show. What a maze runner at the entrance holds is a 6-inch screen and one
//! question at a time — is area 2 empty, reset it, is the light green. A surface
//! is that page, made out of show data: an ordered list of tiles, each pointing at
//! something the show already has (a flow's button, a variable, a timeline, a
//! sequence, one parameter of one fixture). Nothing about a maze is written here;
//! a surface for a theater's stage manager is the same list with other targets.
//!
//! The tiles are one JSON column, the way `Layout::tree` is: a surface is edited
//! as a whole by one person between shows, and reordering a list is rewriting it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    types::{fixture::ParameterKind, variable::VariableOp},
    PultSchema,
};

/// What a tile shows, or does when it is pressed.
///
/// Tagged by `kind` so the browser discriminates on a field, as `LayoutNode` does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum TileTarget {
    /// A label across the page, and nothing else. What makes "Area 1" and
    /// "Area 2" two groups of tiles rather than one list of six.
    Heading,
    /// Press a flow's `Button` node, exactly as its own Press does.
    FlowButton { node_id: Uuid },
    /// What a variable holds: a flag as a light, a number as a figure.
    VariableReadout {
        variable_id: Uuid,
        /// The color for each value, the highest stop not above it winning. Empty
        /// means the tile's own color when the value is on and gray when off.
        stops: Vec<ColorStop>,
    },
    /// Change a variable. Unattributed, through the `variable.*` RPCs, so a phone
    /// resetting an area writes no row in the History panel.
    VariableSetter { variable_id: Uuid, op: VariableOp },
    /// Play or stop a timeline, with whether it is running and where it is.
    Timeline {
        timeline_id: Uuid,
        /// Play from the top rather than from where it stopped: a scare, as
        /// against an ambience that carries on.
        from_start: bool,
    },
    /// Go on a sequence.
    SequenceGo { sequence_id: Uuid },
    /// Take a sequence off.
    SequenceOff { sequence_id: Uuid },
    /// What one parameter of one fixture is putting out now — a traffic light's
    /// color, a relay — evaluated in the page the way the sheet does it.
    Parameter { fixture_id: Uuid, parameter: ParameterKind },
}

/// One color of a readout, from `at` upwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ColorStop {
    pub at: f64,
    /// A CSS color, `#rrggbb`.
    pub color: String,
}

/// How much of a row a tile takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TileSize {
    /// Half a phone's width: two to a row.
    Half,
    /// The whole width.
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SurfaceTile {
    /// Stable across reorders, so a tile being dragged keeps its identity.
    pub id: Uuid,
    /// What the tile says. Empty means the name of what it points at.
    pub label: String,
    /// A CSS color for the tile itself, `#rrggbb`.
    pub color: Option<String>,
    pub size: TileSize,
    pub target: TileTarget,
}

/// One page of tiles.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "surfaces")]
pub struct Surface {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The tiles, in reading order, as one JSON column.
    #[pult(lifecycle = PERSISTED)]
    pub tiles: Vec<SurfaceTile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_round_trips_with_its_kind_as_a_field() {
        let tile = SurfaceTile {
            id: Uuid::nil(),
            label: "Area 2".into(),
            color: None,
            size: TileSize::Half,
            target: TileTarget::VariableReadout {
                variable_id: Uuid::nil(),
                stops: vec![
                    ColorStop { at: 0.0, color: "#22c55e".into() },
                    ColorStop { at: 1.0, color: "#ef4444".into() },
                ],
            },
        };
        let json = serde_json::to_value(&tile).unwrap();
        assert_eq!(json["target"]["kind"], "VariableReadout");
        let back: SurfaceTile = serde_json::from_value(json).unwrap();
        assert_eq!(back, tile);
    }

    #[test]
    fn a_heading_carries_nothing_but_its_label() {
        let json = serde_json::to_value(TileTarget::Heading).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "Heading" }));
    }
}
