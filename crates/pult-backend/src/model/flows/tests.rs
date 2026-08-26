//! Flow tests. Time is passed in, so a ten-second delay runs in microseconds.
//!
//! The first block is the old one-row-per-rule trigger suite, drawn as graphs: a
//! source, a condition, maybe a delay, an action. Every one of those rules still
//! behaves exactly as it did, which is the point — the graph was meant to be another
//! way of drawing them, not a change to what they mean.
//!
//! The second block is what a row could never say.

use std::time::Duration;

use pult_schema::types::fixture::ParameterKind;

use super::*;

// ── Building graphs ───────────────────────────────────────────────────────────

/// A graph under construction, so a test reads as the picture it is drawing.
#[derive(Default)]
struct Draw {
    flows: Vec<Flow>,
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
}

impl Draw {
    fn new() -> Self {
        let mut draw = Draw::default();
        draw.flows.push(Flow { id: Uuid::new_v4(), name: "Doorbell".into(), enabled: true });
        draw
    }

    fn flow_id(&self) -> Uuid {
        self.flows[0].id
    }

    fn node(&mut self, kind: FlowNodeKind) -> Uuid {
        let id = Uuid::new_v4();
        let flow_id = self.flow_id();
        self.nodes.push(FlowNode {
            id,
            flow_id,
            kind,
            x: 0.0,
            y: 0.0,
            active: false,
            last_fired_at: None,
        });
        id
    }

    fn source(&mut self, fixture_id: Uuid, parameter: ParameterKind) -> Uuid {
        self.node(FlowNodeKind::Source(TriggerSource::Parameter { fixture_id, parameter }))
    }

    fn condition(&mut self, condition: TriggerCondition) -> Uuid {
        self.node(FlowNodeKind::Condition(condition))
    }

    fn delay(&mut self, ms: u32) -> Uuid {
        self.node(FlowNodeKind::Delay { ms })
    }

    fn action(&mut self) -> Uuid {
        self.node(FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: Uuid::new_v4() }))
    }

    fn wire(&mut self, from: Uuid, to: Uuid) {
        self.wire_port(from, to, 0);
    }

    fn wire_port(&mut self, from: Uuid, to: Uuid, to_port: u8) {
        let flow_id = self.flow_id();
        self.edges.push(FlowEdge {
            id: Uuid::new_v4(),
            flow_id,
            from_node: from,
            from_port: 0,
            to_node: to,
            to_port,
        });
    }

    fn graph(&self) -> FlowGraph<'_> {
        FlowGraph { flows: &self.flows, nodes: &self.nodes, edges: &self.edges }
    }
}

/// The shape every migrated trigger takes: source → condition → action.
fn a_rule(condition: TriggerCondition, fixture_id: Uuid, parameter: ParameterKind) -> (Draw, Uuid) {
    let mut draw = Draw::new();
    let source = draw.source(fixture_id, parameter);
    let gate = draw.condition(condition);
    let action = draw.action();
    draw.wire(source, gate);
    draw.wire(gate, action);
    (draw, action)
}

fn an_edge(fixture_id: Uuid, key: &str, from: Option<bool>, to: bool) -> InputEvent {
    InputEvent {
        fixture_id,
        key: key.into(),
        previous: from.map(ParameterValue::Bool),
        current: ParameterValue::Bool(to),
    }
}

fn a_reading(fixture_id: Uuid, key: &str, from: Option<f32>, to: f32) -> InputEvent {
    InputEvent {
        fixture_id,
        key: key.into(),
        previous: from.map(ParameterValue::Float),
        current: ParameterValue::Float(to),
    }
}

fn fired(effects: &[FlowEffect]) -> Vec<Uuid> {
    effects
        .iter()
        .filter_map(|e| match e {
            FlowEffect::Fire { node_id, .. } => Some(*node_id),
            FlowEffect::SetActive { .. } => None,
        })
        .collect()
}

// ── Edges ─────────────────────────────────────────────────────────────────────

#[test]
fn a_rising_edge_fires_once() {
    let fixture = Uuid::new_v4();
    let (draw, action) = a_rule(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    let effects = flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);
    assert_eq!(fired(&effects), vec![action]);

    // The same value arriving again is not another edge.
    let effects = flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(true), true)]);
    assert!(fired(&effects).is_empty());
}

