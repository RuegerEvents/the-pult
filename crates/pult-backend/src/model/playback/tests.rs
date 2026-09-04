//! Playback tests.
//!
//! A moment is passed in, so a whole act runs here in microseconds.
//!
//! What these assert changed shape with the change that removed stored values. A pass
//! publishes what is *driving* each parameter — the fades and shapes, anchored in
//! console milliseconds — and never a number. So a test takes what a pass published,
//! puts it on the rig the way the engine does ([`apply`]), and then asks what a
//! consumer would see at a moment ([`live`]). That second step is the same evaluation
//! an output connector and a browser do, which is the point: a test that asserted what
//! the engine wrote would no longer be asserting anything an operator can see.

use std::collections::HashMap;

use pult_schema::types::{
    cue::{Cue, FollowMode, ParameterCapture},
    fixture::{Fixture, FixtureAddress, ParameterKind, ParameterValue},
    programmer::ProgrammerValue,
    sequence::Sequence,
};

use pult_schema::types::effect::Easing;
use super::*;

/// Where this test file's clock starts. Any constant would do: the sequences that do
/// not carry a `went_at` anchor on whatever moment the pass was given.
const WALL: u64 = 1_700_000_000_000;

/// The view most of these tests want: a rig of [`a_fixture`]s, all of one type, and
/// a show that snaps rather than fades when it lets go.
///
/// The type matters. "Starts from dark" is true throughout these tests because the
/// type says a dimmer rests at zero — which is what the node said about its own port
/// — and not because the console assumed it. A test that wants somewhere else builds
/// its own types and calls [`ShowView::new`].
fn view<'a>(
    sequences: &'a [Sequence],
    cues: &'a [Cue],
    fixtures: &'a [Fixture],
    programmer: &'a [ProgrammerValue],
    masters: &'a [pult_schema::types::speedmaster::SpeedMaster],
) -> ShowView<'a> {
    ShowView::new(sequences, cues, fixtures, the_type(), programmer, masters, 0, curves())
}

/// Linear everywhere, which is *not* what a new show has.
///
/// Deliberately: almost every test here asserts where a fade had got to at a given
/// moment, and those are assertions about the arithmetic rather than about the
/// default. The tests that are about the default say so by building their own.
fn curves() -> FadeCurves {
    FadeCurves {
        intensity: Easing::Linear,
        position: Easing::Linear,
        color: Easing::Linear,
        beam: Easing::Linear,
        other: Easing::Linear,
    }
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// One type for every fixture these tests patch, so that a view can be built without
/// each of them carrying its own.
const FIXTURE_TYPE: Uuid = Uuid::from_u128(0x5ea75ea7_0000_0000_0000_000000000001);

/// A dimmer that can also colour, pan and tilt, resting dark and centred — which is
/// what an OpenHaunt node describes and what the console reads back.
fn the_type() -> &'static [pult_schema::types::fixture::FixtureType] {
    use pult_schema::types::fixture::{FixtureType, ParameterDefinition};
    static TYPES: std::sync::OnceLock<Vec<FixtureType>> = std::sync::OnceLock::new();
    TYPES.get_or_init(|| {
        // Intensity, then a colour across three, then pan and tilt: the implicit
        // mode's own order, which is what these channels always were.
        let at = ParameterDefinition::new;
        vec![FixtureType {
            id: FIXTURE_TYPE,
            name: "Spot".into(),
            manufacturer: "Nobody".into(),
            channel_count: 6,
            parameters: vec![
                at(ParameterKind::Intensity, ParameterValue::Float(0.0)),
                at(ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0)),
                at(ParameterKind::Pan, ParameterValue::Float(0.5)),
                at(ParameterKind::Tilt, ParameterValue::Float(0.5)),
            ],
            ..FixtureType::default()
        }]
    })
}

fn a_fixture() -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: FIXTURE_TYPE,
        address: FixtureAddress::dmx(1, 1),
        position: None,
        sensed_values: HashMap::new(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    }
}

/// A fixture already sitting at a value, the way one that has been driven is.
///
/// A parked fade rather than a stored number, because that is the only way a
/// parameter holds a value now: a fade of no length from the value to itself, which
/// evaluates to that value at every moment there is.
fn already_at(fixture: &mut Fixture, key: &str, value: ParameterValue) {
    fixture.live_fades.insert(
        key.into(),
        RunningFade {
            from: value.clone(),
            to: value,
            t0: 0,
            duration_ms: 0,
            easing: Easing::Step,
            cue_id: Uuid::nil(),
        },
    );
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
        easing: None,
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
        easing: Some(Easing::Linear),
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

/// What a parameter is putting out `after` milliseconds into this file's clock.
///
/// Reads the rig, not the pass, and that is the whole point: what a pass publishes is
/// a description, and the value only exists when somebody evaluates it. Call [`apply`]
/// first, the way the engine writes what a pass returned.
fn live(fixtures: &[Fixture], fixture_id: Uuid, key: &str, after_ms: u64) -> Option<ParameterValue> {
    live_under(fixtures, &[], fixture_id, key, after_ms)
}

/// The same question with the programmer over the top, which is where it belongs: the
/// entries are SYNCED show state that every consumer already has, so playback does not
/// republish the rig when somebody lets go of a fader.
fn live_under(
    fixtures: &[Fixture],
    programmer: &[ProgrammerValue],
    fixture_id: Uuid,
    key: &str,
    after_ms: u64,
) -> Option<ParameterValue> {
    let fixture = fixtures.iter().find(|f| f.id == fixture_id)?;
    let held = pult_schema::types::fixture::HeldByProgrammer::of(programmer);
    pult_schema::types::fixture::value_at(
        fixture,
        the_type().iter().find(|t| t.id == fixture.fixture_type_id),
        held.get(fixture_id, key),
        key,
        WALL + after_ms,
    )
}

/// The same, at a moment given as an absolute console millisecond — which is what the
/// effect tests want, because for an effect the anchor is the input everything depends
/// on rather than scaffolding.
fn live_at(fixtures: &[Fixture], fixture_id: Uuid, key: &str, at_ms: u64) -> Option<ParameterValue> {
    let fixture = fixtures.iter().find(|f| f.id == fixture_id)?;
    pult_schema::types::fixture::value_at(
        fixture,
        the_type().iter().find(|t| t.id == fixture.fixture_type_id),
        None,
        key,
        at_ms,
    )
}

/// The same, with the programmer over it.
fn live_at_under(
    fixtures: &[Fixture],
    programmer: &[ProgrammerValue],
    fixture_id: Uuid,
    key: &str,
    at_ms: u64,
) -> Option<ParameterValue> {
    let fixture = fixtures.iter().find(|f| f.id == fixture_id)?;
    let held = pult_schema::types::fixture::HeldByProgrammer::of(programmer);
    pult_schema::types::fixture::value_at(
        fixture,
        the_type().iter().find(|t| t.id == fixture.fixture_type_id),
        held.get(fixture_id, key),
        key,
        at_ms,
    )
}

fn as_float(value: Option<ParameterValue>) -> f32 {
    match value {
        Some(ParameterValue::Float(f)) => f,
        other => panic!("expected a float, got {other:?}"),
    }
}

/// Put what a pass published onto the fixtures, the way the engine does, so the rig
/// carries it into the next pass and into every reading of it.
fn apply(fixtures: &mut [Fixture], effects: &[PlaybackEffect]) {
    for effect in effects {
        match effect {
            PlaybackEffect::SetLiveFades { fixture_id, fades } => {
                if let Some(f) = fixtures.iter_mut().find(|f| f.id == *fixture_id) {
                    f.live_fades = fades.clone();
                }
            }
            PlaybackEffect::SetLiveEffects { fixture_id, effects } => {
                if let Some(f) = fixtures.iter_mut().find(|f| f.id == *fixture_id) {
                    f.live_effects = effects.clone();
                }
            }
            _ => {}
        }
    }
}

/// A pass at an absolute console millisecond, with what it published written back.
///
/// What the effect tests want: an effect's anchor is the input its whole rendering
/// depends on, and the reason two stations agree is that they are given the same one,
/// so those tests name the moment outright rather than counting from a base.
#[allow(clippy::too_many_arguments)]
fn at(
    playback: &mut Playback,
    at_ms: u64,
    fixtures: &mut Vec<Fixture>,
    sequences: &[Sequence],
    cues: &[Cue],
    programmer: &[ProgrammerValue],
    masters: &[pult_schema::types::speedmaster::SpeedMaster],
) -> Vec<PlaybackEffect> {
    let effects = {
        let view = view(sequences, cues, fixtures, programmer, masters);
        playback.pass(at_ms, &view)
    };
    apply(fixtures, &effects);
    effects
}

/// A pass, with what it published written straight back onto the rig.
///
/// Most of these tests want the two together — the engine never does one without the
/// other — and doing them in one call keeps the borrow of `fixtures` short enough that
/// a test can hold a mutable rig and read it afterwards.
#[allow(clippy::too_many_arguments)]
fn pass(
    playback: &mut Playback,
    after_ms: u64,
    fixtures: &mut Vec<Fixture>,
    sequences: &[Sequence],
    cues: &[Cue],
    programmer: &[ProgrammerValue],
    masters: &[pult_schema::types::speedmaster::SpeedMaster],
) -> Vec<PlaybackEffect> {
    let effects = {
        let view = view(sequences, cues, fixtures, programmer, masters);
        playback.pass(WALL + after_ms, &view)
    };
    apply(fixtures, &effects);
    effects
}

// ── Cue activation ────────────────────────────────────────────────────────────

#[test]
fn taking_a_cue_marks_it_active() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let sequence = a_sequence(&[&cue], Some(0));
    let mut fixtures = vec![fixture];
    let cues = [cue.clone()];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let effects = pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: cue.id, is_active: true }));
}

