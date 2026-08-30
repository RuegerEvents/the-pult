//! Playback tests.
//!
//! Time is passed in, so a fade runs to completion here in microseconds.

use std::collections::HashMap;

use pult_schema::types::{
    cue::{Cue, FollowMode, ParameterCapture},
    fixture::{Fixture, FixtureAddress, ParameterKind, ParameterValue},
    programmer::ProgrammerValue,
    sequence::Sequence,
};

use pult_schema::types::effect::Easing;
use super::*;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn a_fixture() -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: Uuid::new_v4(),
        address: FixtureAddress::Dmx { universe: 1, address: 1 },
        position: None,
        live_values: HashMap::new(),
        live_effects: Default::default(),
        live_fades: Default::default(),
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
        effect: None,
        easing: Easing::Linear,
    }
}

fn a_sequence(cues: &[&Cue], active: Option<usize>) -> Sequence {
    Sequence {
        id: Uuid::new_v4(),
        name: "Act 1".into(),
        cue_ids: cues.iter().map(|c| c.id).collect(),
        active_cue_index: active,
        went_at: None,
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
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &[]));

    sequences[0].active_cue_index = Some(1);
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);

    sequences[0].active_cue_index = None;
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[]));

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

    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 0.0, "starts from dark");

    let effects = playback.tick(start + Duration::from_secs(1), &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.25).abs() < 0.001);

    let effects = playback.tick(start + Duration::from_secs(3), &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.75).abs() < 0.001);

    let effects = playback.tick(start + Duration::from_secs(4), &ShowView::new(&sequences, &cues, &fixtures, &[]));
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
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    let effects = playback.tick(start + Duration::from_millis(500), &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));

    // Half way up, take the next cue. The old fade must not keep running.
    let half = start + Duration::from_secs(2);
    playback.tick(half, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    sequences[0].active_cue_index = Some(1);
    let effects = playback.tick(half, &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);

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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);
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
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);

    // Same instant, so nothing has moved.
    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));
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
        effect: None,
        easing: Easing::Linear,
    };
    let cue = a_cue(0, vec![capture]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);
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
        effect: None,
        easing: Easing::Linear,
    };
    let cue = a_cue(0, vec![capture]);
    let fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);
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
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));

    sequences[0].active_cue_index = Some(1);
    playback.tick(start + Duration::from_millis(10), &ShowView::new(&sequences, &cues, &fixtures, &[]));

    let effects = playback.tick(start + Duration::from_secs(10), &ShowView::new(&sequences, &cues, &fixtures, &[]));
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[]);

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
    let effects = playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[]));

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
    playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));

    let none: [Sequence; 0] = [];
    let effects = playback.tick(start, &ShowView::new(&none, &cues, &fixtures, &[]));

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: cue.id, is_active: false }));
}

// ── The programmer ────────────────────────────────────────────────────────────

/// One value held by the programmer. The id is arbitrary here: the overlay keys on
/// the fixture and the parameter, and it is the *frontend* that derives the id from
/// those two so that two consoles converge on one row.
fn held(fixture_id: Uuid, kind: ParameterKind, value: ParameterValue) -> ProgrammerValue {
    ProgrammerValue { id: Uuid::new_v4(), fixture_id, parameter_kind: kind, value, effect: None, locked: false }
}

fn held_intensity(fixture_id: Uuid, level: f32) -> ProgrammerValue {
    held(fixture_id, ParameterKind::Intensity, ParameterValue::Float(level))
}

/// What a fixture is actually putting out, after the tick's effects were applied.
///
/// The programmer tests ask about the stage rather than about the effect list: a
/// tick that changes nothing rightly emits nothing, so "what is the level now" and
/// "what did this tick write" are different questions, and this is the first one.
fn level_of(fixtures: &[Fixture], fixture_id: Uuid) -> f32 {
    as_float(
        fixtures
            .iter()
            .find(|f| f.id == fixture_id)
            .and_then(|f| f.live_values.get("Intensity").cloned()),
    )
}

#[test]
fn a_programmer_value_beats_the_cue_playing_under_it() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.25)];

    let mut playback = Playback::default();
    let now = Instant::now();
    let effects =
        playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);

    assert_eq!(level_of(&fixtures, fixture.id), 0.25);
}

