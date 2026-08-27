//! Workspace layouts: how the panels of the console are arranged on screen.
//!
//! A programming session needs the rig, the values and the cue list visible at once,
//! which a fixed sidebar and a row of tabs can never give. So the workspace is a tree
//! of splits and tab groups, and the tree is show data: an operator who arranged the
//! screen for tech week should find it that way at the next call, on whichever console
//! they sit down at.
//!
//! # What is here and what is not
//!
//! Panel ids are plain strings. The schema knows a layout holds panels; it does not
//! know that one of them is called `rig` and draws a stage in three dimensions. That
//! keeps the frontend's list of panels a frontend concern, and means adding one needs
//! nothing here.
//!
//! Which layout a browser is *looking at*, and any rearranging not yet saved, are not
//! here either. Those are one operator's, not the show's, and live in a store beside
//! the selection.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// Which way a split divides its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SplitDirection {
    /// Children side by side, divided by vertical gutters.
    Row,
    /// Children stacked, divided by horizontal gutters.
    Column,
}

/// One node of a workspace tree: either a division of the space, or panels in it.
///
/// Tagged by `type` so the TypeScript side discriminates on a field rather than on
/// the shape of an object — the layout operations in the frontend are written against
/// that tag, and a bare externally-tagged enum would make every one of them a
/// two-key lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type")]
pub enum LayoutNode {
    /// Space divided among children, each taking its share of `sizes`.
    Split {
        direction: SplitDirection,
        /// One fraction per child, summing to 1.
        sizes: Vec<f32>,
        children: Vec<LayoutNode>,
    },
    /// Panels sharing one rectangle, one of them on top.
    Tabs {
        /// Panel ids, in tab order.
        panels: Vec<String>,
        /// Which of them is showing.
        active: usize,
    },
}

impl LayoutNode {
    /// Every panel this tree holds, in the order it holds them.
    pub fn panels(&self) -> Vec<&str> {
        match self {
            LayoutNode::Split { children, .. } => {
                children.iter().flat_map(|c| c.panels()).collect()
            }
            LayoutNode::Tabs { panels, .. } => panels.iter().map(String::as_str).collect(),
        }
    }
}

/// One saved arrangement of the workspace.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "layouts")]
pub struct Layout {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The whole arrangement, as one JSON column.
    #[pult(lifecycle = PERSISTED)]
    pub tree: LayoutNode,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_tree() -> LayoutNode {
        LayoutNode::Split {
            direction: SplitDirection::Row,
            sizes: vec![0.6, 0.4],
            children: vec![
                LayoutNode::Tabs { panels: vec!["rig".into()], active: 0 },
                LayoutNode::Tabs {
                    panels: vec!["values".into(), "selection".into()],
                    active: 1,
                },
            ],
        }
    }

    #[test]
    fn a_tree_lists_its_panels_left_to_right() {
        assert_eq!(a_tree().panels(), ["rig", "values", "selection"]);
    }

    #[test]
    fn a_tree_round_trips_through_json() {
        let json = serde_json::to_value(a_tree()).unwrap();
        assert_eq!(json["type"], "Split", "the tag is a field, not a wrapping key");
        let back: LayoutNode = serde_json::from_value(json).unwrap();
        assert_eq!(back, a_tree());
    }
}