#[test]
fn moving_on_deactivates_the_previous_cue() {
    let fixture = a_fixture();
    let first = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let second = a_cue(0, vec![intensity(fixture.id, 0.5)]);
    let mut fixtures = vec![fixture];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);

    sequences[0].active_cue_index = Some(1);
    let effects = pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: first.id, is_active: false }));
    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: second.id, is_active: true }));
}

#[test]
fn a_cue_is_the_stack_up_to_it() {
    // A tracking stack: cue 2 brings a second light in and cue 3 says nothing about
    // it. Going back to cue 1 has to let it go — no cue at or before cue 1 has it —
    // and jumping from cue 1 to cue 3 has to bring it back, cue 2 being on the way.
    // The Theatre demo is where this was found: back from cue 5 to cue 1 left the
    // side booms on, because nothing before cue 4 mentions them.
    let a = a_fixture();
    let b = a_fixture();
    let first = a_cue(0, vec![intensity(a.id, 1.0)]);
    let second = a_cue(0, vec![intensity(b.id, 0.8)]);
    let third = a_cue(0, vec![intensity(a.id, 0.5)]);
    let mut fixtures = vec![a.clone(), b.clone()];
    let cues = [first.clone(), second.clone(), third.clone()];
    let mut sequences = [a_sequence(&[&first, &second, &third], Some(0))];
    let mut playback = Playback::default();
    let now = 0;

    for index in 0..3 {
        sequences[0].active_cue_index = Some(index);
        pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    }
    assert_eq!(live(&fixtures, b.id, "Intensity", now), Some(ParameterValue::Float(0.8)), "tracked through cue 3");
    assert_eq!(live(&fixtures, a.id, "Intensity", now), Some(ParameterValue::Float(0.5)));

    sequences[0].active_cue_index = Some(0);
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    assert_eq!(
        live(&fixtures, b.id, "Intensity", now),
        Some(ParameterValue::Float(0.0)),
        "back at cue 1, a light no cue at or before it captures goes home",
    );
    assert_eq!(live(&fixtures, a.id, "Intensity", now), Some(ParameterValue::Float(1.0)));

    sequences[0].active_cue_index = Some(2);
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    assert_eq!(
        live(&fixtures, b.id, "Intensity", now),
        Some(ParameterValue::Float(0.8)),
        "jumping over cue 2 takes its captures along",
    );
    assert_eq!(live(&fixtures, a.id, "Intensity", now), Some(ParameterValue::Float(0.5)));
}

#[test]
fn a_light_another_live_sequence_could_drive_is_not_let_go_by_a_jump_back() {
    let shared = a_fixture();
    let first = a_cue(0, vec![]);
    let second = a_cue(0, vec![intensity(shared.id, 0.8)]);
    let other = a_cue(0, vec![intensity(shared.id, 0.3)]);
    let mut fixtures = vec![shared.clone()];
    let cues = [first.clone(), second.clone(), other.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(1)), a_sequence(&[&other], Some(0))];
    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);

    sequences[0].active_cue_index = Some(0);
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    let showing = live(&fixtures, shared.id, "Intensity", now);
    assert_ne!(showing, Some(ParameterValue::Float(0.0)), "the other sequence still wants it");
}

#[test]
fn taking_the_sequence_off_deactivates_its_cue_and_lets_the_light_go() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let mut sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    sequences[0].active_cue_index = None;
    let effects = pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(effects.contains(&PlaybackEffect::SetCueActive { cue_id: cue.id, is_active: false }));
    assert_eq!(
        live(&fixtures, fixture.id, "Intensity", now),
        Some(ParameterValue::Float(0.0)),
        "no cue active means the sequence was taken off, and what it drove goes home",
    );
}

// ── Fading ────────────────────────────────────────────────────────────────────

#[test]
fn a_zero_time_cue_snaps_straight_to_its_values() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    assert_eq!(as_float(live(&fixtures, fixture.id, "Intensity", 0)), 1.0);
    assert!(!playback.has_work(), "a snap leaves nothing running");
}

