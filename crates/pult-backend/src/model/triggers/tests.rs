//! Trigger tests. Time is passed in, so a ten-second delay runs in microseconds.

use std::time::Duration;

use pult_schema::types::fixture::ParameterKind;

use super::*;

fn a_trigger(condition: TriggerCondition, fixture_id: Uuid, parameter: ParameterKind) -> Trigger {
    Trigger {
        id: Uuid::new_v4(),
        name: "Doorbell".into(),
        source: TriggerSource::Parameter { fixture_id, parameter },
        condition,
        action: TriggerAction::GoNext { sequence_id: Uuid::new_v4() },
        delay_ms: 0,
        enabled: true,
        pending: false,
        last_fired_at: None,
    }
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

fn fired(effects: &[TriggerEffect]) -> Vec<Uuid> {
    effects
        .iter()
        .filter_map(|e| match e {
            TriggerEffect::Fire { trigger_id, .. } => Some(*trigger_id),
            TriggerEffect::SetPending { .. } => None,
        })
        .collect()
}

// ── Edges ─────────────────────────────────────────────────────────────────────

#[test]
fn a_rising_edge_fires_once() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();
    let now = Instant::now();

    let effects = triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)]);
    assert_eq!(fired(&effects), vec![trigger.id]);

    // The same value arriving again is not another edge.
    let effects = triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(true), true)]);
    assert!(fired(&effects).is_empty());
}

#[test]
fn a_contact_closing_for_the_first_time_is_a_rising_edge() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();

    let effects =
        triggers.tick(Instant::now(), &[trigger.clone()], &[an_edge(fixture, "Contact:0", None, true)]);

    assert_eq!(fired(&effects), vec![trigger.id], "no previous value is the same as off");
}

#[test]
fn a_release_does_not_fire_a_rising_edge() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();

    let effects =
        triggers.tick(Instant::now(), &[trigger], &[an_edge(fixture, "Contact:0", Some(true), false)]);

    assert!(fired(&effects).is_empty());
}

#[test]
fn a_falling_edge_fires_on_the_release_and_not_the_press() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::FallingEdge, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();
    let now = Instant::now();

    assert!(fired(&triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)])).is_empty());
    assert_eq!(
        fired(&triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(true), false)])),
        vec![trigger.id],
    );
}

#[test]
fn any_change_fires_on_both_directions_but_not_on_a_repeat() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::AnyChange, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();
    let now = Instant::now();

    assert_eq!(fired(&triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)])).len(), 1);
    assert_eq!(fired(&triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(true), false)])).len(), 1);
    assert!(fired(&triggers.tick(now, &[trigger], &[an_edge(fixture, "Contact:0", Some(false), false)])).is_empty());
}

// ── Thresholds ────────────────────────────────────────────────────────────────

#[test]
fn a_threshold_fires_on_the_crossing_and_not_on_the_level() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::Above(25.0), fixture, ParameterKind::Temperature);
    let mut triggers = Triggers::default();
    let now = Instant::now();

    assert!(fired(&triggers.tick(now, &[trigger.clone()], &[a_reading(fixture, "Temperature", Some(20.0), 24.0)])).is_empty());
    assert_eq!(fired(&triggers.tick(now, &[trigger.clone()], &[a_reading(fixture, "Temperature", Some(24.0), 26.0)])).len(), 1);
    assert!(
        fired(&triggers.tick(now, &[trigger], &[a_reading(fixture, "Temperature", Some(26.0), 27.0)])).is_empty(),
        "a room that is already warm must not fire the cue on every reading",
    );
}

#[test]
fn a_threshold_arms_again_once_the_level_comes_back() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::Above(25.0), fixture, ParameterKind::Temperature);
    let mut triggers = Triggers::default();
    let now = Instant::now();

    triggers.tick(now, &[trigger.clone()], &[a_reading(fixture, "Temperature", Some(20.0), 26.0)]);
    triggers.tick(now, &[trigger.clone()], &[a_reading(fixture, "Temperature", Some(26.0), 20.0)]);

    assert_eq!(
        fired(&triggers.tick(now, &[trigger], &[a_reading(fixture, "Temperature", Some(20.0), 26.0)])).len(),
        1,
    );
}

#[test]
fn a_below_threshold_is_the_mirror_of_an_above_one() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::Below(5.0), fixture, ParameterKind::Temperature);
    let mut triggers = Triggers::default();
    let now = Instant::now();

    assert!(fired(&triggers.tick(now, &[trigger.clone()], &[a_reading(fixture, "Temperature", Some(10.0), 6.0)])).is_empty());
    assert_eq!(fired(&triggers.tick(now, &[trigger], &[a_reading(fixture, "Temperature", Some(6.0), 4.0)])).len(), 1);
}

// ── What a trigger watches ────────────────────────────────────────────────────

