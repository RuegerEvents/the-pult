//! Flows: something happening in the rig making something happen in the show,
//! drawn as a graph.
//!
//! The spec's event system and its node-based workflow are the same feature seen
//! twice. A flow is a graph of nodes — a source, a condition, a delay, an action —
//! and a single chain of those four is exactly the one-row-per-rule trigger this
//! replaced. What the graph adds is everything a row could not say: two contacts
//! into an `And`, one delay feeding three actions, a condition reused twice.
//!
//! `TriggerSource` is an enum with one variant so far. OSC, MIDI, and "a cue
//! finished" all belong beside `Parameter`, and each one becomes a node the editor
//! can place without anything here changing shape.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    pult_commands,
    types::fixture::{ParameterKind, ParameterValue},
    PultSchema,
};

// ── What a node watches, decides, and does ────────────────────────────────────

/// What a flow watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TriggerSource {
    /// One parameter of one fixture. A contact on an I/O node, a temperature, or
    /// anything else that lands in `live_values`.
    Parameter { fixture_id: Uuid, parameter: ParameterKind },
}

/// When a condition passes a signal on.
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

/// What a flow does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TriggerAction {
    GoNext { sequence_id: Uuid },
    GoToCue { sequence_id: Uuid, cue_id: Uuid },
    SetParameter { fixture_id: Uuid, parameter: ParameterKind, value: ParameterValue },
}

// ── Nodes ─────────────────────────────────────────────────────────────────────

/// What one node is.
///
/// The three trigger types above are carried whole rather than flattened into
/// variants of their own, so a new `TriggerSource` or `TriggerAction` is a node the
/// moment it exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FlowNodeKind {
    /// Reads a parameter. Emits a level.
    Source(TriggerSource),
    /// Pressed by hand in the editor. Emits a pulse.
    Button,
    /// Watches a level for the change it is looking for. Emits a pulse.
    Condition(TriggerCondition),
    And,
    Or,
    Not,
    /// Holds a pulse back.
    Delay { ms: u32 },
    /// Does something to the show. Consumes a pulse.
    Action(TriggerAction),
}

/// What a port carries.
///
/// A *level* is a value that stays put — a contact that is closed stays closed. A
/// *pulse* is an instant, and nothing about it lingers. Keeping them apart is what
/// stops a graph from asking a level to fire a cue, which is the mistake that makes
/// a warm room retrigger forty times a second.
///
/// The editor refuses to connect one to the other, so an unevaluable graph cannot
/// be drawn in the first place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PortKind {
    Level,
    Pulse,
}

impl FlowNodeKind {
    /// The input ports, in handle order.
    pub fn inputs(&self) -> &'static [PortKind] {
        match self {
            FlowNodeKind::Source(_) | FlowNodeKind::Button => &[],
            FlowNodeKind::Condition(_) | FlowNodeKind::Not => &[PortKind::Level],
            FlowNodeKind::And | FlowNodeKind::Or => &[PortKind::Level, PortKind::Level],
            FlowNodeKind::Delay { .. } | FlowNodeKind::Action(_) => &[PortKind::Pulse],
        }
    }

    /// The output ports, in handle order.
    pub fn outputs(&self) -> &'static [PortKind] {
        match self {
            FlowNodeKind::Source(_) => &[PortKind::Level],
            FlowNodeKind::And | FlowNodeKind::Or | FlowNodeKind::Not => &[PortKind::Level],
            FlowNodeKind::Button | FlowNodeKind::Condition(_) | FlowNodeKind::Delay { .. } => {
                &[PortKind::Pulse]
            }
            FlowNodeKind::Action(_) => &[],
        }
    }
}

// ── Entities ──────────────────────────────────────────────────────────────────

/// One graph.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "flows")]
pub struct Flow {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub enabled: bool,
}

/// One node in one graph.
///
/// Nodes and edges are collections of their own rather than two `Vec`s on `Flow`,
/// so dragging a node patches one row instead of rewriting the graph, and two
/// operators moving two different nodes both keep their work.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "flow_nodes")]
pub struct FlowNode {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub flow_id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub kind: FlowNodeKind,
    /// Where the node sits on the canvas. Not a rig position — this is the drawing.
    #[pult(lifecycle = PERSISTED)]
    pub x: f32,
    #[pult(lifecycle = PERSISTED)]
    pub y: f32,
    /// Lit right now: a level that is true, or a pulse that just passed through.
    ///
    /// SYNCED rather than LOCAL so every console watching the same flow sees the
    /// same thing light up. Without it a graph is a diagram; with it, it is an
    /// instrument you can watch working.
    #[pult(lifecycle = SYNCED)]
    pub active: bool,
    #[pult(lifecycle = SYNCED)]
    pub last_fired_at: Option<DateTime<Utc>>,
}

/// One connection, from an output port to an input port.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "flow_edges")]
pub struct FlowEdge {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub flow_id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub from_node: Uuid,
    /// Which output handle of `from_node`, indexing [`FlowNodeKind::outputs`].
    #[pult(lifecycle = PERSISTED)]
    pub from_port: u8,
    #[pult(lifecycle = PERSISTED)]
    pub to_node: Uuid,
    /// Which input handle of `to_node`, indexing [`FlowNodeKind::inputs`].
    #[pult(lifecycle = PERSISTED)]
    pub to_port: u8,
}

#[pult_commands]
impl FlowNode {
    /// Press a button node.
    ///
    /// The press is this timestamp changing, and nothing else. That makes it an
    /// ordinary replicated field write, so a button pressed on a tablet reaches the
    /// leader — the only node that fires anything — by the path every other change
    /// already takes.
    #[pult_command]
    pub fn press(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.last_fired_at = Some(Utc::now());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_of_four_nodes_has_matching_ports() {
        let source = FlowNodeKind::Source(TriggerSource::Parameter {
            fixture_id: Uuid::nil(),
            parameter: ParameterKind::Contact(0),
        });
        let condition = FlowNodeKind::Condition(TriggerCondition::RisingEdge);
        let delay = FlowNodeKind::Delay { ms: 500 };
        let action = FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: Uuid::nil() });

        // Exactly the shape every migrated trigger takes.
        assert_eq!(source.outputs(), condition.inputs());
        assert_eq!(condition.outputs(), delay.inputs());
        assert_eq!(delay.outputs(), action.inputs());
        assert!(action.outputs().is_empty(), "an action is the end of the line");
    }

    #[test]
    fn a_level_cannot_be_asked_to_fire_a_cue() {
        let source = FlowNodeKind::Source(TriggerSource::Parameter {
            fixture_id: Uuid::nil(),
            parameter: ParameterKind::Contact(0),
        });
        let action = FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: Uuid::nil() });

        assert_ne!(
            source.outputs()[0],
            action.inputs()[0],
            "a source emits a level and an action wants a pulse: a condition has to sit between them"
        );
    }

    #[test]
    fn and_takes_two_levels_and_gives_one_back() {
        assert_eq!(FlowNodeKind::And.inputs(), &[PortKind::Level, PortKind::Level]);
        assert_eq!(FlowNodeKind::And.outputs(), &[PortKind::Level]);
    }
}