#[test]
fn a_fade_moves_through_its_middle_before_reaching_the_target() {
    let fixture = a_fixture();
    let cue = a_cue(4000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;

    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(as_float(live(&fixtures, fixture.id, "Intensity", start)), 0.0, "starts from dark");

    pass(&mut playback, start + 1000, &mut fixtures, &sequences, &cues, &[], &[]);
        assert!((as_float(live(&fixtures, fixture.id, "Intensity", start + 1000)) - 0.25).abs() < 0.001);

    pass(&mut playback, start + 3000, &mut fixtures, &sequences, &cues, &[], &[]);
        assert!((as_float(live(&fixtures, fixture.id, "Intensity", start + 3000)) - 0.75).abs() < 0.001);

    pass(&mut playback, start + 4000, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(as_float(live(&fixtures, fixture.id, "Intensity", start + 4000)), 1.0);
    assert!(!playback.has_work(), "a finished fade is dropped");
}

#[test]
fn a_fade_starts_from_where_the_fixture_already_is() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "Intensity", ParameterValue::Float(0.5));
    let cue = a_cue(1000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!((as_float(live(&fixtures, fixture.id, "Intensity", start + 500)) - 0.75).abs() < 0.001);
}

#[test]
fn re_cueing_mid_fade_picks_up_from_the_value_on_stage() {
    let fixture = a_fixture();
    let slow = a_cue(4000, vec![intensity(fixture.id, 1.0)]);
    let snap = a_cue(0, vec![intensity(fixture.id, 0.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [slow.clone(), snap.clone()];
    let mut sequences = [a_sequence(&[&slow, &snap], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);

    // Half way up, take the next cue. The old fade must not keep running.
    let half = start + 2000;
    pass(&mut playback, half, &mut fixtures, &sequences, &cues, &[], &[]);
    sequences[0].active_cue_index = Some(1);
    pass(&mut playback, half, &mut fixtures, &sequences, &cues, &[], &[]);

    assert_eq!(as_float(live(&fixtures, fixture.id, "Intensity", half)), 0.0);
    assert!(!playback.has_work());
}

#[test]
fn a_capture_delay_holds_the_parameter_before_it_moves() {
    let fixture = a_fixture();
    let mut capture = intensity(fixture.id, 1.0);
    capture.delay_in_ms = 1000;
    capture.fade_in_ms = 1000;
    let cue = a_cue(0, vec![capture]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;

    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);
    assert_eq!(
        as_float(live(&fixtures, fixture.id, "Intensity", start + 500)),
        0.0,
        "the fade is described from the far side of the delay, so nothing moves inside it",
    );

    pass(&mut playback, start + 1500, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!((as_float(live(&fixtures, fixture.id, "Intensity", start + 1500)) - 0.5).abs() < 0.001);
}

#[test]
fn a_capture_fade_time_overrides_the_cue_s() {
    let fixture = a_fixture();
    let mut fast = intensity(fixture.id, 1.0);
    fast.fade_in_ms = 1000;
    let cue = a_cue(10_000, vec![fast]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 1000, &mut fixtures, &sequences, &cues, &[], &[]);

    assert_eq!(as_float(live(&fixtures, fixture.id, "Intensity", start + 1000)), 1.0);
}

#[test]
fn fading_one_parameter_leaves_the_others_where_they_were() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "Pan", ParameterValue::Float(0.3));
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    assert_eq!(as_float(live(&fixtures, fixture.id, "Pan", 0)), 0.3);
    assert_eq!(as_float(live(&fixtures, fixture.id, "Intensity", 0)), 1.0);
}

#[test]
fn an_unchanged_fixture_is_not_written_again() {
    let fixture = a_fixture();
    let cue = a_cue(1000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);

    // The fade is described once. A second pass over an unchanged show has nothing to
    // add — not even half way through, when the value it renders has plainly moved.
    let again = pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(again.is_empty(), "an unchanged description must not be written again");
    let midway = pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(midway.is_empty(), "and the fade progressing is not a change to the show");
}

// ── Split fades ───────────────────────────────────────────────────────────────

/// Two fixtures under one cue, one coming up and one going down, so a single tick
/// shows both times at once — which is the whole of what an out time is for.
#[test]
fn a_cue_with_an_out_time_takes_it_only_where_the_parameter_comes_down() {
    let rising = a_fixture();
    let mut falling = a_fixture();
    already_at(&mut falling, "Intensity", ParameterValue::Float(1.0));

    let mut cue = a_cue(1000, vec![intensity(rising.id, 1.0), intensity(falling.id, 0.0)]);
    cue.fade_out_ms = 4000;

    let mut fixtures = vec![rising.clone(), falling.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);

    // Halfway through a one-second in time.
    assert!((as_float(live(&fixtures, rising.id, "Intensity", start + 500)) - 0.5).abs() < 0.001);
    // An eighth of the way through a four-second out time, from full.
    assert!((as_float(live(&fixtures, falling.id, "Intensity", start + 500)) - 0.875).abs() < 0.001);
}

/// The property every show written before this change relies on: an unset out time
/// is not a snap, it is "this cue does not split its fade".
#[test]
fn a_cue_with_no_out_time_fades_one_way_in_both_directions() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "Intensity", ParameterValue::Float(1.0));
    let cue = a_cue(1000, vec![intensity(fixture.id, 0.0)]);

    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(
        (as_float(live(&fixtures, fixture.id, "Intensity", start + 500)) - 0.5).abs() < 0.001,
        "the in time, taken downwards",
    );
}

#[test]
fn a_captures_own_out_time_wins_over_the_cues() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let mut capture = intensity(fixture.id, 0.0);
    capture.fade_out_ms = 2000;
    let mut cue = a_cue(1000, vec![capture]);
    cue.fade_out_ms = 8000;

    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(
        (as_float(live(&fixtures, fixture.id, "Intensity", start + 500)) - 0.75).abs() < 0.001,
        "a quarter of the way through two seconds, not a sixteenth of eight",
    );
}

/// A colour going to black *looks* like a fade out and is not treated as one. There
/// is no agreed way to rank two colours, and a console that guessed at one would be
/// giving some cues a time nobody asked for.
#[test]
fn a_parameter_with_no_order_takes_the_in_time() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "ColorRgb", ParameterValue::rgb(1.0, 1.0, 1.0));
    let capture = ParameterCapture {
        fixture_id: fixture.id,
        parameter_kind: ParameterKind::ColorRgb,
        value: ParameterValue::rgb(0.0, 0.0, 0.0),
        fade_in_ms: 0,
        fade_out_ms: 0,
        delay_in_ms: 0,
        effect: None,
        easing: Some(Easing::Linear),
    };
    let mut cue = a_cue(1000, vec![capture]);
    cue.fade_out_ms = 8000;

    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);

    match live(&fixtures, fixture.id, "ColorRgb", start + 500) {
        Some(ParameterValue::Color { r, .. }) => {
            assert!((r - 0.5).abs() < 0.001, "half of one second, not of eight");
        }
        other => panic!("expected a colour, got {other:?}"),
    }
}

// ── Colour and other parameter kinds ──────────────────────────────────────────

#[test]
fn colour_fades_channel_by_channel() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "ColorRgb", ParameterValue::rgb(0.0, 0.0, 0.0));
    let capture = ParameterCapture {
        fixture_id: fixture.id,
        parameter_kind: ParameterKind::ColorRgb,
        value: ParameterValue::rgb(1.0, 0.5, 0.0),
        fade_in_ms: 1000,
        fade_out_ms: 0,
        delay_in_ms: 0,
        effect: None,
        easing: Some(Easing::Linear),
    };
    let cue = a_cue(0, vec![capture]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 500, &mut fixtures, &sequences, &cues, &[], &[]);

    match live(&fixtures, fixture.id, "ColorRgb", start + 500) {
        Some(ParameterValue::Color { r, g, b, .. }) => {
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
    already_at(&mut fixture, "Raw:5", ParameterValue::Bool(false));
    let capture = ParameterCapture {
        fixture_id: fixture.id,
        parameter_kind: ParameterKind::Raw(5),
        value: ParameterValue::Bool(true),
        fade_in_ms: 4000,
        fade_out_ms: 0,
        delay_in_ms: 0,
        effect: None,
        easing: Some(Easing::Linear),
    };
    let cue = a_cue(0, vec![capture]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    pass(&mut playback, start + 1, &mut fixtures, &sequences, &cues, &[], &[]);

    assert_eq!(live(&fixtures, fixture.id, "Raw:5", start + 1), Some(ParameterValue::Bool(true)));
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
    let mut fixtures = vec![fixture.clone()];
    let cues = [first.clone(), second.clone()];
    let sequences = [a_sequence(&[&first, &second], Some(0))];
    let seq_id = sequences[0].id;

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);

    // A follow's `at` is the moment it came due, on the same clock a fade is measured
    // against — so it advances with the tick rather than staying at the base.
    let effects = pass(&mut playback, start + 2500, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(
        !effects.iter().any(|e| matches!(e, PlaybackEffect::GoNext { sequence_id, .. } if *sequence_id == seq_id)),
        "the delay is measured from the end of the fade, not the start",
    );

    let effects = pass(&mut playback, start + 3100, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(effects.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL + 3100 }));
}

#[test]
fn a_follow_fires_once() {
    let fixture = a_fixture();
    let mut first = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 0 };
    let second = a_cue(0, vec![]);
    let mut fixtures = vec![fixture];
    let cues = [first.clone(), second.clone()];
    let sequences = [a_sequence(&[&first, &second], Some(0))];
    let seq_id = sequences[0].id;

    let mut playback = Playback::default();
    let start = 0;

    // No fade and no delay, so it is due on the tick that takes the cue.
    let first_tick = pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(first_tick.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL }));

    let second_tick = pass(&mut playback, start + 10, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(!second_tick.contains(&PlaybackEffect::GoNext { sequence_id: seq_id, at: WALL }));
}