#[test]
fn a_contact_closing_for_the_first_time_is_a_rising_edge() {
    let fixture = Uuid::new_v4();
    let (draw, action) = a_rule(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects = flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:0", None, true)]);

    assert_eq!(fired(&effects), vec![action], "no previous value is the same as off");
}

#[test]
fn a_release_does_not_fire_a_rising_edge() {
    let fixture = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects =
        flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:0", Some(true), false)]);

    assert!(fired(&effects).is_empty());
}

#[test]
fn a_falling_edge_fires_on_the_release_and_not_the_press() {
    let fixture = Uuid::new_v4();
    let (draw, action) = a_rule(TriggerCondition::FallingEdge, fixture, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    assert!(fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]))
        .is_empty());
    assert_eq!(
        fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(true), false)])),
        vec![action],
    );
}

#[test]
fn any_change_fires_on_both_directions_but_not_on_a_repeat() {
    let fixture = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::AnyChange, fixture, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    assert_eq!(
        fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)])).len(),
        1
    );
    assert_eq!(
        fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(true), false)])).len(),
        1
    );
    assert!(fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), false)]))
        .is_empty());
}

// ── Thresholds ────────────────────────────────────────────────────────────────

#[test]
fn a_threshold_fires_on_the_crossing_and_not_on_the_level() {
    let fixture = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::Above(25.0), fixture, ParameterKind::Temperature);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    assert!(fired(&flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(20.0), 24.0)]))
        .is_empty());
    assert_eq!(
        fired(&flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(24.0), 26.0)]))
            .len(),
        1
    );
    assert!(
        fired(&flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(26.0), 27.0)]))
            .is_empty(),
        "a room that is already warm must not fire the cue on every reading",
    );
}

#[test]
fn a_threshold_arms_again_once_the_level_comes_back() {
    let fixture = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::Above(25.0), fixture, ParameterKind::Temperature);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(20.0), 26.0)]);
    flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(26.0), 20.0)]);

    assert_eq!(
        fired(&flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(20.0), 26.0)]))
            .len(),
        1,
    );
}

#[test]
fn a_below_threshold_is_the_mirror_of_an_above_one() {
    let fixture = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::Below(5.0), fixture, ParameterKind::Temperature);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    assert!(fired(&flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(10.0), 6.0)]))
        .is_empty());
    assert_eq!(
        fired(&flows.tick(now, &graph, &[a_reading(fixture, "Temperature", Some(6.0), 4.0)])).len(),
        1
    );
}

// ── What a source watches ─────────────────────────────────────────────────────

#[test]
fn a_source_ignores_a_parameter_it_is_not_watching() {
    let fixture = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects =
        flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:1", Some(false), true)]);

    assert!(fired(&effects).is_empty(), "contact 1 is not contact 0");
}

#[test]
fn a_source_ignores_the_same_parameter_on_another_fixture() {
    let watched = Uuid::new_v4();
    let (draw, _) = a_rule(TriggerCondition::RisingEdge, watched, ParameterKind::Contact(0));
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects = flows.tick(
        Instant::now(),
        &graph,
        &[an_edge(Uuid::new_v4(), "Contact:0", Some(false), true)],
    );

    assert!(fired(&effects).is_empty());
}

#[test]
fn a_disabled_flow_does_nothing() {
    let fixture = Uuid::new_v4();
    let (mut draw, _) = a_rule(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    draw.flows[0].enabled = false;
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects =
        flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);

    assert!(effects.is_empty());
}

#[test]
fn one_input_can_fire_several_conditions() {
    let fixture = Uuid::new_v4();
    let mut draw = Draw::new();
    let source = draw.source(fixture, ParameterKind::Contact(0));
    let rising = draw.condition(TriggerCondition::RisingEdge);
    let any = draw.condition(TriggerCondition::AnyChange);
    let one = draw.action();
    let two = draw.action();
    draw.wire(source, rising);
    draw.wire(source, any);
    draw.wire(rising, one);
    draw.wire(any, two);
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects =
        flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);

    assert_eq!(fired(&effects).len(), 2);
}

// ── Delays ────────────────────────────────────────────────────────────────────

/// source → condition → delay → action, the four-node chain a delayed trigger
/// migrates into.
fn a_delayed_rule(fixture: Uuid, ms: u32) -> (Draw, Uuid, Uuid) {
    let mut draw = Draw::new();
    let source = draw.source(fixture, ParameterKind::Contact(0));
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let wait = draw.delay(ms);
    let action = draw.action();
    draw.wire(source, gate);
    draw.wire(gate, wait);
    draw.wire(wait, action);
    (draw, wait, action)
}

