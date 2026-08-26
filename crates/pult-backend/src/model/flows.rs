//! Flow evaluation: pure, like playback, and for the same reason.
//!
//! [`Flows::tick`] takes the graph, the inputs that arrived since the last tick,
//! and a timestamp, and returns what to do. It reads no clock and touches no
//! engine, so a test can run a ten-second delay in microseconds.
//!
//! Inputs are handed in as a list rather than read from the current state, because
//! a button pressed and released between two ticks would otherwise look like
//! nothing happening at all.
//!
//! # Levels and pulses
//!
//! A *level* is a value that stays put: a contact that is closed stays closed until
//! something opens it. A *pulse* is an instant with no duration. Sources emit
//! levels, `And`/`Or`/`Not` combine them, and a `Condition` is the one thing that
//! turns a level into a pulse — by noticing a *change* in it. That asymmetry is the
//! whole reason a warm room does not fire a cue forty times a second, and keeping
//! the two kinds apart in the type system is what stops a graph from being drawn
//! that would.
//!
//! Levels are read through [`Flows::levels`], which this module maintains itself
//! from the input events. An `And` therefore reads the current value of a source
//! that did not change this tick without reaching into engine state, and a source
//! nothing has ever reported reads as false — which is the honest answer on a cold
//! start rather than a guess.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Utc};

use pult_schema::types::{
    fixture::ParameterValue,
    flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
};
use uuid::Uuid;

/// One parameter changing, as the engine saw it happen.
#[derive(Debug, Clone, PartialEq)]
pub struct InputEvent {
    pub fixture_id: Uuid,
    /// The `live_values` key, as [`crate::model::playback::parameter_key`] writes it.
    pub key: String,
    /// What was there before, if anything ever was.
    pub previous: Option<ParameterValue>,
    pub current: ParameterValue,
}

/// What a flow tick asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowEffect {
    /// Carry out an action node's action, and mark it as having fired.
    Fire { node_id: Uuid, action: TriggerAction },
    /// A node lit up or went dark.
    SetActive { node_id: Uuid, active: bool },
}

/// The graph as the engine reads it out of show state.
pub struct FlowGraph<'a> {
    pub flows: &'a [Flow],
    pub nodes: &'a [FlowNode],
    pub edges: &'a [FlowEdge],
}

impl FlowGraph<'_> {
    fn node(&self, id: Uuid) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Is this node in a flow that is switched on? A node whose flow was deleted
    /// answers no, so an orphan cannot fire.
    fn live(&self, node: &FlowNode) -> bool {
        self.flows.iter().any(|f| f.id == node.flow_id && f.enabled)
    }

    /// Everything reachable from one output port.
    fn downstream(&self, node_id: Uuid, port: u8) -> impl Iterator<Item = &FlowEdge> {
        self.edges.iter().filter(move |e| e.from_node == node_id && e.from_port == port)
    }

    /// What feeds one input port. Several edges into one port is an implicit *or*:
    /// any pulse arriving fires it, and any true level makes it true.
    fn upstream(&self, node_id: Uuid, port: u8) -> impl Iterator<Item = &FlowEdge> {
        self.edges.iter().filter(move |e| e.to_node == node_id && e.to_port == port)
    }
}

#[derive(Default)]
pub struct Flows {
    /// Delay nodes whose pulse is held, and when it is due.
    pending: Vec<(Uuid, Instant)>,
    /// The last value seen for each parameter, keyed as the input events name it.
    levels: HashMap<(Uuid, String), ParameterValue>,
    /// Which nodes are currently reported as lit, so the engine is only told about
    /// changes rather than the same truth forty times a second.
    lit: HashSet<Uuid>,
    /// The `last_fired_at` last seen on each button.
    ///
    /// A press is a write to that replicated field, so any console can press a
    /// button and the leader — the only node that fires anything — sees it arrive
    /// like any other change. A button seen for the first time is recorded and not
    /// fired, or a follower joining a running show would set every button off.
    buttons: HashMap<Uuid, Option<DateTime<Utc>>>,
}

