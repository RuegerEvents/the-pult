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

/// The wall clock the value-and-fade tests tick at.
///
/// Fixed, and any constant would do: none of those sequences carries a `went_at`, so
/// the anchor falls back to whatever wall clock the tick was given and every fade
/// measures from `now` exactly as it did before there was a wall clock at all. The
/// effect tests below call `tick` directly, because for them the wall clock is the
/// thing under test.
const WALL: u64 = 1_700_000_000_000;

impl Playback {
    fn tick_at(&mut self, now: Instant, view: &ShowView<'_>) -> Vec<PlaybackEffect> {
        self.tick(now, WALL, view)
    }
}

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
    let effects = playback.tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    sequences[0].active_cue_index = Some(1);
    let effects = playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    let effects = playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    apply(&mut fixtures, &effects);

    sequences[0].active_cue_index = None;
    let effects = playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    let effects = playback.tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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

    let effects = playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(as_float(live(&effects, fixture.id, "Intensity")), 0.0, "starts from dark");

    let effects = playback.tick_at(start + Duration::from_secs(1), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    apply(&mut fixtures, &effects);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.25).abs() < 0.001);

    let effects = playback.tick_at(start + Duration::from_secs(3), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    apply(&mut fixtures, &effects);
    assert!((as_float(live(&effects, fixture.id, "Intensity")) - 0.75).abs() < 0.001);

    let effects = playback.tick_at(start + Duration::from_secs(4), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
    playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    let effects = playback.tick_at(start + Duration::from_millis(500), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    // Half way up, take the next cue. The old fade must not keep running.
    let half = start + Duration::from_secs(2);
    playback.tick_at(half, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    sequences[0].active_cue_index = Some(1);
    let effects = playback.tick_at(half, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);

    playback.tick_at(start, &view);
    let effects = playback.tick_at(start + Duration::from_millis(500), &view);
    assert!(live(&effects, fixture.id, "Intensity").is_none(), "nothing moves during the delay");

    let effects = playback.tick_at(start + Duration::from_millis(1500), &view);
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);
    playback.tick_at(start, &view);
    let effects = playback.tick_at(start + Duration::from_millis(1000), &view);

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
    let effects = playback.tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    let effects = playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    apply(&mut fixtures, &effects);

    // Same instant, so nothing has moved.
    let effects = playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);
    playback.tick_at(start, &view);
    let effects = playback.tick_at(start + Duration::from_millis(500), &view);

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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);
    playback.tick_at(start, &view);
    let effects = playback.tick_at(start + Duration::from_millis(1), &view);

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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);
    let seq_id = sequences[0].id;

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick_at(start, &view);

    let effects = playback.tick_at(start + Duration::from_millis(2500), &view);
    assert!(
        !effects.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL }),
        "the delay is measured from the end of the fade, not the start",
    );

    let effects = playback.tick_at(start + Duration::from_millis(3100), &view);
    assert!(effects.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL }));
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);
    let seq_id = sequences[0].id;

    let mut playback = Playback::default();
    let start = Instant::now();

    // No fade and no delay, so it is due on the tick that takes the cue.
    let first_tick = playback.tick_at(start, &view);
    assert!(first_tick.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL }));

    let second_tick = playback.tick_at(start + Duration::from_millis(10), &view);
    assert!(!second_tick.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL }));
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
    playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    sequences[0].active_cue_index = Some(1);
    playback.tick_at(start + Duration::from_millis(10), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let effects = playback.tick_at(start + Duration::from_secs(10), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
    let view = ShowView::new(&sequences, &cues, &fixtures, &[], &[]);

    let mut playback = Playback::default();
    let start = Instant::now();
    playback.tick_at(start, &view);
    let effects = playback.tick_at(start + Duration::from_secs(60), &view);

    assert!(!effects.iter().any(|e| matches!(e, PlaybackEffect::GoNext { .. })));
}

// ── Idling ────────────────────────────────────────────────────────────────────

#[test]
fn an_idle_show_reports_no_work() {
    let fixtures: [Fixture; 0] = [];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];

    let mut playback = Playback::default();
    let effects = playback.tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

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
    playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let none: [Sequence; 0] = [];
    let effects = playback.tick_at(start, &ShowView::new(&none, &cues, &fixtures, &[], &[]));

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
        playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
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
        playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);

    let halfway = start + Duration::from_millis(2000);
    let effects =
        playback.tick_at(halfway, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(
        level_of(&fixtures, fixture.id),
        0.1,
        "the fade is running, but the programmer is what reaches the output",
    );

    // Let go. The cue is halfway up, so that is where the parameter belongs.
    let effects = playback.tick_at(halfway, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
    let effects = playback.tick_at(start, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.8);

    let programmer = [held_intensity(fixture.id, 0.2)];
    let later = start + Duration::from_millis(100);
    let effects =
        playback.tick_at(later, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.2);

    let effects = playback.tick_at(later, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
        playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.7);

    let effects = playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
            .tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
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
        playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);

    // A flow action, or an input off a device: it writes `live_values` directly and
    // knows nothing about the programmer.
    fixtures[0].live_values.insert("Intensity".into(), ParameterValue::Float(0.95));

    let effects =
        playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);
    assert_eq!(level_of(&fixtures, fixture.id), 0.3);

    // And that write is what the value goes back to, because it is what playback
    // would be showing now.
    let effects = playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
        playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);
    assert!(!effects.is_empty());

    let effects =
        playback.tick_at(now, &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
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
    playback.tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    assert!(playback.has_work());

    playback.tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
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
        .tick_at(Instant::now(), &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]));
    apply(&mut fixtures, &effects);

    assert_eq!(level_of(&fixtures, fixture.id), 0.1);
    assert_eq!(
        fixtures[0].live_values.get("ColorRgb"),
        Some(&ParameterValue::Color { r: 1.0, g: 0.0, b: 0.0 }),
        "the colour was never in the programmer, so the cue still owns it",
    );
}