#[test]
fn a_delay_holds_the_action_back_and_lights_up_while_it_waits() {
    let fixture = Uuid::new_v4();
    let (draw, wait, action) = a_delayed_rule(fixture, 2000);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    let effects = flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);
    assert!(fired(&effects).is_empty());
    assert!(effects.contains(&FlowEffect::SetActive { node_id: wait, active: true }));
    assert!(flows.has_work());

    // Still waiting, one tick later.
    assert!(fired(&flows.tick(now + Duration::from_millis(25), &graph, &[])).is_empty());

    let effects = flows.tick(now + Duration::from_millis(2000), &graph, &[]);
    assert_eq!(fired(&effects), vec![action]);
    assert!(!flows.has_work());
}

#[test]
fn a_node_deleted_while_its_delay_runs_does_not_go_off() {
    let fixture = Uuid::new_v4();
    let (draw, _, _) = a_delayed_rule(fixture, 2000);
    let mut flows = Flows::default();
    let now = Instant::now();

    flows.tick(now, &draw.graph(), &[an_edge(fixture, "Contact:0", Some(false), true)]);

    let empty = Draw::new();
    let effects = flows.tick(now + Duration::from_millis(2000), &empty.graph(), &[]);

    assert!(fired(&effects).is_empty());
    assert!(!flows.has_work());
}

#[test]
fn a_flow_switched_off_while_its_delay_runs_does_not_go_off() {
    let fixture = Uuid::new_v4();
    let (mut draw, _, _) = a_delayed_rule(fixture, 2000);
    let mut flows = Flows::default();
    let now = Instant::now();

    flows.tick(now, &draw.graph(), &[an_edge(fixture, "Contact:0", Some(false), true)]);
    draw.flows[0].enabled = false;

    assert!(fired(&flows.tick(now + Duration::from_millis(2000), &draw.graph(), &[])).is_empty());
}

#[test]
fn pressing_again_during_a_delay_restarts_it_rather_than_queueing_a_second() {
    let fixture = Uuid::new_v4();
    let (draw, _, action) = a_delayed_rule(fixture, 1000);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);
    flows.tick(
        now + Duration::from_millis(500),
        &graph,
        &[an_edge(fixture, "Contact:0", Some(false), true)],
    );

    // The first delay would have been up by now; the second has not.
    assert!(fired(&flows.tick(now + Duration::from_millis(1000), &graph, &[])).is_empty());
    assert_eq!(
        fired(&flows.tick(now + Duration::from_millis(1500), &graph, &[])),
        vec![action],
    );
}

#[test]
fn nothing_happens_on_a_tick_with_no_input_and_no_delay_running() {
    let draw = Draw::new();
    let mut flows = Flows::default();
    assert!(!flows.has_work());
    assert!(flows.tick(Instant::now(), &draw.graph(), &[]).is_empty());
}

// ── What a row could not say ──────────────────────────────────────────────────

#[test]
fn two_contacts_into_an_and_fire_only_when_both_are_closed() {
    let left = Uuid::new_v4();
    let right = Uuid::new_v4();
    let mut draw = Draw::new();
    let a = draw.source(left, ParameterKind::Contact(0));
    let b = draw.source(right, ParameterKind::Contact(0));
    let both = draw.node(FlowNodeKind::And);
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let action = draw.action();
    draw.wire_port(a, both, 0);
    draw.wire_port(b, both, 1);
    draw.wire(both, gate);
    draw.wire(gate, action);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    // One alone is not enough.
    assert!(
        fired(&flows.tick(now, &graph, &[an_edge(left, "Contact:0", Some(false), true)]))
            .is_empty(),
        "one contact closing is not both of them",
    );

    // The second one closing makes the gate true, which is a rising edge of its own.
    assert_eq!(
        fired(&flows.tick(now, &graph, &[an_edge(right, "Contact:0", Some(false), true)])),
        vec![action],
    );

    // And it does not fire again while they both stay closed.
    assert!(fired(&flows.tick(now, &graph, &[an_edge(left, "Contact:0", Some(true), true)]))
        .is_empty());
}

#[test]
fn an_or_fires_on_whichever_arrives_first() {
    let left = Uuid::new_v4();
    let right = Uuid::new_v4();
    let mut draw = Draw::new();
    let a = draw.source(left, ParameterKind::Contact(0));
    let b = draw.source(right, ParameterKind::Contact(0));
    let either = draw.node(FlowNodeKind::Or);
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let action = draw.action();
    draw.wire_port(a, either, 0);
    draw.wire_port(b, either, 1);
    draw.wire(either, gate);
    draw.wire(gate, action);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    assert_eq!(
        fired(&flows.tick(now, &graph, &[an_edge(right, "Contact:0", Some(false), true)])),
        vec![action],
    );
    // The other one closing changes nothing: the gate was already true.
    assert!(fired(&flows.tick(now, &graph, &[an_edge(left, "Contact:0", Some(false), true)]))
        .is_empty());
}