#[test]
fn a_fade_keeps_running_under_a_held_value_and_release_lands_on_it() {
    let fixture = a_fixture();
    // Four seconds up, so the fade is plainly mid-flight when the value goes back.
    let cue = a_cue(4000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.1)];

    let mut playback = Playback::default();
    let start = Instant::now();
    let effects =
        playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);

    let halfway = start + Duration::from_millis(2000);
    let effects =
        playback.tick(halfway, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);
    assert_eq!(
        level_of(&fixtures, fixture.id),
        0.1,
        "the fade is running, but the programmer is what reaches the output",
    );

    // Let go. The cue is halfway up, so that is where the parameter belongs.
    let effects = playback.tick(halfway, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    let released = level_of(&fixtures, fixture.id);
    assert!(
        released > 0.4 && released < 0.6,
        "release should land on the fade, not on where it started: got {released}",
    );
}

#[test]
fn releasing_with_no_fade_puts_back_what_was_there() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = Instant::now();
    // The cue lands first, with nothing in the programmer.
    let effects = playback.tick(start, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.8);

    let programmer = [held_intensity(fixture.id, 0.2)];
    let later = start + Duration::from_millis(100);
    let effects =
        playback.tick(later, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.2);

    let effects = playback.tick(later, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.8);
}

#[test]
fn a_held_value_over_a_fixture_no_cue_has_touched_releases_to_dark() {
    let fixture = a_fixture();
    let mut fixtures = [fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.7)];

    let mut playback = Playback::default();
    let now = Instant::now();
    let effects =
        playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.7);

    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(
        level_of(&fixtures, fixture.id),
        0.0,
        "nothing was underneath, so letting go leaves it off",
    );
}

#[test]
fn locking_a_value_changes_nothing_about_the_output() {
    let fixture = a_fixture();
    let fixtures = [fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];

    let mut unlocked = held_intensity(fixture.id, 0.4);
    let mut locked = unlocked.clone();
    locked.locked = true;

    // Parking is about what survives Clear and Store, which is a decision the
    // frontend makes. Nothing here should be able to tell the two apart.
    let of = |entry: &ProgrammerValue| {
        let programmer = [entry.clone()];
        let mut fixtures = fixtures.clone();
        let mut playback = Playback::default();
        let effects = playback
            .tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &programmer));
        apply(&mut fixtures, &effects);
        level_of(&fixtures, fixture.id)
    };

    unlocked.locked = false;
    assert_eq!(of(&unlocked), of(&locked));
}

#[test]
fn another_writer_under_a_held_key_is_covered_again() {
    let fixture = a_fixture();
    let mut fixtures = [fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.3)];

    let mut playback = Playback::default();
    let now = Instant::now();
    let effects =
        playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);

    // A flow action, or an input off a device: it writes `live_values` directly and
    // knows nothing about the programmer.
    fixtures[0].live_values.insert("Intensity".into(), ParameterValue::Float(0.95));

    let effects =
        playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.3);

    // And that write is what the value goes back to, because it is what playback
    // would be showing now.
    let effects = playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.95);
}

#[test]
fn a_settled_programmer_is_not_re_emitted_every_tick() {
    let fixture = a_fixture();
    let mut fixtures = [fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.5)];

    let mut playback = Playback::default();
    let now = Instant::now();
    let effects =
        playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);
    assert!(!effects.is_empty());

    let effects =
        playback.tick(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    assert!(effects.is_empty(), "nothing moved, so nothing should be written");
}

#[test]
fn holding_a_value_is_work_so_the_engine_keeps_ticking() {
    let fixture = a_fixture();
    let fixtures = [fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.5)];

    // Otherwise a flow action writing the same key would take it for good: that
    // write does not bump the show's version, so nothing else would ask for a tick.
    let mut playback = Playback::default();
    playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    assert!(playback.has_work());

    playback.tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[]));
    assert!(!playback.has_work(), "and stops once the programmer is empty");
}

#[test]
fn the_programmer_leaves_parameters_it_does_not_hold_alone() {
    let fixture = a_fixture();
    let cue = a_cue(
        0,
        vec![
            intensity(fixture.id, 1.0),
            ParameterCapture {
                fixture_id: fixture.id,
                parameter_kind: ParameterKind::ColorRgb,
                value: ParameterValue::Color { r: 1.0, g: 0.0, b: 0.0 },
                fade_in_ms: 0,
                fade_out_ms: 0,
                delay_in_ms: 0,
                effect: None,
                easing: Easing::Linear,
            },
        ],
    );
    let mut fixtures = [fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.1)];

    let mut playback = Playback::default();
    let effects = playback
        .tick(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &programmer));
    apply(&mut fixtures, &effects);

    assert_eq!(level_of(&fixtures, fixture.id), 0.1);
    assert_eq!(
        fixtures[0].live_values.get("ColorRgb"),
        Some(&ParameterValue::Color { r: 1.0, g: 0.0, b: 0.0 }),
        "the colour was never in the programmer, so the cue still owns it",
    );
}