#[test]
fn taking_a_cue_by_hand_cancels_a_pending_follow() {
    let fixture = a_fixture();
    let mut first = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 5000 };
    let second = a_cue(0, vec![]);
    let mut fixtures = vec![fixture];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);

    sequences[0].active_cue_index = Some(1);
    pass(&mut playback, start + 10, &mut fixtures, &sequences, &cues, &[], &[]);

    let effects = pass(&mut playback, start + 10000, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(
        !effects.iter().any(|e| matches!(e, PlaybackEffect::GoNext { .. })),
        "the follow belonged to a cue that is no longer running",
    );
}

#[test]
fn a_manual_cue_never_fires_a_follow() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
    let effects = pass(&mut playback, start + 60000, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(!effects.iter().any(|e| matches!(e, PlaybackEffect::GoNext { .. })));
}

// ── Idling ────────────────────────────────────────────────────────────────────

#[test]
fn an_idle_show_reports_no_work() {
    let mut fixtures: Vec<Fixture> = vec![];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];

    let mut playback = Playback::default();
    let effects = pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(effects.is_empty());
    assert!(!playback.has_work());
}

#[test]
fn a_deleted_sequence_releases_its_cue() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);

    let none: [Sequence; 0] = [];
    let effects = pass(&mut playback, start, &mut fixtures, &none, &cues, &[], &[]);

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

/// What a fixture is actually putting out, with the programmer over the top.
///
/// The programmer tests ask about the stage rather than about the effect list: a pass
/// that changes nothing rightly publishes nothing, so "what is the level now" and
/// "what did this pass write" are different questions, and this is the first one.
fn level_of(
    fixtures: &[Fixture],
    programmer: &[ProgrammerValue],
    fixture_id: Uuid,
    after_ms: u64,
) -> f32 {
    as_float(live_under(fixtures, programmer, fixture_id, "Intensity", after_ms))
}

#[test]
fn a_programmer_value_beats_the_cue_playing_under_it() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.25)];

    let mut playback = Playback::default();
    let now = 0;
        pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);
    
    assert_eq!(level_of(&fixtures, &programmer, fixture.id, now), 0.25);
}

#[test]
fn a_fade_keeps_running_under_a_held_value_and_release_lands_on_it() {
    let fixture = a_fixture();
    // Four seconds up, so the fade is plainly mid-flight when the value goes back.
    let cue = a_cue(4000, vec![intensity(fixture.id, 1.0)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.1)];

    let mut playback = Playback::default();
    let start = 0;
        pass(&mut playback, start, &mut fixtures, &sequences, &cues, &programmer, &[]);
    
    let halfway = start + 2000;
        pass(&mut playback, halfway, &mut fixtures, &sequences, &cues, &programmer, &[]);
        assert_eq!(
        level_of(&fixtures, &programmer, fixture.id, halfway),
        0.1,
        "the fade is running, but the programmer is what reaches the output",
    );

    // Let go. The cue is halfway up, so that is where the parameter belongs.
    pass(&mut playback, halfway, &mut fixtures, &sequences, &cues, &[], &[]);
        let released = level_of(&fixtures, &[], fixture.id, halfway);
    assert!(
        released > 0.4 && released < 0.6,
        "release should land on the fade, not on where it started: got {released}",
    );
}

#[test]
fn releasing_with_no_fade_puts_back_what_was_there() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let start = 0;
    // The cue lands first, with nothing in the programmer.
    pass(&mut playback, start, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(level_of(&fixtures, &[], fixture.id, start), 0.8);

    let programmer = [held_intensity(fixture.id, 0.2)];
    let later = start + 100;
        pass(&mut playback, later, &mut fixtures, &sequences, &cues, &programmer, &[]);
        assert_eq!(level_of(&fixtures, &programmer, fixture.id, later), 0.2);

    pass(&mut playback, later, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(level_of(&fixtures, &[], fixture.id, later), 0.8);
}

/// Nothing was underneath, so letting go lands on where the parameter rests. For a
/// dimmer that is dark — but because its type says so, not because the console
/// assumed a zero of the right shape.
#[test]
fn a_held_value_over_a_fixture_no_cue_has_touched_releases_to_where_it_rests() {
    let fixture = a_fixture();
    let mut fixtures = vec![fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.7)];

    let mut playback = Playback::default();
    let now = 0;
        pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);
        assert_eq!(level_of(&fixtures, &programmer, fixture.id, now), 0.7);

    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(level_of(&fixtures, &[], fixture.id, now), 0.0, "which for a dimmer is off");
}

/// The house light. Its type is a dimmer like any other and rests dark; this one
/// rests on, and letting go of it has to put it back on rather than leave the
/// audience in the dark.
#[test]
fn a_fixture_that_rests_on_releases_to_on() {
    let mut fixture = a_fixture();
    fixture.home_values.insert("Intensity".into(), ParameterValue::Float(1.0));
    let mut fixtures = vec![fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.2)];

    let mut playback = Playback::default();
    let now = 0;
        pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);
        assert_eq!(level_of(&fixtures, &programmer, fixture.id, now), 0.2, "the operator has it");

    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(level_of(&fixtures, &[], fixture.id, now), 1.0, "and letting go gives it back");
}

/// A fade on a parameter nothing has driven starts where that parameter rests, which
/// is not necessarily where a dimmer rests. A mover's tilt sits centred, and a cue
/// tilting it should not swing it up from the floor first.
#[test]
fn a_first_fade_starts_from_where_the_parameter_rests() {
    let fixture = a_fixture();
    let cue = a_cue(
        4000,
        vec![ParameterCapture {
            fixture_id: fixture.id,
            parameter_kind: ParameterKind::Tilt,
            value: ParameterValue::Float(1.0),
            fade_in_ms: 0,
            fade_out_ms: 0,
            delay_in_ms: 0,
            easing: Some(Easing::Linear),
            effect: None,
        }],
    );
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    assert_eq!(
        as_float(live(&fixtures, fixture.id, "Tilt", 0)),
        0.5,
        "centred, which is where the node said this port rests",
    );
}

#[test]
fn locking_a_value_changes_nothing_about_the_output() {
    let fixture = a_fixture();
    let fixtures = vec![fixture.clone()];
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
        pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &programmer, &[]);
        level_of(&fixtures, &programmer, fixture.id, 0)
    };

    unlocked.locked = false;
    assert_eq!(of(&unlocked), of(&locked));
}

#[test]
fn another_writer_under_a_held_key_is_covered_again() {
    let fixture = a_fixture();
    let mut fixtures = vec![fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.3)];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);

    // A flow action driving the same parameter. It knows nothing about the programmer,
    // and it does not need to: the programmer is a layer over what it wrote rather
    // than a writer racing it.
    playback.set_parameter(fixture.id, "Intensity".into(), ParameterValue::Float(0.95), WALL + now);
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);
    assert_eq!(level_of(&fixtures, &programmer, fixture.id, now), 0.3);

    // And that drive is what the value goes back to, because it is what playback is
    // showing underneath.
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    assert_eq!(level_of(&fixtures, &[], fixture.id, now), 0.95);
}

