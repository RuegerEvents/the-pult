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
//!
//! # Variables, and what a flow sees of its own writes
//!
//! A variable is read out of the graph rather than out of `levels`, because unlike a
//! parameter it has a value before anything has reported one: its initial value.
//! Its *changes* are found by comparing what the graph holds with what the last tick
//! saw, so a write from a flow, from a phone and from a peer all reach a source the
//! same way, and a variable seen for the first time is recorded rather than fired —
//! the rule a button follows, and for the same reason: a station promoted mid-show
//! must not fire every flow that reads one.
//!
//! **Nothing a flow writes is visible inside the pass that wrote it.** An action is
//! an effect the engine applies afterwards, so a [`FlowNodeKind::Gate`] reads the
//! value from before the pulse. That is what makes "PIR → gate on *free* → scare"
//! beside "PIR → set *occupied*" fire the scare once and then lock it out. The write
//! is seen on the next pass, and a run of passes each reacting only to the last one's
//! variable writes is cut at [`MAX_CHAIN`]: feedback that settles works, and an
//! oscillator stops and says so.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Utc};

use pult_schema::types::{
    fixture::ParameterValue,
    flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
    variable::Variable,
};
use uuid::Uuid;

/// One parameter changing, as the engine saw it happen.
#[derive(Debug, Clone, PartialEq)]
pub struct InputEvent {
    pub fixture_id: Uuid,
    /// The parameter key, as [`crate::model::playback::parameter_key`] writes it.
    pub key: String,
    /// What was there before, if anything ever was.
    pub previous: Option<ParameterValue>,
    pub current: ParameterValue,
}

/// How many passes in a row may react only to the previous pass's variable writes
/// before the chain is cut. Generous for anything that settles — a count clamped at a
/// bound stops writing in one — and small enough that an oscillator is noticed in a
/// fraction of a second rather than spinning the engine.
pub const MAX_CHAIN: u32 = 16;

/// What a flow tick asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowEffect {
    /// Carry out an action node's action, and mark it as having fired.
    Fire { node_id: Uuid, action: TriggerAction },
    /// A node lit up or went dark.
    SetActive { node_id: Uuid, active: bool },
    /// A pulse went through a node.
    Pulsed { node_id: Uuid },
    /// A feedback loop through variables was cut: these variables changed, and
    /// nothing reading them was told. The engine says so in the log.
    ChainCut { variables: Vec<Uuid> },
}

/// What a level is read from: a fixture parameter, keyed as the input events name
/// it, or a show variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Signal {
    Parameter(Uuid, String),
    Variable(Uuid),
}

impl Signal {
    fn of(source: &TriggerSource) -> Signal {
        match source {
            TriggerSource::Parameter { fixture_id, parameter } => {
                Signal::Parameter(*fixture_id, crate::model::playback::parameter_key(parameter))
            }
            TriggerSource::Variable { variable_id } => Signal::Variable(*variable_id),
        }
    }
}

/// The graph as the engine reads it out of show state.
pub struct FlowGraph<'a> {
    pub flows: &'a [Flow],
    pub nodes: &'a [FlowNode],
    pub edges: &'a [FlowEdge],
    pub variables: &'a [Variable],
}