#[test]
fn a_not_inverts_the_level_underneath_it() {
    let fixture = Uuid::new_v4();
    let mut draw = Draw::new();
    let source = draw.source(fixture, ParameterKind::Contact(0));
    let inverted = draw.node(FlowNodeKind::Not);
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let action = draw.action();
    draw.wire(source, inverted);
    draw.wire(inverted, gate);
    draw.wire(gate, action);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    // Closing the contact makes the inverted level false: no rising edge.
    assert!(fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]))
        .is_empty());
    // Releasing it makes the inverted level true, which is the edge.
    assert_eq!(
        fired(&flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(true), false)])),
        vec![action],
    );
}

#[test]
fn one_delay_can_feed_several_actions() {
    let fixture = Uuid::new_v4();
    let mut draw = Draw::new();
    let source = draw.source(fixture, ParameterKind::Contact(0));
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let wait = draw.delay(500);
    let one = draw.action();
    let two = draw.action();
    draw.wire(source, gate);
    draw.wire(gate, wait);
    draw.wire(wait, one);
    draw.wire(wait, two);
    let graph = draw.graph();
    let mut flows = Flows::default();
    let now = Instant::now();

    flows.tick(now, &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);
    let effects = flows.tick(now + Duration::from_millis(500), &graph, &[]);

    let mut both = fired(&effects);
    both.sort();
    let mut expected = vec![one, two];
    expected.sort();
    assert_eq!(both, expected, "a delay fans out to everything wired to it");
}

#[test]
fn a_button_fires_when_its_stamp_changes() {
    let mut draw = Draw::new();
    let button = draw.node(FlowNodeKind::Button);
    let action = draw.action();
    draw.wire(button, action);
    let now = Instant::now();
    let mut flows = Flows::default();

    // First sight of the button is not a press, or a console joining a running show
    // would set every button off as it caught up.
    assert!(fired(&flows.tick(now, &draw.graph(), &[])).is_empty());

    draw.nodes[0].last_fired_at = Some(chrono::Utc::now());
    assert_eq!(fired(&flows.tick(now, &draw.graph(), &[])), vec![action]);

    // And the same stamp arriving again is not a second press.
    assert!(fired(&flows.tick(now, &draw.graph(), &[])).is_empty());
}

#[test]
fn a_button_in_a_switched_off_flow_does_nothing() {
    let mut draw = Draw::new();
    let button = draw.node(FlowNodeKind::Button);
    let action = draw.action();
    draw.wire(button, action);
    draw.flows[0].enabled = false;
    let now = Instant::now();
    let mut flows = Flows::default();

    flows.tick(now, &draw.graph(), &[]);
    draw.nodes[0].last_fired_at = Some(chrono::Utc::now());

    assert!(fired(&flows.tick(now, &draw.graph(), &[])).is_empty());
    let _ = button;
}

#[test]
fn a_cycle_stops_instead_of_hanging() {
    let fixture = Uuid::new_v4();
    let mut draw = Draw::new();
    let source = draw.source(fixture, ParameterKind::Contact(0));
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let first = draw.delay(0);
    let second = draw.delay(0);
    let action = draw.action();
    draw.wire(source, gate);
    draw.wire(gate, first);
    draw.wire(first, second);
    // Back where it came from. A drawing mistake, not a reason to spin forever.
    draw.wire(second, first);
    draw.wire(second, action);
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects =
        flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);

    assert_eq!(fired(&effects), vec![action]);
}

#[test]
fn a_level_cycle_reads_as_false_rather_than_recurring() {
    let mut draw = Draw::new();
    let one = draw.node(FlowNodeKind::And);
    let two = draw.node(FlowNodeKind::And);
    draw.wire_port(one, two, 0);
    draw.wire_port(two, one, 0);
    let graph = draw.graph();
    let flows = Flows::default();

    assert_eq!(flows.level_of(&graph, one, &mut HashSet::new()), Some(false));
}

#[test]
fn a_diamond_fires_its_action_once() {
    let fixture = Uuid::new_v4();
    let mut draw = Draw::new();
    let source = draw.source(fixture, ParameterKind::Contact(0));
    let gate = draw.condition(TriggerCondition::RisingEdge);
    let left = draw.delay(0);
    let right = draw.delay(0);
    let action = draw.action();
    draw.wire(source, gate);
    draw.wire(gate, left);
    draw.wire(gate, right);
    draw.wire(left, action);
    draw.wire(right, action);
    let graph = draw.graph();
    let mut flows = Flows::default();

    let effects =
        flows.tick(Instant::now(), &graph, &[an_edge(fixture, "Contact:0", Some(false), true)]);

    assert_eq!(fired(&effects), vec![action], "two paths to one action are still one action");
}
