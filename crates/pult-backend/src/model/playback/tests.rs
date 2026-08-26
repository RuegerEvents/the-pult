//! Playback tests.
//!
//! Time is passed in, so a fade runs to completion here in microseconds.

use std::collections::HashMap;

use pult_schema::types::{
    cue::{Cue, FollowMode, ParameterCapture},
    fixture::{Fixture, ParameterKind, ParameterValue},
    sequence::Sequence,
};

use super::*;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn a_fixture() -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: Uuid::new_v4(),
        universe: 1,
        dmx_address: 1,
        position: None,
        live_values: HashMap::new(),
        active_preset: None,
    }
}

fn a_cue(fade_in_ms: u32, captures: Vec<ParameterCapture>) -> Cue {
    Cue {
        id: Uuid::new_v4(),
        name: "Cue".into(),
        number: 1.0,
        captures,
        follow_mode: FollowMode::Manual,
        fade_in_ms,
        fade_out_ms: 0,
        is_active: false,
    }
}

fn intensity(fixture_id: Uuid, value: f32) -> ParameterCapture {
    ParameterCapture {
        fixture_id,
        parameter_kind: ParameterKind::Intensity,
        value: ParameterValue::Float(value),
        fade_in_ms: 0,
        fade_out_ms: 0,
        delay_in_ms: 0,
    }
}

fn a_sequence(cues: &[&Cue], active: Option<usize>) -> Sequence {
    Sequence {
        id: Uuid::new_v4(),
        name: "Act 1".into(),
        cue_ids: cues.iter().map(|c| c.id).collect(),
        active_cue_index: active,
    }
}

fn live(effects: &[PlaybackEffect], fixture_id: Uuid, key: &str) -> Option<ParameterValue> {
    effects.iter().rev().find_map(|e| match e {
        PlaybackEffect::SetLiveValues { fixture_id: f, values } if *f == fixture_id => {
            values.get(key).cloned()
        }
        _ => None,
    })
}

fn as_float(value: Option<ParameterValue>) -> f32 {
    match value {
        Some(ParameterValue::Float(f)) => f,
        other => panic!("expected a float, got {other:?}"),
    }
}

/// Apply live-value effects back onto the fixtures, the way the engine does, so the
/// next tick sees the state the last one produced.
fn apply(fixtures: &mut [Fixture], effects: &[PlaybackEffect]) {
    for effect in effects {
        if let PlaybackEffect::SetLiveValues { fixture_id, values } = effect {
            if let Some(f) = fixtures.iter_mut().find(|f| f.id == *fixture_id) {
                f.live_values = values.clone();
            }
        }
    }
}

// ── Cue activation ────────────────────────────────────────────────────────────

#[test]
fn taking_a_cue_marks_it_active() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let sequence = a_sequence(&[&cue], Some(0));
    let fixtures = [fixture];
    let cues = [cue.clone()];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures));

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: cue.id, is_active: true }));
}

#[test]
fn moving_on_deactivates_the_previous_cue() {
    let fixture = a_fixture();
    let first = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let second = a_cue(0, vec![intensity(fixture.id, 0.5)]);
    let fixtures = [fixture];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let now = Instant::now();
    playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures));

    sequences[0].active_cue_index = Some(1);
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures));

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: first.id, is_active: false }));
    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: second.id, is_active: true }));
}

#[test]
fn running_off_the_end_deactivates_the_last_cue_and_holds_the_output() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let mut sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let now = Instant::now();
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures));
    apply(&mut fixtures, &effects);

    sequences[0].active_cue_index = None;
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures));

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: cue.id, is_active: false }));
    assert_eq!(
        fixtures[0].live_values.get("Intensity"),
        Some(&ParameterValue::Float(1.0)),
        "a light does not go dark because the operator ran out of cues",
    );
}

// ── Fading ────────────────────────────────────────────────────────────────────

#[test]
fn a_zero_time_cue_snaps_straight_to_its_values() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures));

    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 1.0);
    assert!(!playback.has_work(), "a snap leaves nothing running");
}

#[test]
fn a_fade_moves_through_its_middle_before_reaching_the_target() {
    let fixture = a_fixture();
    let cue = a_cue(4000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();

    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));
    apply(&mut fixtures, &effects);
    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 0.0, "starts from dark");

    let effects = playback.tick(start + Duration::from_secs(1), &ShowView::new(&sequences, &cues, &fixtures));
    apply(&mut fixtures, &effects);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.25).abs() < 0.001);

    let effects = playback.tick(start + Duration::from_secs(3), &ShowView::new(&sequences, &cues, &fixtures));
    apply(&mut fixtures, &effects);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.75).abs() < 0.001);

    let effects = playback.tick(start + Duration::from_secs(4), &ShowView::new(&sequences, &cues, &fixtures));
    apply(&mut fixtures, &effects);
    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 1.0);
    assert!(!playback.has_work(), "a finished fade is dropped");
}