// ── Effects ───────────────────────────────────────────────────────────────────
//
// These call `tick` directly with an explicit wall clock, because for an effect the
// wall clock is not scaffolding: it is the input the whole rendering depends on, and
// the reason two stations agree is that they are given the same one.

use pult_schema::types::{
    effect::{Curve, Direction, EffectSource, EffectSpec, Rate, RunningEffect, Shape, Spread},
    speedmaster::SpeedMaster,
};

/// A 1 Hz sine from dark to full, with no anchor of its own.
fn a_sine(phase: f32) -> EffectSpec {
    EffectSpec {
        effect_id: Uuid::nil(),
        curve: Curve::Shape(Shape::Sine),
        rate: Rate::Hz(1.0),
        low: ParameterValue::Float(0.0),
        high: ParameterValue::Float(1.0),
        width: 0.5,
        direction: Direction::Forward,
        phase,
        spread: Spread::Even,
        t0: None,
    }
}

fn intensity_effect(fixture_id: Uuid, spec: EffectSpec) -> ParameterCapture {
    ParameterCapture { effect: Some(spec), ..intensity(fixture_id, 0.0) }
}

fn held_effect(fixture_id: Uuid, spec: EffectSpec) -> ProgrammerValue {
    ProgrammerValue {
        id: Uuid::new_v4(),
        fixture_id,
        parameter_kind: ParameterKind::Intensity,
        value: ParameterValue::Float(0.0),
        effect: Some(spec),
        locked: false,
    }
}

fn held_value(fixture_id: Uuid, value: f32) -> ProgrammerValue {
    ProgrammerValue {
        id: Uuid::new_v4(),
        fixture_id,
        parameter_kind: ParameterKind::Intensity,
        value: ParameterValue::Float(value),
        effect: None,
        locked: false,
    }
}

fn running_effects(
    effects: &[PlaybackEffect],
    fixture_id: Uuid,
) -> Option<HashMap<String, RunningEffect>> {
    effects.iter().rev().find_map(|e| match e {
        PlaybackEffect::SetLiveEffects { fixture_id: f, effects } if *f == fixture_id => {
            Some(effects.clone())
        }
        _ => None,
    })
}

fn running_fades(
    effects: &[PlaybackEffect],
    fixture_id: Uuid,
) -> Option<HashMap<String, RunningFade>> {
    effects.iter().rev().find_map(|e| match e {
        PlaybackEffect::SetLiveFades { fixture_id: f, fades } if *f == fixture_id => {
            Some(fades.clone())
        }
        _ => None,
    })
}