#[test]
fn a_settled_programmer_is_not_re_emitted() {
    let fixture = a_fixture();
    let mut fixtures = vec![fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_effect(fixture.id, a_sine(0.0))];

    let mut playback = Playback::default();
    let first = pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &programmer, &[]);
    assert!(!first.is_empty(), "the shape it is holding is published once");

    let again = pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &programmer, &[]);
    assert!(again.is_empty(), "nothing changed, so nothing should be written");
}

/// A plain held value is never published at all. It is a SYNCED programmer entry that
/// every consumer already has, and putting a second copy of it on the fixture would be
/// the station telling the browser what the browser told the station.
#[test]
fn a_plain_held_value_is_not_republished_onto_the_fixture() {
    let fixture = a_fixture();
    let mut fixtures = vec![fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.5)];

    let mut playback = Playback::default();
    let out = pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &programmer, &[]);

    assert!(out.is_empty(), "nothing to publish: {out:?}");
    assert_eq!(level_of(&fixtures, &programmer, fixture.id, 0), 0.5, "and it still shows");
}

/// Holding a value used to be outstanding work, because a flow action writing the
/// same key would otherwise take it for good. It is not any more: the programmer is a
/// layer evaluated over playback rather than a value racing other writers, so there is
/// nothing to keep re-asserting.
#[test]
fn holding_a_value_is_not_work_the_engine_has_to_keep_doing() {
    let fixture = a_fixture();
    let mut fixtures = vec![fixture.clone()];
    let cues: [Cue; 0] = [];
    let sequences: [Sequence; 0] = [];
    let programmer = [held_intensity(fixture.id, 0.5)];

    let mut playback = Playback::default();
    pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &programmer, &[]);
    assert!(!playback.has_work());
    assert_eq!(level_of(&fixtures, &programmer, fixture.id, 0), 0.5, "and it still holds");
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
                value: ParameterValue::rgb(1.0, 0.0, 0.0),
                fade_in_ms: 0,
                fade_out_ms: 0,
                delay_in_ms: 0,
                effect: None,
                easing: Some(Easing::Linear),
            },
        ],
    );
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.1)];

    let mut playback = Playback::default();
    pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &programmer, &[]);

    assert_eq!(level_of(&fixtures, &programmer, fixture.id, 0), 0.1);
    assert_eq!(
        live(&fixtures, fixture.id, "ColorRgb", 0),
        Some(ParameterValue::rgb(1.0, 0.0, 0.0)),
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
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    at(&mut playback, 10_000, &mut fixtures, &sequences, &cues, &[], &[]);

    // One pass, two readings. The shape was published once and the moment does the
    // rest, which is the whole of what this change did.
    let peak = as_float(live_at(&fixtures, fixture.id, "Intensity", 10_250));
    assert!((peak - 1.0).abs() < 1e-4, "peak: {peak}");
    let trough = as_float(live_at(&fixtures, fixture.id, "Intensity", 10_750));
    assert!(trough.abs() < 1e-4, "trough: {trough}");
}

/// Two stations run the same tick at different milliseconds. Anchored on the cue,
/// they still render the same value; anchored on their own arrival, they would not.
#[test]
fn two_stations_that_took_the_cue_at_different_moments_still_agree() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(10_000);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    // One station starts ticking 40 ms after the Go, the other 600 ms after.
    let mut prompt = Playback::default();
    at(&mut prompt, 10_040, &mut fixtures, &sequences, &cues, &[], &[]);
    let mut late = Playback::default();
    at(&mut late, 10_600, &mut fixtures, &sequences, &cues, &[], &[]);

    // Each station's rig carries what its own playback published.
    let mut here = fixtures.clone();
    let mut there = fixtures.clone();
    apply(&mut here, &at(&mut prompt, 11_250, &mut fixtures.clone(), &sequences, &cues, &[], &[]));
    apply(&mut there, &at(&mut late, 11_250, &mut fixtures.clone(), &sequences, &cues, &[], &[]));

    assert_eq!(
        live_at(&here, fixture.id, "Intensity", 11_250),
        live_at(&there, fixture.id, "Intensity", 11_250),
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
    let mut fixtures = vec![one.clone(), two.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    at(&mut playback, 250, &mut fixtures, &sequences, &cues, &[], &[]);

    let a = as_float(live_at(&fixtures, one.id, "Intensity", 250));
    let b = as_float(live_at(&fixtures, two.id, "Intensity", 250));
    assert!((a - 1.0).abs() < 1e-4, "the first is at the top: {a}");
    assert!(b.abs() < 1e-4, "the second is at the bottom: {b}");
}

/// An effect never arrives anywhere, which used to mean a station running one could
/// never stop ticking. It is not work any more, and that is the change: the shape was
/// described once, and a value nobody stores needs nobody to advance it.
#[test]
fn a_running_effect_is_not_outstanding_work() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let mut fixtures = vec![fixture];
    let cues = [cue];

    let mut playback = Playback::default();
    assert!(!playback.has_work(), "nothing yet");

    at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(!playback.has_work(), "and still nothing: the chase runs itself");
    assert_eq!(playback.next_deadline(), None, "so there is nothing to wake up for");
}