impl Flows {
    /// Is there anything to do even with no input? A delay still has to expire.
    pub fn has_work(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn tick(
        &mut self,
        now: Instant,
        graph: &FlowGraph,
        inputs: &[InputEvent],
    ) -> Vec<FlowEffect> {
        let mut effects = Vec::new();

        // A node deleted, or a flow switched off, while a delay was running does not
        // go off afterwards. Dropped silently: there is nothing left to mark as done.
        self.pending.retain(|(id, _)| {
            graph.node(*id).map(|n| graph.live(n)).unwrap_or(false)
        });

        // Nodes lit this tick. A pulse has no duration, so what "lit" means for a
        // pulse node is "something went through it just now"; the next tick with
        // nothing going through clears it.
        let mut lit: HashSet<Uuid> = HashSet::new();

        for node in graph.nodes.iter().filter(|n| matches!(n.kind, FlowNodeKind::Button)) {
            let seen_before = self.buttons.insert(node.id, node.last_fired_at);
            if !graph.live(node) {
                continue;
            }
            // `Some(previous)` means this button was already known, so a different
            // stamp is a press. `None` means it has only just come into view.
            if seen_before.is_some_and(|previous| previous != node.last_fired_at) {
                lit.insert(node.id);
                self.pulse_downstream(now, graph, node.id, &mut lit, &mut effects);
            }
        }
        self.buttons.retain(|id, _| graph.node(*id).is_some());

        for input in inputs {
            let key = (input.fixture_id, input.key.clone());
            // The engine's own before-and-after, not this module's cached level: a
            // contact pressed and released between two ticks arrives as two events,
            // and only the reported `previous` still knows that happened.
            let previous = input.previous.clone();
            self.levels.insert(key.clone(), input.current.clone());

            // Only conditions downstream of a source that watches *this* parameter
            // can have changed. Everything else still reads the same level it did.
            for node in graph.nodes.iter().filter(|n| graph.live(n)) {
                let FlowNodeKind::Source(source) = &node.kind else { continue };
                if !watches(source, input) {
                    continue;
                }
                if as_bool(Some(&input.current)).unwrap_or(false) {
                    lit.insert(node.id);
                }
                self.propagate_level(
                    now,
                    graph,
                    node.id,
                    &key,
                    previous.as_ref(),
                    &input.current,
                    &mut lit,
                    &mut effects,
                );
            }
        }

        // A delay that has come due releases its pulse, however quiet the rig is.
        let (due, waiting): (Vec<_>, Vec<_>) = self.pending.iter().partition(|(_, at)| *at <= now);
        self.pending = waiting;
        for (node_id, _) in due {
            lit.insert(node_id);
            self.pulse_downstream(now, graph, node_id, &mut lit, &mut effects);
        }

        // A pending delay stays lit while it counts down, so the graph shows the
        // wait rather than going dark between the condition and the cue.
        for (node_id, _) in &self.pending {
            lit.insert(*node_id);
        }

        // Levels that are still true keep their nodes lit even on a tick where
        // nothing changed, which is what makes a closed contact look closed.
        for node in graph.nodes.iter().filter(|n| graph.live(n)) {
            if matches!(node.kind, FlowNodeKind::Delay { .. }) {
                continue;
            }
            if self.level_of(graph, node.id, &mut HashSet::new()) == Some(true) {
                lit.insert(node.id);
            }
        }

        for node_id in lit.difference(&self.lit) {
            effects.push(FlowEffect::SetActive { node_id: *node_id, active: true });
        }
        for node_id in self.lit.difference(&lit) {
            effects.push(FlowEffect::SetActive { node_id: *node_id, active: false });
        }
        self.lit = lit;

        effects
    }

    /// A source's level changed. Walk the level edges out of it, and fire any
    /// condition that was waiting for exactly this change.
    fn propagate_level(
        &mut self,
        now: Instant,
        graph: &FlowGraph,
        from: Uuid,
        changed: &(Uuid, String),
        previous: Option<&ParameterValue>,
        current: &ParameterValue,
        lit: &mut HashSet<Uuid>,
        effects: &mut Vec<FlowEffect>,
    ) {
        let mut seen = HashSet::new();
        let mut queue = vec![from];
        // Conditions found downstream of the change, deduplicated: a diamond in the
        // graph must not fire the same condition twice on one input.
        let mut conditions: Vec<Uuid> = Vec::new();

        while let Some(node_id) = queue.pop() {
            if !seen.insert(node_id) {
                continue;
            }
            for edge in graph.downstream(node_id, 0) {
                let Some(target) = graph.node(edge.to_node) else { continue };
                if !graph.live(target) {
                    continue;
                }
                match target.kind {
                    FlowNodeKind::Condition(_) => {
                        if !conditions.contains(&target.id) {
                            conditions.push(target.id);
                        }
                    }
                    // Logic nodes pass a level on, so a change behind one still
                    // reaches the condition in front of it.
                    FlowNodeKind::And | FlowNodeKind::Or | FlowNodeKind::Not => {
                        queue.push(target.id);
                    }
                    _ => {}
                }
            }
        }

        for condition_id in conditions {
            let Some(node) = graph.node(condition_id) else { continue };
            let FlowNodeKind::Condition(condition) = node.kind else { continue };

            let Some((before, after)) =
                self.gate_values(graph, condition_id, changed, previous, current)
            else {
                continue;
            };

            if fires(condition, before.as_ref(), &after) {
                lit.insert(condition_id);
                self.pulse_downstream(now, graph, condition_id, lit, effects);
            }
        }
    }

    /// What a condition's input read before and after this change.
    fn gate_values(
        &self,
        graph: &FlowGraph,
        condition_id: Uuid,
        changed: &(Uuid, String),
        previous: Option<&ParameterValue>,
        current: &ParameterValue,
    ) -> Option<(Option<ParameterValue>, ParameterValue)> {
        let feeder = graph.upstream(condition_id, 0).next()?;
        let source = graph.node(feeder.from_node)?;
        match source.kind {
            // Straight off a source: the raw values, so `Above(21.5)` compares
            // against a temperature rather than against a truth value.
            FlowNodeKind::Source(_) => Some((previous.cloned(), current.clone())),
            // Behind a gate: the gate is a boolean, and its "before" is whatever it
            // evaluated to with the old level in place. That is what makes "both
            // contacts closed" a rising edge in its own right, on the tick the
            // second one closes.
            FlowNodeKind::And | FlowNodeKind::Or | FlowNodeKind::Not => {
                let after = self.level_of(graph, source.id, &mut HashSet::new()).unwrap_or(false);
                let before = self
                    .level_before(graph, source.id, changed, previous, &mut HashSet::new())
                    .unwrap_or(false);
                Some((Some(ParameterValue::Bool(before)), ParameterValue::Bool(after)))
            }
            _ => None,
        }
    }

    /// The current truth value of a node's output level.
    ///
    /// `None` means the node does not carry a level at all, which is different from
    /// a level that happens to be false.
    fn level_of(&self, graph: &FlowGraph, node_id: Uuid, seen: &mut HashSet<Uuid>) -> Option<bool> {
        if !seen.insert(node_id) {
            // A cycle. Reading it as false terminates rather than recurring, and a
            // graph with a loop in it is a drawing mistake, not a hang.
            return Some(false);
        }
        let node = graph.node(node_id)?;
        match &node.kind {
            FlowNodeKind::Source(TriggerSource::Parameter { fixture_id, parameter }) => {
                let key = crate::model::playback::parameter_key(parameter);
                Some(
                    self.levels
                        .get(&(*fixture_id, key))
                        .and_then(|v| as_bool(Some(v)))
                        .unwrap_or(false),
                )
            }
            FlowNodeKind::And => Some(
                self.port_level(graph, node_id, 0, seen) && self.port_level(graph, node_id, 1, seen),
            ),
            FlowNodeKind::Or => Some(
                self.port_level(graph, node_id, 0, seen) || self.port_level(graph, node_id, 1, seen),
            ),
            FlowNodeKind::Not => Some(!self.port_level(graph, node_id, 0, seen)),
            _ => None,
        }
    }

    /// What one input port reads. Several edges into it are an *or*: any true level
    /// arriving makes the port true.
    fn port_level(
        &self,
        graph: &FlowGraph,
        node_id: Uuid,
        port: u8,
        seen: &mut HashSet<Uuid>,
    ) -> bool {
        graph.upstream(node_id, port).any(|edge| {
            let mut branch = seen.clone();
            self.level_of(graph, edge.from_node, &mut branch).unwrap_or(false)
        })
    }

    /// The same question, asked of the state before this tick's parameter changed.
    ///
    /// Only the source watching `changed` reads differently; every other source in
    /// the graph read then what it reads now, which is what `levels` already holds.
    fn level_before(
        &self,
        graph: &FlowGraph,
        node_id: Uuid,
        changed: &(Uuid, String),
        previous: Option<&ParameterValue>,
        seen: &mut HashSet<Uuid>,
    ) -> Option<bool> {
        if !seen.insert(node_id) {
            return Some(false);
        }
        let node = graph.node(node_id)?;
        match &node.kind {
            FlowNodeKind::Source(TriggerSource::Parameter { fixture_id, parameter }) => {
                let key = (*fixture_id, crate::model::playback::parameter_key(parameter));
                if key == *changed {
                    Some(as_bool(previous).unwrap_or(false))
                } else {
                    Some(self.levels.get(&key).and_then(|v| as_bool(Some(v))).unwrap_or(false))
                }
            }
            FlowNodeKind::And => Some(
                self.port_level_before(graph, node_id, 0, changed, previous, seen)
                    && self.port_level_before(graph, node_id, 1, changed, previous, seen),
            ),
            FlowNodeKind::Or => Some(
                self.port_level_before(graph, node_id, 0, changed, previous, seen)
                    || self.port_level_before(graph, node_id, 1, changed, previous, seen),
            ),
            FlowNodeKind::Not => {
                Some(!self.port_level_before(graph, node_id, 0, changed, previous, seen))
            }
            _ => None,
        }
    }

    fn port_level_before(
        &self,
        graph: &FlowGraph,
        node_id: Uuid,
        port: u8,
        changed: &(Uuid, String),
        previous: Option<&ParameterValue>,
        seen: &mut HashSet<Uuid>,
    ) -> bool {
        graph.upstream(node_id, port).any(|edge| {
            let mut branch = seen.clone();
            self.level_before(graph, edge.from_node, changed, previous, &mut branch)
                .unwrap_or(false)
        })
    }

    /// Send a pulse out of a node and follow it wherever it goes.
    ///
    /// `seen` is what makes a diamond fire its action once rather than twice, and a
    /// cycle stop rather than spin: a node takes the pulse the first time it arrives
    /// on a pass, and ignores it afterwards.
    fn pulse_downstream(
        &mut self,
        now: Instant,
        graph: &FlowGraph,
        from: Uuid,
        lit: &mut HashSet<Uuid>,
        effects: &mut Vec<FlowEffect>,
    ) {
        let mut seen = HashSet::from([from]);
        let mut queue = vec![from];

        while let Some(node_id) = queue.pop() {
            for edge in graph.downstream(node_id, 0) {
                let Some(target) = graph.node(edge.to_node) else { continue };
                if !graph.live(target) || !seen.insert(target.id) {
                    continue;
                }
                match &target.kind {
                    FlowNodeKind::Delay { ms } => {
                        lit.insert(target.id);
                        if *ms == 0 {
                            queue.push(target.id);
                            continue;
                        }
                        let due = now + std::time::Duration::from_millis(*ms as u64);
                        // Re-arming a delay that is already counting restarts it,
                        // rather than queueing a second pulse behind the first.
                        self.pending.retain(|(id, _)| *id != target.id);
                        self.pending.push((target.id, due));
                    }
                    FlowNodeKind::Action(action) => {
                        lit.insert(target.id);
                        effects.push(FlowEffect::Fire {
                            node_id: target.id,
                            action: action.clone(),
                        });
                    }
                    // Nothing else takes a pulse. The editor will not draw such an
                    // edge, and an imported graph that has one simply stops here.
                    _ => {}
                }
            }
        }
    }

}

/// Is this input the thing the source is watching?
fn watches(source: &TriggerSource, input: &InputEvent) -> bool {
    match source {
        TriggerSource::Parameter { fixture_id, parameter } => {
            *fixture_id == input.fixture_id
                && crate::model::playback::parameter_key(parameter) == input.key
        }
    }
}

fn fires(
    condition: TriggerCondition,
    previous: Option<&ParameterValue>,
    current: &ParameterValue,
) -> bool {
    match condition {
        TriggerCondition::RisingEdge => {
            !as_bool(previous).unwrap_or(false) && as_bool(Some(current)) == Some(true)
        }
        TriggerCondition::FallingEdge => {
            as_bool(previous) == Some(true) && as_bool(Some(current)) == Some(false)
        }
        TriggerCondition::AnyChange => previous != Some(current),
        // On the crossing, not on the level: a room that is already warm must not
        // fire the cue again on every reading.
        TriggerCondition::Above(threshold) => {
            let now = as_number(Some(current));
            let before = as_number(previous);
            now.is_some_and(|c| c > threshold) && before.map(|p| p <= threshold).unwrap_or(true)
        }
        TriggerCondition::Below(threshold) => {
            let now = as_number(Some(current));
            let before = as_number(previous);
            now.is_some_and(|c| c < threshold) && before.map(|p| p >= threshold).unwrap_or(true)
        }
    }
}

/// A parameter as a truth value, for the kinds that have one.
fn as_bool(value: Option<&ParameterValue>) -> Option<bool> {
    match value? {
        ParameterValue::Bool(b) => Some(*b),
        // A level counts as closed once it is off zero, so an edge condition works
        // on a dimmer as well as on a contact.
        ParameterValue::Float(f) => Some(*f > 0.0),
        ParameterValue::Int(i) => Some(*i != 0),
        ParameterValue::Text(t) => Some(!t.is_empty()),
        ParameterValue::Color { .. } => None,
    }
}

fn as_number(value: Option<&ParameterValue>) -> Option<f32> {
    match value? {
        ParameterValue::Float(f) => Some(*f),
        ParameterValue::Int(i) => Some(*i as f32),
        ParameterValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        ParameterValue::Text(_) | ParameterValue::Color { .. } => None,
    }
}

#[cfg(test)]
mod tests;