#[test]
fn a_fade_starts_from_where_the_fixture_already_is() {
    let mut fixture = a_fixture();
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(0.5));
    let cue = a_cue(1000, vec![intensity(fixture.id, 1.0)]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));
    let effects = playback.tick(start + Duration::from_millis(500), &ShowView::new(&sequences, &cues, &fixtures));

    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.75).abs() < 0.001);
}

#[test]
fn re_cueing_mid_fade_picks_up_from_the_value_on_stage() {
    let fixture = a_fixture();
    let slow = a_cue(4000, vec![intensity(fixture.id, 1.0)]);
    let snap = a_cue(0, vec![intensity(fixture.id, 0.0)]);
    let fixtures = [fixture.clone()];
    let cues = [slow.clone(), snap.clone()];
    let mut sequences = [a_sequence(&[&slow, &snap], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));

    // Half way up, take the next cue. The old fade must not keep running.
    let half = start + Duration::from_secs(2);
    playback.tick(half, &ShowView::new(&sequences, &cues, &fixtures));
    sequences[0].active_cue_index = Some(1);
    let effects = playback.tick(half, &ShowView::new(&sequences, &cues, &fixtures));

    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 0.0);
    assert!(!playback.has_work());
}

#[test]
fn a_capture_delay_holds_the_parameter_before_it_moves() {
    let fixture = a_fixture();
    let mut capture = intensity(fixture.id, 1.0);
    capture.delay_in_ms = 1000;
    capture.fade_in_ms = 1000;
    let cue = a_cue(0, vec![capture]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let view = ShowView::new(&sequences, &cues, &fixtures);

    playback.tick(start, &view);
    let effects = playback.tick(start + Duration::from_millis(500), &view);
    assert!(live(&effects, fixture.id, "Intensity").is_none(), "nothing moves during the delay");

    let effects = playback.tick(start + Duration::from_millis(1500), &view);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.5).abs() < 0.001);
}

#[test]
fn a_capture_fade_time_overrides_the_cue_s() {
    let fixture = a_fixture();
    let mut fast = intensity(fixture.id, 1.0);
    fast.fade_in_ms = 1000;
    let cue = a_cue(10_000, vec![fast]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let view = ShowView::new(&sequences, &cues, &fixtures);
    playback.tick(start, &view);
    let effects = playback.tick(start + Duration::from_millis(1000), &view);

    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 1.0);
}

#[test]
fn fading_one_parameter_leaves_the_others_where_they_were() {
    let mut fixture = a_fixture();
    fixture.live_values.insert("Pan".into(), ParameterValue::Float(0.3));
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures));

    assert_eq!(as_float(live(&effects, fixture.id, "Pan")), 0.3);
    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 1.0);
}

#[test]
fn an_unchanged_fixture_is_not_written_again() {
    let fixture = a_fixture();
    let cue = a_cue(1000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));
    apply(&mut fixtures, &effects);

    // Same instant, so nothing has moved.
    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));
    assert!(
        !effects.iter().any(|e| matches!(e, PlaybackEffect::SetLiveValues { .. })),
        "an unmoved fixture must not be rewritten every tick",
    );
}

// ── Colour and other parameter kinds ──────────────────────────────────────────