#[test]
fn leaving_the_cue_stops_its_effect() {
    let fixture = a_fixture();
    let first = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let second = a_cue(0, vec![intensity(fixture.id, 0.25)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(!fixtures[0].live_effects.is_empty(), "the effect is running");

    sequences[0].active_cue_index = Some(1);
    let out = at(&mut playback, 100, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(
        running_effects(&out, fixture.id).is_some_and(|e| e.is_empty()),
        "and the plugins are told it stopped: {out:?}",
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
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    // The programmer runs one half a cycle out, so it is at the top at the same moment.
    let programmer = [held_effect(fixture.id, a_sine(0.5))];

    let mut playback = Playback::default();
    let out = at(&mut playback, 750, &mut fixtures, &sequences, &cues, &programmer, &[]);

    let value = as_float(live_at(&fixtures, fixture.id, "Intensity", 750));
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
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];
    let programmer = [held_value(fixture.id, 0.3)];

    let mut playback = Playback::default();

    // The chase is running and the plugins have been told so.
    let before = at(&mut playback, 250, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(!running_effects(&before, fixture.id).unwrap().is_empty(), "running");

    // Then somebody grabs the fader.
    let after = at(&mut playback, 260, &mut fixtures, &sequences, &cues, &programmer, &[]);

    assert_eq!(
        live_at_under(&fixtures, &programmer, fixture.id, "Intensity", 260),
        Some(ParameterValue::Float(0.3)),
    );
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
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];
    let holding = [held_value(fixture.id, 0.3)];

    let mut playback = Playback::default();
    at(&mut playback, 250, &mut fixtures, &sequences, &cues, &holding, &[]);

    // Let go at three quarters of a cycle, where the sine is at the bottom.
    let out = at(&mut playback, 750, &mut fixtures, &sequences, &cues, &[], &[]);

    let value = as_float(live_at(&fixtures, fixture.id, "Intensity", 750));
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
    let mut fixtures = vec![fixture.clone()];
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
    let out = at(&mut playback, 51_250, &mut fixtures, &sequences, &cues, &[], &masters);

    let listed = running_effects(&out, fixture.id).expect("listed");
    assert!((listed["Intensity"].rate_hz - 1.0).abs() < 1e-4, "one hertz");
    assert_eq!(listed["Intensity"].t0, 1_000, "the master's anchor");
    // 51250 is a quarter of a second past a whole number of cycles from 1000.
    let value = as_float(live_at(&fixtures, fixture.id, "Intensity", 51_250));
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
    let mut fixtures = vec![fixture.clone()];
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
    let slow = master(60.0, 0);
    let before = at(&mut playback, 0, &mut fixtures, &[], &[], &programmer, &slow);
    assert!((running_effects(&before, fixture.id).unwrap()["Intensity"].rate_hz - 1.0).abs() < 1e-4);

    let fast = master(240.0, 5_000);
    let after = at(&mut playback, 1, &mut fixtures, &[], &[], &programmer, &fast);
    let listed = running_effects(&after, fixture.id).unwrap();
    assert!((listed["Intensity"].rate_hz - 4.0).abs() < 1e-4, "four hertz");
    assert_eq!(listed["Intensity"].t0, 5_000, "measured from the tap that changed it");
}

// ── What is handed to the plugins ─────────────────────────────────────────────

#[test]
fn a_fade_is_described_from_the_cues_anchor_and_stays_after_it_lands() {
    let fixture = a_fixture();
    let capture =
        ParameterCapture { delay_in_ms: 500, easing: Some(Easing::EaseInOut), ..intensity(fixture.id, 1.0) };
    let cue = a_cue(3_000, vec![capture]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(10_000);
    let cue_id = cue.id;
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let out = at(&mut playback, 10_000, &mut fixtures, &sequences, &cues, &[], &[]);

    let listed = running_fades(&out, fixture.id).expect("listed");
    let fade = &listed["Intensity"];
    assert_eq!(fade.t0, 10_500, "the anchor plus the capture's delay");
    assert_eq!(fade.duration_ms, 3_000);
    assert_eq!(fade.easing, Easing::EaseInOut);
    assert_eq!(fade.cue_id, cue_id);

    // Well past the end of it. The fade stays, because it is the only record of where
    // the parameter got to — nothing stores the number it landed on.
    let done = at(&mut playback, 14_000, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(done.is_empty(), "and nothing was written to say it had arrived: {done:?}");
    assert_eq!(
        live_at(&fixtures, fixture.id, "Intensity", 14_000),
        Some(ParameterValue::Float(1.0)),
        "which is what a landed fade evaluates to for ever after",
    );
}

/// A key that is being driven by an effect has no fade to describe, even if one is
/// still notionally running underneath: what a node needs is the one instruction that
/// is actually reaching the light.
#[test]
fn a_key_under_an_effect_is_not_also_listed_as_a_fade() {
    let fixture = a_fixture();
    let first = a_cue(3_000, vec![intensity(fixture.id, 1.0)]);
    let second = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    let mut playback = Playback::default();
    let fading = at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(!running_fades(&fading, fixture.id).unwrap().is_empty(), "the fade is listed first");

    sequences[0].active_cue_index = Some(1);
    let out = at(&mut playback, 100, &mut fixtures, &sequences, &cues, &[], &[]);

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
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue];
    let sequences = [sequence];

    let mut playback = Playback::default();
    let first = at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(running_effects(&first, fixture.id).is_some(), "said once");

    let second = at(&mut playback, 25, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(
        running_effects(&second, fixture.id).is_none(),
        "and not again on the next tick, though the value it renders has moved",
    );
    assert!(live_at(&fixtures, fixture.id, "Intensity", 25).is_some(), "the value still goes out");
}

// ── What a pass costs ────────────────────────────────────────────────────────
//
// Task 19 left this open: an effect never arrives anywhere, so a station running one
// never idled, and a chase up meant 40 Hz of writes for as long as it was up. That
// question is answered rather than measured now — a pass happens when the *show*
// changes, so a chase running costs exactly nothing until somebody takes a cue.
//
// What is left worth asserting is the shape of the work rather than its duration: how
// many writes a pass asks the engine to make, and that a second pass over an unchanged
// show asks for none.

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

/// A chase over a large rig is described once and then costs nothing.
///
/// The number this replaces was five hundred writes every twenty-five milliseconds,
/// for as long as the chase was up, on every station running the show. It is now two
/// writes per fixture once, and zero thereafter.
#[test]
fn a_running_effect_is_described_once_and_then_asks_for_nothing() {
    let (mut fixtures, cues, sequences) = a_rig_under_one_effect(500);
    let mut playback = Playback::default();

    let taken = at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);
    let descriptions = taken
        .iter()
        .filter(|e| {
            matches!(e, PlaybackEffect::SetLiveEffects { .. } | PlaybackEffect::SetLiveFades { .. })
        })
        .count();
    assert_eq!(descriptions, 1000, "one effects write and one fades write per fixture");

    for after in [25, 1_000, 60_000] {
        let out = at(&mut playback, after, &mut fixtures, &sequences, &cues, &[], &[]);
        assert!(
            out.is_empty(),
            "the shape has not changed at {after} ms, so there is nothing to say: {out:?}",
        );
    }
}

/// And the value it renders still moves, which is the whole trade: the console said
/// what is happening once, and time does the rest.
#[test]
fn the_value_moves_although_nothing_was_written() {
    let (mut fixtures, cues, sequences) = a_rig_under_one_effect(4);
    let mut playback = Playback::default();
    at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);
    let id = fixtures[0].id;

    // A 1 Hz sine anchored at 0: half at the start, the top a quarter in, the bottom
    // three quarters in. Nothing was written between these three readings.
    assert!((as_float(live_at(&fixtures, id, "Intensity", 0)) - 0.5).abs() < 1e-4);
    assert!((as_float(live_at(&fixtures, id, "Intensity", 250)) - 1.0).abs() < 1e-4);
    assert!(as_float(live_at(&fixtures, id, "Intensity", 750)).abs() < 1e-4);
}

/// A rig that is holding still asks for nothing however large it is, and the engine
/// has nothing left to wake up for.
#[test]
fn a_rig_that_is_holding_still_asks_for_nothing() {
    let mut fixtures: Vec<Fixture> = (0..500).map(|_| a_fixture()).collect();
    let captures = fixtures.iter().map(|f| intensity(f.id, 0.5)).collect();
    let cue = a_cue(0, captures);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];

    let mut playback = Playback::default();
    at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    let out = at(&mut playback, 25, &mut fixtures, &sequences, &cues, &[], &[]);
    assert!(out.is_empty(), "a still rig asks for nothing: {} effects", out.len());
    assert!(!playback.has_work(), "and there is nothing to wake the engine for");
    assert_eq!(playback.next_deadline(), None, "not even a deadline to sleep on");
}

// ── Taking a sequence off ─────────────────────────────────────────────────────
//
// The act the console did not have. Everything the sequence could drive and nothing
// else is still driving goes to where it rests — worked out from the show, so a
// station that joined at the interval releases exactly what one that ran the whole
// act releases.

/// The same view with the show asking for a fade home rather than a snap.
fn view_fading_home<'a>(
    sequences: &'a [Sequence],
    cues: &'a [Cue],
    fixtures: &'a [Fixture],
    programmer: &'a [ProgrammerValue],
    home_fade_ms: u32,
) -> ShowView<'a> {
    ShowView::new(sequences, cues, fixtures, the_type(), programmer, &[], home_fade_ms, curves())
}

#[test]
fn taking_a_sequence_off_puts_what_it_was_driving_back() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let mut sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(level_of(&fixtures, &[], fixture.id, now), 0.8);

    sequences[0].active_cue_index = None;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    assert_eq!(level_of(&fixtures, &[], fixture.id, now), 0.0, "back where a dimmer rests");
}