impl FlowGraph<'_> {
    fn node(&self, id: Uuid) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    fn variable(&self, id: Uuid) -> Option<&Variable> {
        self.variables.iter().find(|v| v.id == id)
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
    levels: HashMap<Signal, ParameterValue>,
    /// What each variable held at the end of the last tick, which is how a change to
    /// one is noticed whoever made it.
    variables: HashMap<Uuid, f64>,
    /// Whether the last tick asked for a variable to be written, and how many ticks in
    /// a row have reacted to nothing else. See [`MAX_CHAIN`].
    wrote_variables: bool,
    chain: u32,
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

    /// The variables whose value moved since the last tick, as before-and-after.
    ///
    /// A variable seen for the first time is recorded and not reported, and one that
    /// has gone is forgotten.
    fn variable_changes(&mut self, graph: &FlowGraph) -> Vec<(Signal, Option<ParameterValue>, ParameterValue)> {
        let mut changes = Vec::new();
        for variable in graph.variables {
            let now = variable.current();
            match self.variables.insert(variable.id, now) {
                Some(before) if before != now => changes.push((
                    Signal::Variable(variable.id),
                    Some(ParameterValue::Float(before as f32)),
                    ParameterValue::Float(now as f32),
                )),
                _ => {}
            }
        }
        self.variables.retain(|id, _| graph.variable(*id).is_some());
        changes
    }

    /// When the soonest held pulse comes due.
    ///
    /// The engine sleeps on this rather than polling for it. Without it a graph with
    /// a five-minute delay in it would keep the engine awake for five minutes,
    /// which is the shape of periodic work this change exists to remove.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.iter().map(|(_, at)| *at).min()
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

        let mut changes: Vec<(Signal, Option<ParameterValue>, ParameterValue)> = inputs
            .iter()
            .map(|input| {
                let signal = Signal::Parameter(input.fixture_id, input.key.clone());
                // The engine's own before-and-after, not this module's cached level: a
                // contact pressed and released between two ticks arrives as two events,
                // and only the reported `previous` still knows that happened.
                self.levels.insert(signal.clone(), input.current.clone());
                (signal, input.previous.clone(), input.current.clone())
            })
            .collect();

        // Variables, and the chain rule: a tick that has nothing to react to but the
        // last tick's own variable writes is one more link, and past the limit what
        // changed is recorded and not passed on.
        let moved = self.variable_changes(graph);
        let pressed = !lit.is_empty();
        if !moved.is_empty() && inputs.is_empty() && !pressed && self.wrote_variables {
            self.chain += 1;
        } else {
            self.chain = 0;
        }
        if self.chain > MAX_CHAIN {
            effects.push(FlowEffect::ChainCut {
                variables: moved
                    .iter()
                    .filter_map(|(signal, ..)| match signal {
                        Signal::Variable(id) => Some(*id),
                        Signal::Parameter(..) => None,
                    })
                    .collect(),
            });
            self.chain = 0;
        } else {
            changes.extend(moved);
        }

        for (signal, previous, current) in &changes {
            // Only conditions downstream of a source that watches *this* signal can
            // have changed. Everything else still reads the same level it did.
            for node in graph.nodes.iter().filter(|n| graph.live(n)) {
                let FlowNodeKind::Source(source) = &node.kind else { continue };
                if Signal::of(source) != *signal {
                    continue;
                }
                if as_bool(Some(current)).unwrap_or(false) {
                    lit.insert(node.id);
                }
                self.propagate_level(
                    now,
                    graph,
                    node.id,
                    signal,
                    previous.as_ref(),
                    current,
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

        // What lasts: a pending delay stays lit while it counts down, so the graph
        // shows the wait; a level that is still true keeps its node lit even on a tick
        // where nothing changed, which is what makes a closed contact look closed.
        let mut steady: HashSet<Uuid> = self.pending.iter().map(|(id, _)| *id).collect();
        for node in graph.nodes.iter().filter(|n| graph.live(n)) {
            if matches!(node.kind, FlowNodeKind::Delay { .. }) {
                continue;
            }
            if self.level_of(graph, node.id, &mut HashSet::new()) == Some(true) {
                steady.insert(node.id);
            }
        }
        // Everything else lit this tick was a pulse going through: said once, as a
        // flash, and not left lit until whatever the next tick turns out to be.
        for node_id in lit.difference(&steady) {
            effects.push(FlowEffect::Pulsed { node_id: *node_id });
        }

        for node_id in steady.difference(&self.lit) {
            effects.push(FlowEffect::SetActive { node_id: *node_id, active: true });
        }
        for node_id in self.lit.difference(&steady) {
            effects.push(FlowEffect::SetActive { node_id: *node_id, active: false });
        }
        self.lit = steady;

        self.wrote_variables = effects.iter().any(|effect| {
            matches!(effect, FlowEffect::Fire { action: TriggerAction::SetVariable { .. }, .. })
        });
        effects
    }

    /// A source's level changed. Walk the level edges out of it, and fire any
    /// condition that was waiting for exactly this change.
    fn propagate_level(
        &mut self,
        now: Instant,
        graph: &FlowGraph,
        from: Uuid,
        changed: &Signal,
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
        changed: &Signal,
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
            FlowNodeKind::Source(source) => Some(self.truth_of(graph, &Signal::of(source))),
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

    /// A source's level as a truth value: what the rig last reported for a
    /// parameter, what the graph holds for a variable.
    fn truth_of(&self, graph: &FlowGraph, signal: &Signal) -> bool {
        match signal {
            Signal::Parameter(..) => {
                self.levels.get(signal).and_then(|v| as_bool(Some(v))).unwrap_or(false)
            }
            Signal::Variable(id) => graph.variable(*id).is_some_and(|v| v.current() != 0.0),
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
        changed: &Signal,
        previous: Option<&ParameterValue>,
        seen: &mut HashSet<Uuid>,
    ) -> Option<bool> {
        if !seen.insert(node_id) {
            return Some(false);
        }
        let node = graph.node(node_id)?;
        match &node.kind {
            FlowNodeKind::Source(source) => {
                let signal = Signal::of(source);
                if signal == *changed {
                    Some(as_bool(previous).unwrap_or(false))
                } else {
                    Some(self.truth_of(graph, &signal))
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
        changed: &Signal,
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
                    // The pulse goes on only while the level on port 1 is true, read
                    // as it stood before this pass wrote anything. A pulse arriving on
                    // the level port is a drawing the editor refuses, and stops here.
                    FlowNodeKind::Gate if edge.to_port == 0 => {
                        if self.port_level(graph, target.id, 1, &mut HashSet::new()) {
                            lit.insert(target.id);
                            queue.push(target.id);
                        }
                    }
                    // Nothing else takes a pulse. The editor will not draw such an
                    // edge, and an imported graph that has one simply stops here.
                    _ => {}
                }
            }
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