#[test]
fn a_trigger_ignores_a_parameter_it_is_not_watching() {
    let fixture = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();

    let effects = triggers.tick(
        Instant::now(),
        &[trigger],
        &[an_edge(fixture, "Contact:1", Some(false), true)],
    );

    assert!(fired(&effects).is_empty(), "contact 1 is not contact 0");
}

#[test]
fn a_trigger_ignores_the_same_parameter_on_another_fixture() {
    let watched = Uuid::new_v4();
    let trigger = a_trigger(TriggerCondition::RisingEdge, watched, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();

    let effects = triggers.tick(
        Instant::now(),
        &[trigger],
        &[an_edge(Uuid::new_v4(), "Contact:0", Some(false), true)],
    );

    assert!(fired(&effects).is_empty());
}

#[test]
fn a_disabled_trigger_does_nothing() {
    let fixture = Uuid::new_v4();
    let mut trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    trigger.enabled = false;
    let mut triggers = Triggers::default();

    let effects =
        triggers.tick(Instant::now(), &[trigger], &[an_edge(fixture, "Contact:0", Some(false), true)]);

    assert!(effects.is_empty());
}

#[test]
fn one_input_can_fire_several_triggers() {
    let fixture = Uuid::new_v4();
    let one = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    let two = a_trigger(TriggerCondition::AnyChange, fixture, ParameterKind::Contact(0));
    let mut triggers = Triggers::default();

    let effects = triggers.tick(
        Instant::now(),
        &[one.clone(), two.clone()],
        &[an_edge(fixture, "Contact:0", Some(false), true)],
    );

    assert_eq!(fired(&effects).len(), 2);
}

// ── Delays ────────────────────────────────────────────────────────────────────

#[test]
fn a_delay_holds_the_action_back_and_marks_the_trigger_pending() {
    let fixture = Uuid::new_v4();
    let mut trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    trigger.delay_ms = 2000;
    let mut triggers = Triggers::default();
    let now = Instant::now();

    let effects = triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)]);
    assert!(fired(&effects).is_empty());
    assert_eq!(
        effects,
        vec![TriggerEffect::SetPending { trigger_id: trigger.id, pending: true }],
    );
    assert!(triggers.has_work());

    // Still waiting, one tick later.
    assert!(triggers.tick(now + Duration::from_millis(25), &[trigger.clone()], &[]).is_empty());

    let effects = triggers.tick(now + Duration::from_millis(2000), &[trigger.clone()], &[]);
    assert_eq!(fired(&effects), vec![trigger.id]);
    assert!(effects.contains(&TriggerEffect::SetPending { trigger_id: trigger.id, pending: false }));
    assert!(!triggers.has_work());
}

#[test]
fn a_trigger_deleted_while_its_delay_runs_does_not_go_off() {
    let fixture = Uuid::new_v4();
    let mut trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    trigger.delay_ms = 2000;
    let mut triggers = Triggers::default();
    let now = Instant::now();

    triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)]);

    let effects = triggers.tick(now + Duration::from_millis(2000), &[], &[]);

    assert!(effects.is_empty());
    assert!(!triggers.has_work());
}

#[test]
fn a_trigger_switched_off_while_its_delay_runs_does_not_go_off() {
    let fixture = Uuid::new_v4();
    let mut trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    trigger.delay_ms = 2000;
    let mut triggers = Triggers::default();
    let now = Instant::now();

    triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)]);
    trigger.enabled = false;

    assert!(triggers.tick(now + Duration::from_millis(2000), &[trigger], &[]).is_empty());
}

#[test]
fn pressing_again_during_a_delay_restarts_it_rather_than_queueing_a_second() {
    let fixture = Uuid::new_v4();
    let mut trigger = a_trigger(TriggerCondition::RisingEdge, fixture, ParameterKind::Contact(0));
    trigger.delay_ms = 1000;
    let mut triggers = Triggers::default();
    let now = Instant::now();

    triggers.tick(now, &[trigger.clone()], &[an_edge(fixture, "Contact:0", Some(false), true)]);
    triggers.tick(
        now + Duration::from_millis(500),
        &[trigger.clone()],
        &[an_edge(fixture, "Contact:0", Some(false), true)],
    );

    // The first delay would have been up by now; the second has not.
    assert!(fired(&triggers.tick(now + Duration::from_millis(1000), &[trigger.clone()], &[])).is_empty());
    assert_eq!(
        fired(&triggers.tick(now + Duration::from_millis(1500), &[trigger.clone()], &[])),
        vec![trigger.id],
    );
}

#[test]
fn nothing_happens_on_a_tick_with_no_input_and_no_delay_running() {
    let mut triggers = Triggers::default();
    assert!(!triggers.has_work());
    assert!(triggers.tick(Instant::now(), &[], &[]).is_empty());
}