/// A sequence taken off must not reach into another one's fixture. The rule is
/// deliberately conservative: a parameter another live sequence *could* drive is left
/// alone, whether or not that sequence has reached the cue that drives it.
#[test]
fn a_parameter_another_live_sequence_could_drive_is_left_alone() {
    let ours = a_fixture();
    let theirs = a_fixture();
    let mine = a_cue(0, vec![intensity(ours.id, 0.8), intensity(theirs.id, 0.8)]);
    let yours = a_cue(0, vec![intensity(theirs.id, 0.4)]);
    let mut fixtures = vec![ours.clone(), theirs.clone()];
    let cues = [mine.clone(), yours.clone()];
    let mut sequences = [a_sequence(&[&mine], Some(0)), a_sequence(&[&yours], Some(0))];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    sequences[0].active_cue_index = None;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    assert_eq!(level_of(&fixtures, &[], ours.id, now), 0.0, "only this sequence had it");
    assert_eq!(
        level_of(&fixtures, &[], theirs.id, now),
        0.4,
        "the other sequence is still on and still has it",
    );
}

/// The operator's hands beat a release like they beat everything else. What is
/// underneath is by then the home value, so clearing afterwards lands there.
#[test]
fn a_held_parameter_is_not_taken_off_with_the_sequence() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let mut sequences = [a_sequence(&[&cue], Some(0))];
    let programmer = [held_intensity(fixture.id, 0.6)];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);
        assert_eq!(level_of(&fixtures, &programmer, fixture.id, now), 0.6);

    sequences[0].active_cue_index = None;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &programmer, &[]);
        assert_eq!(level_of(&fixtures, &programmer, fixture.id, now), 0.6, "still the operator's");

    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
        assert_eq!(level_of(&fixtures, &[], fixture.id, now), 0.0, "and letting go lands on home");
}

/// Opening a show is not taking every sequence off. Nothing is active when a
/// showfile loads — `active_cue_index` is SYNCED, not persisted — and a release on
/// the first tick would put a rig somewhere nobody asked for.
#[test]
fn a_show_that_was_never_on_releases_nothing() {
    let mut fixture = a_fixture();
    already_at(&mut fixture, "Intensity", ParameterValue::Float(0.3));
    let cue = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let sequences = [a_sequence(&[&cue], None)];

    let mut playback = Playback::default();
    let effects = pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    assert!(effects.is_empty(), "nothing transitioned, so nothing was released: {effects:?}");
    assert_eq!(
        live(&fixtures, fixture.id, "Intensity", 0),
        Some(ParameterValue::Float(0.3)),
        "and the fixture is left exactly where it was found",
    );
}

/// A station that joined at the interval never ran act one. It releases what the
/// show says the sequence could drive, which is the same set the console that ran it
/// releases — the whole reason the set is read from the cues rather than remembered.
#[test]
fn a_station_that_never_ran_the_earlier_cues_releases_the_same_parameters() {
    let fixture = a_fixture();
    let first = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let second = a_cue(0, vec![]);
    let mut fixtures = vec![fixture.clone()];
    already_at(&mut fixtures[0], "Intensity", ParameterValue::Float(0.8));
    let cues = [first.clone(), second.clone()];
    // On the second cue, and this playback has never seen the first one run.
    let mut sequences = [a_sequence(&[&first, &second], Some(1))];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);

    sequences[0].active_cue_index = None;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    assert_eq!(
        level_of(&fixtures, &[], fixture.id, now),
        0.0,
        "the first cue's parameter went home even though this station never ran it",
    );
}

/// A show that asks for a fade home gets one. Sampled a sixth of the way in, so a
/// loaded machine would have to overrun by two and a half seconds to fool it.
#[test]
fn a_show_with_a_home_time_fades_home() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity(fixture.id, 0.6)]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let mut sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let home = |playback: &mut Playback, after: u64, fixtures: &mut Vec<Fixture>, sequences: &[Sequence]| {
        let effects = {
            let view = view_fading_home(sequences, &cues, fixtures, &[], 3000);
            playback.pass(WALL + after, &view)
        };
        apply(fixtures, &effects);
    };

    home(&mut playback, 0, &mut fixtures, &sequences);
    assert_eq!(level_of(&fixtures, &[], fixture.id, 0), 0.6);

    sequences[0].active_cue_index = None;
    home(&mut playback, 0, &mut fixtures, &sequences);
    assert_eq!(
        level_of(&fixtures, &[], fixture.id, 0),
        0.6,
        "the fade has not begun to move yet",
    );

    // No further pass: the fade home was described once, and the moment does the rest.
    let sixth = level_of(&fixtures, &[], fixture.id, 500);
    assert!(sixth < 0.6 && sixth > 0.0, "part of the way home, not there: {sixth}");
    assert_eq!(level_of(&fixtures, &[], fixture.id, 3000), 0.0, "and it arrives");
}

/// A sequence taken off stops asserting its effects too. An effect has nowhere to
/// arrive, so leaving one running under a release would be a rig that never lets go.
#[test]
fn taking_a_sequence_off_stops_its_effects() {
    use pult_schema::types::effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread};

    let fixture = a_fixture();
    let mut capture = intensity(fixture.id, 0.5);
    capture.effect = Some(EffectSpec {
        effect_id: Uuid::new_v4(),
        curve: Curve::Shape(Shape::Sine),
        rate: Rate::Hz(1.0),
        low: ParameterValue::Float(0.0),
        high: ParameterValue::Float(1.0),
        width: 0.5,
        direction: Direction::Forward,
        phase: 0.0,
        spread: Spread::Even,
        t0: None,
    });
    let cue = a_cue(0, vec![capture]);
    let mut fixtures = vec![fixture.clone()];
    let cues = [cue.clone()];
    let mut sequences = [a_sequence(&[&cue], Some(0))];

    let mut playback = Playback::default();
    let now = 0;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    sequences[0].active_cue_index = None;
    pass(&mut playback, now, &mut fixtures, &sequences, &cues, &[], &[]);
    
    assert_eq!(level_of(&fixtures, &[], fixture.id, now), 0.0, "home, not still tracing a sine");
    assert!(!playback.has_work(), "and nothing is left running");
}

/// Two stations, one of which has been up all evening and one of which has just
/// joined, arriving at the same rig from one act.
///
/// This is the property the whole release rule is shaped around. Playback is a pure
/// function of replicated state, so the two differ only in what each has *watched*
/// happen — and the release must not depend on that, or the second console would put
/// the rig somewhere the first one did not.
#[test]
fn two_stations_release_to_the_same_rig() {
    let fixture = a_fixture();
    let first = a_cue(0, vec![intensity(fixture.id, 0.8)]);
    let second = a_cue(0, vec![intensity(fixture.id, 0.4)]);
    let cues = [first.clone(), second.clone()];
    let mut sequences = [a_sequence(&[&first, &second], Some(0))];

    // The console that ran the act.
    let mut ran_it = Playback::default();
    let mut here = vec![fixture.clone()];
    let now = 0;
    let effects = pass(&mut ran_it, now, &mut here, &sequences, &cues, &[], &[]);
    apply(&mut here, &effects);
    sequences[0].active_cue_index = Some(1);
    let effects = pass(&mut ran_it, now, &mut here, &sequences, &cues, &[], &[]);
    apply(&mut here, &effects);

    // The console that walked in during cue two, with the show as it now stands.
    let mut just_arrived = Playback::default();
    let mut there = here.clone();
    let effects = pass(&mut just_arrived, now, &mut there, &sequences, &cues, &[], &[]);
    apply(&mut there, &effects);

    sequences[0].active_cue_index = None;
    let effects = pass(&mut ran_it, now, &mut here, &sequences, &cues, &[], &[]);
    apply(&mut here, &effects);
    let effects = pass(&mut just_arrived, now, &mut there, &sequences, &cues, &[], &[]);
    apply(&mut there, &effects);

    assert_eq!(level_of(&here, &[], fixture.id, now), 0.0);
    assert_eq!(
        level_of(&here, &[], fixture.id, now),
        level_of(&there, &[], fixture.id, now),
        "one rig, whatever each console watched happen",
    );
}

// ── What shape a fade has ────────────────────────────────────────────────────