#[test]
fn colour_fades_channel_by_channel() {
    let mut fixture = a_fixture();
    fixture
        .live_values
        .insert("ColorRgb".into(), ParameterValue::Color { r: 0.0, g: 0.0, b: 0.0 });
    let capture = ParameterCapture {
        fixture_id: fixture.id,
        parameter_kind: ParameterKind::ColorRgb,
        value: ParameterValue::Color { r: 1.0, g: 0.5, b: 0.0 },
        fade_in_ms: 1000,
        fade_out_ms: 0,
        delay_in_ms: 0,
    };
    let cue = a_cue(0, vec![capture]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let view = ShowView::new(&sequences, &cues, &fixtures);
    playback.tick(start, &view);
    let effects = playback.tick(start + Duration::from_millis(500), &view);

    match live(&effects, fixture.id, "ColorRgb") {
        Some(ParameterValue::Color { r, g, b }) => {
            assert!((r - 0.5).abs() < 0.001);
            assert!((g - 0.25).abs() < 0.001);
            assert!(b.abs() < 0.001);
        }
        other => panic!("expected a colour, got {other:?}"),
    }
}

#[test]
fn a_boolean_switches_at_the_top_of_the_fade_not_the_end() {
    let mut fixture = a_fixture();
    fixture.live_values.insert("Raw:5".into(), ParameterValue::Bool(false));
    let capture = ParameterCapture {
        fixture_id: fixture.id,
        parameter_kind: ParameterKind::Raw(5),
        value: ParameterValue::Bool(true),
        fade_in_ms: 4000,
        fade_out_ms: 0,
        delay_in_ms: 0,
    };
    let cue = a_cue(0, vec![capture]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let view = ShowView::new(&sequences, &cues, &fixtures);
    playback.tick(start, &view);
    let effects = playback.tick(start + Duration::from_millis(1), &view);

    assert_eq!(live(&effects, fixture.id, "Raw:5"), Some(ParameterValue::Bool(true)));
}

#[test]
fn raw_channels_get_their_own_live_value_keys() {
    assert_eq!(parameter_key(&ParameterKind::Raw(5)), "Raw:5");
    assert_ne!(parameter_key(&ParameterKind::Raw(5)), parameter_key(&ParameterKind::Raw(6)));
    assert_eq!(parameter_key(&ParameterKind::Intensity), "Intensity");
}

// ── Follow cues ───────────────────────────────────────────────────────────────

#[test]
fn a_follow_cue_fires_after_the_fade_plus_its_delay() {
    let fixture = a_fixture();
    let mut first = a_cue(2000, vec![intensity(fixture.id, 1.0)]);
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 1000 };
    let second = a_cue(0, vec![intensity(fixture.id, 0.0)]);
    let fixtures = [fixture.clone()];
    let cues = [first.clone(), second.clone()];
    let sequences = [a_sequence(&[&first, &second], Some(0))];
    let view = ShowView::new(&sequences, &cues, &fixtures);
    let seq_id = sequences[0].id;

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick(start, &view);

    let effects = playback.tick(start + Duration::from_millis(2500), &view);
    assert!(
        !effects.contains(&PlaybackEffect::GoNext { sequence_id: seq_id }),
        "the delay is measured from the end of the fade, not the start",
    );

    let effects = playback.tick(start + Duration::from_millis(3100), &view);
    assert!(effects.contains(&PlaybackEffect::GoNext { sequence_id: seq_id }));
}

#[test]
fn a_follow_fires_once() {
    let fixture = a_fixture();
    let mut first = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 0 };
    let second = a_cue(0, vec![]);
    let fixtures = [fixture];
    let cues = [first.clone(), second.clone()];
    let sequences = [a_sequence(&[&first, &second], Some(0))];
    let view = ShowView::new(&sequences, &cues, &fixtures);
    let seq_id = sequences[0].id;

    let mut playback = Playback::default();
    let start = Instant::now();

    // No fade and no delay, so it is due on the tick that takes the cue.
    let first_tick = playback.tick(start, &view);
    assert!(first_tick.contains(&PlaybackEffect::GoNext { sequence_id: seq_id }));

    let second_tick = playback.tick(start + Duration::from_millis(10), &view);
    assert!(!second_tick.contains(&PlaybackEffect::GoNext { sequence_id: seq_id }));
}

#[test]
fn taking_a_cue_by_hand_cancels_a_pending_follow() {
    let fixture = a_fixture();
    let mut first = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 5000 };
    let second = a_cue(0, vec![]);
    let fixtures = [fixture];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));

    sequences[0].active_cue_index = Some(1);
    playback.tick(start + Duration::from_millis(10), &ShowView::new(&sequences, &cues, &fixtures));

    let effects = playback.tick(start + Duration::from_secs(10), &ShowView::new(&sequences, &cues, &fixtures));
    assert!(
        !effects.iter().any(|e| matches!(e, PlaybackEffect::GoNext { .. })),
        "the follow belonged to a cue that is no longer running",
    );
}

#[test]
fn a_manual_cue_never_fires_a_follow() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let fixtures = [fixture];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let view = ShowView::new(&sequences, &cues, &fixtures);

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick(start, &view);
    let effects = playback.tick(start + Duration::from_secs(60), &view);

    assert!(!effects.iter().any(|e| matches!(e, PlaybackEffect::GoNext { .. })));
}

// ── Idling ────────────────────────────────────────────────────────────────────

#[test]
fn an_idle_show_reports_no_work() {
    let fixtures: [Fixture; 0] = [];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];

    let mut playback = Playback::default();
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures));

    assert!(effects.is_empty());
    assert!(!playback.has_work());
}

#[test]
fn a_deleted_sequence_releases_its_cue() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let fixtures = [fixture];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures));

    let none: [Sequence; 0] = [];
    let effects = playback.tick(start, &ShowView::new(&none, &cues, &fixtures));

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: cue.id, is_active: false }));
}