/// The cue's `went_at` is the anchor, so the sine is at the top a quarter second
/// after the Go and at the bottom three quarters after — on every station, whichever
/// millisecond each of them happened to run this tick.
#[test]
fn a_cue_effect_is_measured_from_when_the_cue_went() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(10_000);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];
    let view = || ShowView::new(&sequences, &cues, &fixtures, &[], &[]);

    let mut playback = Playback::default();
    let now = Instant::now();

    let peak = playback.tick(now, 10_250, &view());
    assert!((as_float(live(&peak, fixture.id, "Intensity")) - 1.0).abs() < 1e-4, "peak");

    let trough = playback.tick(now, 10_750, &view());
    assert!(as_float(live(&trough, fixture.id, "Intensity")).abs() < 1e-4, "trough");
}

/// Two stations run the same tick at different milliseconds. Anchored on the cue,
/// they still render the same value; anchored on their own arrival, they would not.
#[test]
fn two_stations_that_took_the_cue_at_different_moments_still_agree() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(10_000);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    // One station starts ticking 40 ms after the Go, the other 600 ms after.
    let mut prompt = Playback::default();
    prompt.tick(Instant::now(), 10_040, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    let mut late = Playback::default();
    late.tick(Instant::now(), 10_600, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    // Now both are asked for the same instant.
    let a = prompt.tick(Instant::now(), 11_250, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    let b = late.tick(Instant::now(), 11_250, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    assert_eq!(
        live(&a, fixture.id, "Intensity"),
        live(&b, fixture.id, "Intensity"),
        "the same cue at the same instant is the same value",
    );
}

#[test]
fn two_fixtures_half_a_cycle_apart_are_mirror_images() {
    let one = a_fixture();
    let two = a_fixture();
    let cue = a_cue(
        0,
        vec![intensity_effect(one.id, a_sine(0.0)), intensity_effect(two.id, a_sine(0.5))],
    );
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    let fixtures = [one.clone(), two.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let out = playback.tick(Instant::now(), 250, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let a = as_float(live(&out, one.id, "Intensity"));
    let b = as_float(live(&out, two.id, "Intensity"));
    assert!((a - 1.0).abs() < 1e-4, "the first is at the top: {a}");
    assert!(b.abs() < 1e-4, "the second is at the bottom: {b}");
}

/// An effect never arrives anywhere, so the engine can never stop ticking while one
/// is running. Without this the show would freeze at whatever the last tick rendered.
#[test]
fn a_running_effect_is_outstanding_work() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let fixtures = [fixture];
    let cues = [cue];

    let mut playback = Playback::default();
    assert!(!playback.has_work(), "nothing yet");

    playback.tick(Instant::now(), 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    assert!(playback.has_work(), "and now there is, for as long as it runs");
}

#[test]
fn leaving_the_cue_stops_its_effect() {
    let fixture = a_fixture();
    let first = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let second = a_cue(0, vec![intensity(fixture.id, 0.25)]);
    let fixtures = [fixture.clone()];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let now = Instant::now();
    playback.tick(now, 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    assert!(playback.has_work(), "the effect is running");

    sequences[0].active_cue_index = Some(1);
    let out = playback.tick(now, 100, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    assert!(
        running_effects(&out, fixture.id).is_some_and(|e| e.is_empty()),
        "and the plugins are told it stopped",
    );
}

// ── Precedence ────────────────────────────────────────────────────────────────

/// The whole rule, in one test. The overlay writes last, so what it holds wins; and
/// what it holds may be a value, which is how an operator takes a light out of a
/// chase by grabbing its fader.
#[test]
fn a_programmer_effect_beats_a_cue_effect_and_a_plain_value_beats_both() {
    let fixture = a_fixture();
    // The cue runs a sine that is at the bottom at 750 ms.
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    // The programmer runs one half a cycle out, so it is at the top at the same moment.
    let programmer = [held_effect(fixture.id, a_sine(0.5))];

    let mut playback = Playback::default();
    let out = playback.tick(
        Instant::now(),
        750,
        &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]),
    );

    let value = as_float(live(&out, fixture.id, "Intensity"));
    assert!((value - 1.0).abs() < 1e-4, "the programmer's effect is what shows: {value}");

    let listed = running_effects(&out, fixture.id).expect("listed");
    assert_eq!(listed["Intensity"].source, EffectSource::Programmer, "and it is the one named");
    assert!((listed["Intensity"].phase - 0.5).abs() < 1e-4);
}

/// Grabbing a fader over a chase stops the chase on that light. The plugins have to
/// hear about that as an *absence*, because absence is what tells a node to stop
/// tracing a shape it was handed and start taking values again.
#[test]
fn a_plain_programmer_value_covers_a_cue_effect_and_unlists_it() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];
    let programmer = [held_value(fixture.id, 0.3)];

    let mut playback = Playback::default();
    let now = Instant::now();

    // The chase is running and the plugins have been told so.
    let before = playback.tick(now, 250, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    assert!(!running_effects(&before, fixture.id).unwrap().is_empty(), "running");

    // Then somebody grabs the fader.
    let after = playback.tick(
        now,
        260,
        &ShowView::new(&sequences, &cues, &fixtures, &programmer, &[]),
    );

    assert_eq!(live(&after, fixture.id, "Intensity"), Some(ParameterValue::Float(0.3)));
    assert!(
        running_effects(&after, fixture.id).is_some_and(|e| e.is_empty()),
        "nothing is periodic on this fixture as far as a node is concerned",
    );
}

/// Releasing puts the cue's effect back, and puts it back *where it has got to* —
/// not where it was when the operator grabbed the fader. That is the same rule fades
/// already followed, and it is why a look can be built during a chase.
#[test]
fn releasing_a_held_value_gives_the_cue_effect_back() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];
    let holding = [held_value(fixture.id, 0.3)];

    let mut playback = Playback::default();
    let now = Instant::now();
    playback.tick(now, 250, &ShowView::new(&sequences, &cues, &fixtures, &holding, &[]));

    // Let go at three quarters of a cycle, where the sine is at the bottom.
    let out = playback.tick(now, 750, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let value = as_float(live(&out, fixture.id, "Intensity"));
    assert!(value.abs() < 1e-4, "back on the effect, at the trough it has reached: {value}");
    let listed = running_effects(&out, fixture.id).expect("listed again");
    assert!(matches!(listed["Intensity"].source, EffectSource::Cue(_)));
}

// ── Speed masters ─────────────────────────────────────────────────────────────

#[test]
fn an_effect_on_a_master_takes_the_masters_tempo_and_anchor() {
    let fixture = a_fixture();
    let master_id = Uuid::new_v4();
    let spec = EffectSpec {
        rate: Rate::Master { id: master_id, multiplier: 1.0 },
        ..a_sine(0.0)
    };
    let cue = a_cue(0, vec![intensity_effect(fixture.id, spec)]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(50_000);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    // 120 bpm halved is 1 Hz, anchored at 1000 rather than at the cue's 50000.
    let masters = [SpeedMaster {
        id: master_id,
        name: "Chases".into(),
        bpm: 120.0,
        multiplier: 0.5,
        running: true,
        t0: 1_000,
    }];

    let mut playback = Playback::default();
    let out = playback.tick(
        Instant::now(),
        51_250,
        &ShowView::new(&sequences, &cues, &fixtures, &[], &masters),
    );

    let listed = running_effects(&out, fixture.id).expect("listed");
    assert!((listed["Intensity"].rate_hz - 1.0).abs() < 1e-4, "one hertz");
    assert_eq!(listed["Intensity"].t0, 1_000, "the master's anchor");
    // 51250 is a quarter of a second past a whole number of cycles from 1000.
    let value = as_float(live(&out, fixture.id, "Intensity"));
    assert!((value - 1.0).abs() < 1e-4, "at the top: {value}");
}

/// A tempo edit is picked up on the next tick without anything going looking for the
/// entries that named the master, because the rate is resolved every tick rather
/// than stored resolved.
#[test]
fn editing_the_master_re_resolves_every_effect_on_it() {
    let fixture = a_fixture();
    let master_id = Uuid::new_v4();
    let spec = EffectSpec { rate: Rate::Master { id: master_id, multiplier: 1.0 }, ..a_sine(0.0) };
    let programmer = [held_effect(fixture.id, spec)];
    let fixtures = [fixture.clone()];
    let master = |bpm: f32, t0: u64| {
        [SpeedMaster {
            id: master_id,
            name: "Chases".into(),
            bpm,
            multiplier: 1.0,
            running: true,
            t0,
        }]
    };

    let mut playback = Playback::default();
    let now = Instant::now();
    let slow = master(60.0, 0);
    let before = playback.tick(now, 0, &ShowView::new(&[], &[], &fixtures, &programmer, &slow));
    assert!((running_effects(&before, fixture.id).unwrap()["Intensity"].rate_hz - 1.0).abs() < 1e-4);

    let fast = master(240.0, 5_000);
    let after = playback.tick(now, 1, &ShowView::new(&[], &[], &fixtures, &programmer, &fast));
    let listed = running_effects(&after, fixture.id).unwrap();
    assert!((listed["Intensity"].rate_hz - 4.0).abs() < 1e-4, "four hertz");
    assert_eq!(listed["Intensity"].t0, 5_000, "measured from the tap that changed it");
}

// ── What is handed to the plugins ─────────────────────────────────────────────

#[test]
fn a_fade_is_described_from_the_cues_anchor_and_leaves_when_it_lands() {
    let fixture = a_fixture();
    let capture =
        ParameterCapture { delay_in_ms: 500, easing: Easing::EaseInOut, ..intensity(fixture.id, 1.0) };
    let cue = a_cue(3_000, vec![capture]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(10_000);
    let cue_id = cue.id;
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let now = Instant::now();
    let out = playback.tick(now, 10_000, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let listed = running_fades(&out, fixture.id).expect("listed");
    let fade = &listed["Intensity"];
    assert_eq!(fade.t0, 10_500, "the anchor plus the capture's delay");
    assert_eq!(fade.duration_ms, 3_000);
    assert_eq!(fade.easing, Easing::EaseInOut);
    assert_eq!(fade.cue_id, cue_id);

    // Well past the end of it.
    let done = playback.tick(
        now + Duration::from_millis(4_000),
        14_000,
        &ShowView::new(&sequences, &cues, &fixtures, &[], &[]),
    );
    assert!(running_fades(&done, fixture.id).is_some_and(|f| f.is_empty()), "gone");
}

/// A key that is being driven by an effect has no fade to describe, even if one is
/// still notionally running underneath: what a node needs is the one instruction that
/// is actually reaching the light.
#[test]
fn a_key_under_an_effect_is_not_also_listed_as_a_fade() {
    let fixture = a_fixture();
    let first = a_cue(3_000, vec![intensity(fixture.id, 1.0)]);
    let second = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let fixtures = [fixture.clone()];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let now = Instant::now();
    let fading = playback.tick(now, 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    assert!(!running_fades(&fading, fixture.id).unwrap().is_empty(), "the fade is listed first");

    sequences[0].active_cue_index = Some(1);
    let out = playback.tick(now, 100, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    assert!(!running_effects(&out, fixture.id).unwrap().is_empty(), "the effect is listed");
    assert!(running_fades(&out, fixture.id).unwrap().is_empty(), "and no fade beside it");
}

/// Forty times a second is too often to repeat a description that has not changed.
#[test]
fn an_unchanged_description_is_not_written_again() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    let fixtures = [fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let now = Instant::now();
    let first = playback.tick(now, 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    assert!(running_effects(&first, fixture.id).is_some(), "said once");

    let second = playback.tick(now, 25, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));
    assert!(
        running_effects(&second, fixture.id).is_none(),
        "and not again on the next tick, though the value it renders has moved",
    );
    assert!(live(&second, fixture.id, "Intensity").is_some(), "the value still goes out");
}

// ── What a tick costs ─────────────────────────────────────────────────────────
//
// Task 19 left this open: an effect never arrives anywhere, so a station running
// one never idles. Before effects existed a settled show stopped ticking entirely;
// now a show with a chase up ticks at 40 Hz for as long as it is up, on every
// station. These put a number on it rather than leaving it as a worry.
//
// Not assertions about wall-clock speed — a loaded CI box would fail those, and a
// test that fails because somebody else was compiling is a test people delete.
// They print, and the one assertion is the shape of the work rather than its
// duration: how many writes a tick asks the engine to make.

/// A rig of `n` fixtures, all under one cue, all running a sine.
fn a_rig_under_one_effect(n: usize) -> (Vec<Fixture>, Vec<Cue>, Vec<Sequence>) {
    let fixtures: Vec<Fixture> = (0..n).map(|_| a_fixture()).collect();
    let captures = fixtures
        .iter()
        .enumerate()
        .map(|(i, f)| intensity_effect(f.id, a_sine(i as f32 / n as f32)))
        .collect();
    let cue = a_cue(0, captures);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    (fixtures, vec![cue], vec![sequence])
}

/// Roughly how long one tick takes, averaged over enough of them to mean something.
fn cost_per_tick(n: usize, ticks: u32) -> std::time::Duration {
    let (fixtures, cues, sequences) = a_rig_under_one_effect(n);
    let mut playback = Playback::default();
    let now = Instant::now();

    // The first tick starts the cue and is not typical of the rest.
    playback.tick(now, 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let started = Instant::now();
    for i in 0..ticks {
        playback.tick(
            now + Duration::from_millis(i as u64 * 25),
            i as u64 * 25,
            &ShowView::new(&sequences, &cues, &fixtures, &[], &[]),
        );
    }
    started.elapsed() / ticks
}

/// The headline number: what a tick costs at a few rig sizes.
///
/// The budget is 25 ms — that is the tick interval, and a tick that takes longer
/// than the gap between ticks is a console that has fallen behind rather than a
/// console that is busy.
#[test]
fn a_tick_costs_something_worth_writing_down() {
    for n in [1, 50, 200, 500, 1000] {
        let each = cost_per_tick(n, 200);
        let share = each.as_secs_f64() / TICK.as_secs_f64() * 100.0;
        println!("{n:>5} fixtures on one effect: {each:>10.2?} per tick ({share:.2}% of the budget)");
    }
}

/// The thing task 19 actually worried about: how many writes a tick asks for.
///
/// `emit` compares against the show and drops the no-ops, so a rig holding still
/// costs nothing to keep holding. A rig under an effect does not hold still, and
/// this is the number that grows with it.
#[test]
fn a_tick_asks_for_one_write_per_moving_fixture_and_no_more() {
    let (fixtures, cues, sequences) = a_rig_under_one_effect(500);
    let mut playback = Playback::default();
    let now = Instant::now();
    playback.tick(now, 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let out = playback.tick(now, 25, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    let values = out
        .iter()
        .filter(|e| matches!(e, PlaybackEffect::SetLiveValues { .. }))
        .count();
    let descriptions = out
        .iter()
        .filter(|e| {
            matches!(e, PlaybackEffect::SetLiveEffects { .. } | PlaybackEffect::SetLiveFades { .. })
        })
        .count();

    assert_eq!(values, 500, "one value write per fixture that moved");
    assert_eq!(
        descriptions, 0,
        "and none of the descriptions again: they were said on the first tick and have not changed",
    );
}

/// The other half of the same question, and the reassuring half: a rig that is not
/// moving costs nothing per tick however large it is.
///
/// This is what makes the number above the honest worst case rather than the normal
/// one. A show is mostly still.
#[test]
fn a_rig_that_is_holding_still_asks_for_nothing() {
    let fixtures: Vec<Fixture> = (0..500).map(|_| a_fixture()).collect();
    let captures = fixtures.iter().map(|f| intensity(f.id, 0.5)).collect();
    let cue = a_cue(0, captures);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];

    let mut playback = Playback::default();
    let now = Instant::now();
    // Take the cue and let its zero-length fades land.
    playback.tick(now, 0, &ShowView::new(&sequences, &cues, &fixtures, &[], &[]));

    // The show now says what the cue asked for, so the next tick has nothing to say.
    let settled: Vec<Fixture> = fixtures
        .iter()
        .map(|f| {
            let mut f = f.clone();
            f.live_values.insert("Intensity".into(), ParameterValue::Float(0.5));
            f
        })
        .collect();
    let out = playback.tick(now, 25, &ShowView::new(&sequences, &cues, &settled, &[], &[]));

    assert!(out.is_empty(), "a still rig asks for nothing: {} effects", out.len());
    assert!(!playback.has_work(), "and the engine can stop ticking altogether");
}