/// The curve a parameter is actually fading on, after a pass.
fn curve_of(fixtures: &[Fixture], fixture_id: Uuid, key: &str) -> Easing {
    fixtures
        .iter()
        .find(|f| f.id == fixture_id)
        .and_then(|f| f.live_fades.get(key))
        .unwrap_or_else(|| panic!("nothing is fading {key}"))
        .easing
}

/// A cue with a curve on it and a position capture that names none.
fn a_position_cue(fixture_id: Uuid, cue_curve: Option<Easing>, capture_curve: Option<Easing>) -> Cue {
    let capture = ParameterCapture {
        fixture_id,
        parameter_kind: ParameterKind::Pan,
        value: ParameterValue::Float(1.0),
        fade_in_ms: 0,
        fade_out_ms: 0,
        delay_in_ms: 0,
        effect: None,
        easing: capture_curve,
    };
    Cue { easing: cue_curve, ..a_cue(4_000, vec![capture]) }
}

/// The view the three tests below run under, so that each of the three answers is a
/// different named curve and no two of them could be confused.
fn view_with_curves<'a>(
    sequences: &'a [Sequence],
    cues: &'a [Cue],
    fixtures: &'a [Fixture],
) -> ShowView<'a> {
    ShowView::new(
        sequences,
        cues,
        fixtures,
        the_type(),
        &[],
        &[],
        0,
        FadeCurves { position: Easing::EaseOut, ..curves() },
    )
}

#[test]
fn a_capture_with_no_curve_of_its_own_takes_the_cues() {
    let fixture = a_fixture();
    let cue = a_position_cue(fixture.id, Some(Easing::EaseIn), None);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];
    let mut fixtures = vec![fixture.clone()];

    let effects = {
        let view = view_with_curves(&sequences, &cues, &fixtures);
        Playback::default().pass(WALL, &view)
    };
    apply(&mut fixtures, &effects);

    assert_eq!(curve_of(&fixtures, fixture.id, "Pan"), Easing::EaseIn);
}

#[test]
fn a_cue_with_no_curve_of_its_own_takes_the_shows_for_that_group() {
    let fixture = a_fixture();
    // Nothing between the parameter and the show, which is what every cue nobody has
    // opened this control on looks like.
    let cue = a_position_cue(fixture.id, None, None);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];
    let mut fixtures = vec![fixture.clone()];

    let effects = {
        let view = view_with_curves(&sequences, &cues, &fixtures);
        Playback::default().pass(WALL, &view)
    };
    apply(&mut fixtures, &effects);

    assert_eq!(
        curve_of(&fixtures, fixture.id, "Pan"),
        Easing::EaseOut,
        "the show's answer for position, and not its answer for everything",
    );
}

#[test]
fn a_captures_own_curve_beats_both() {
    let fixture = a_fixture();
    let cue = a_position_cue(fixture.id, Some(Easing::EaseIn), Some(Easing::Step));
    let sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];
    let mut fixtures = vec![fixture.clone()];

    let effects = {
        let view = view_with_curves(&sequences, &cues, &fixtures);
        Playback::default().pass(WALL, &view)
    };
    apply(&mut fixtures, &effects);

    assert_eq!(curve_of(&fixtures, fixture.id, "Pan"), Easing::Step);
}

#[test]
fn going_home_takes_the_shows_curve_too() {
    // A release is a move, and a head letting go of a mark moves the way the show
    // says heads move. Nothing above it can say otherwise: no cue is doing it, so
    // the curve the cue ran on does not follow the parameter home.
    let fixture = a_fixture();
    let cue = a_position_cue(fixture.id, Some(Easing::EaseIn), Some(Easing::Step));
    let mut sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];
    let mut fixtures = vec![fixture.clone()];

    let mut playback = Playback::default();
    let mut run = |playback: &mut Playback, at: u64, fixtures: &mut Vec<Fixture>, sequences: &[Sequence]| {
        let effects = {
            let view = ShowView::new(
                sequences,
                &cues,
                fixtures,
                the_type(),
                &[],
                &[],
                // A home time, or the release lands at once and has no shape to have.
                2_000,
                FadeCurves { position: Easing::EaseOut, ..curves() },
            );
            playback.pass(at, &view)
        };
        apply(fixtures, &effects);
    };

    run(&mut playback, WALL, &mut fixtures, &sequences);
    // Long enough that the pan has arrived, so letting go has somewhere to travel
    // from: a release that is already home writes no fade at all.
    run(&mut playback, WALL + 5_000, &mut fixtures, &sequences);
    assert_eq!(curve_of(&fixtures, fixture.id, "Pan"), Easing::Step, "the cue's, while it runs");

    sequences[0].active_cue_index = None;
    run(&mut playback, WALL + 5_000, &mut fixtures, &sequences);

    assert_eq!(
        curve_of(&fixtures, fixture.id, "Pan"),
        Easing::EaseOut,
        "the cue's curve went with the cue",
    );
}

// ── The numbers did not move ─────────────────────────────────────────────────

/// The non-goal of the whole change, written as a test: the same fade produces the
/// same number at the same instant. If a single frame differs, that is a bug in this
/// change rather than a new behaviour.
///
/// The figures below were taken from the console before values stopped being stored —
/// a four-second ease-in-out from dark to full, sampled every half second, which is
/// the arithmetic `Playback::tick` used to run once per 25 ms and write down.
#[test]
fn a_cue_fade_gives_the_numbers_it_always_gave() {
    let fixture = a_fixture();
    let capture = ParameterCapture { easing: Some(Easing::EaseInOut), ..intensity(fixture.id, 1.0) };
    let cue = a_cue(4_000, vec![capture]);
    let sequences = [a_sequence(&[&cue], Some(0))];
    let cues = [cue];
    let mut fixtures = vec![fixture.clone()];

    let mut playback = Playback::default();
    pass(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    // t/4000 through an ease-in-out: 2t² below the halfway mark, 1 - 2(1-t)² above.
    let expected = [
        (0, 0.0),
        (500, 0.031_25),
        (1_000, 0.125),
        (1_500, 0.281_25),
        (2_000, 0.5),
        (2_500, 0.718_75),
        (3_000, 0.875),
        (3_500, 0.968_75),
        (4_000, 1.0),
        (9_000, 1.0),
    ];
    for (at, want) in expected {
        let got = as_float(live(&fixtures, fixture.id, "Intensity", at));
        assert!((got - want).abs() < 1e-5, "at {at} ms: expected {want}, got {got}");
    }
}

/// And the same for a cue effect, which was already evaluated this way — so this is
/// the control: if these numbers had moved, the move would be in the arithmetic
/// rather than in what stopped storing it.
#[test]
fn a_cue_effect_gives_the_numbers_it_always_gave() {
    let fixture = a_fixture();
    let cue = a_cue(0, vec![intensity_effect(fixture.id, a_sine(0.0))]);
    let mut sequence = a_sequence(&[&cue], Some(0));
    sequence.went_at = Some(0);
    let sequences = [sequence];
    let cues = [cue];
    let mut fixtures = vec![fixture.clone()];

    let mut playback = Playback::default();
    at(&mut playback, 0, &mut fixtures, &sequences, &cues, &[], &[]);

    // 0.5 + 0.5·sin(2πx) at one hertz: half at the start, the top a quarter in, half
    // again at the half, the bottom three quarters in.
    for (at_ms, want) in [(0u64, 0.5f32), (250, 1.0), (500, 0.5), (750, 0.0), (1_000, 0.5)] {
        let got = as_float(live_at(&fixtures, fixture.id, "Intensity", at_ms));
        assert!((got - want).abs() < 1e-5, "at {at_ms} ms: expected {want}, got {got}");
    }
}
