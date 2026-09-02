//! The numeric table.
//!
//! These numbers are the contract between three implementations that share no code:
//! this renderer, the simulator in `tools/openhaunt-node-sim`, and `oh_curve.c` in the
//! firmware. Both of the others repeat the same table in their own test suites. A
//! change here that is not made there shows up as a node drifting out of phase with
//! the console driving it, which is much harder to spot than a failing test.

use uuid::Uuid;

use super::*;
use crate::value::ParameterValue;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn assert_close(got: f32, want: f32, what: &str) {
    assert!(close(got, want), "{what}: got {got}, want {want}");
}

/// For readings taken at a whole millisecond that is not a whole fraction of the
/// cycle. A third of a second is 333.33 ms and the clock only counts whole ones, so
/// the crossfade tests land a fifth of a percent off and should.
fn assert_near(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() < 0.01, "{what}: got {got}, want about {want}");
}

fn float(v: &ParameterValue) -> f32 {
    match v {
        ParameterValue::Float(f) => *f,
        other => panic!("expected a float, got {other:?}"),
    }
}

// ── Shapes ────────────────────────────────────────────────────────────────────

/// A cycle starts at half and peaks a quarter in. This is the phase convention the
/// wire protocol assumes, so a node that starts its sine at zero is a bug.
#[test]
fn a_sine_starts_halfway_up_and_peaks_a_quarter_in() {
    assert_close(curve_level(Shape::Sine, 0.5, 0.0), 0.5, "start");
    assert_close(curve_level(Shape::Sine, 0.5, 0.25), 1.0, "peak");
    assert_close(curve_level(Shape::Sine, 0.5, 0.5), 0.5, "back through the middle");
    assert_close(curve_level(Shape::Sine, 0.5, 0.75), 0.0, "trough");
    assert_close(curve_level(Shape::Sine, 0.5, 1.0), 0.5, "round again");
}

#[test]
fn a_triangle_rises_over_the_first_half_and_falls_over_the_second() {
    assert_close(curve_level(Shape::Triangle, 0.5, 0.0), 0.0, "start");
    assert_close(curve_level(Shape::Triangle, 0.5, 0.25), 0.5, "half way up");
    assert_close(curve_level(Shape::Triangle, 0.5, 0.5), 1.0, "peak");
    assert_close(curve_level(Shape::Triangle, 0.5, 0.75), 0.5, "half way down");
}

#[test]
fn a_square_spends_width_of_its_cycle_high() {
    assert_close(curve_level(Shape::Square, 0.5, 0.0), 1.0, "on at the top");
    assert_close(curve_level(Shape::Square, 0.5, 0.49), 1.0, "still on");
    assert_close(curve_level(Shape::Square, 0.5, 0.5), 0.0, "off at half");

    // A quarter duty is on for a quarter of the cycle.
    assert_close(curve_level(Shape::Square, 0.25, 0.24), 1.0, "narrow, on");
    assert_close(curve_level(Shape::Square, 0.25, 0.26), 0.0, "narrow, off");
}

#[test]
fn the_saws_run_opposite_ways() {
    assert_close(curve_level(Shape::SawUp, 0.5, 0.0), 0.0, "up starts low");
    assert_close(curve_level(Shape::SawUp, 0.5, 0.75), 0.75, "up rises");
    assert_close(curve_level(Shape::SawDown, 0.5, 0.0), 1.0, "down starts high");
    assert_close(curve_level(Shape::SawDown, 0.5, 0.75), 0.25, "down falls");
}

#[test]
fn every_shape_stays_inside_its_range() {
    let shapes = [Shape::Sine, Shape::Triangle, Shape::Square, Shape::SawUp, Shape::SawDown];
    for shape in shapes {
        for step in 0..=100 {
            let level = curve_level(shape, 0.5, step as f32 / 100.0);
            assert!((0.0..=1.0).contains(&level), "{shape:?} at {step} gave {level}");
        }
    }
}

// ── Easings ───────────────────────────────────────────────────────────────────

#[test]
fn every_easing_runs_from_nothing_to_everything() {
    let easings = [Easing::Step, Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut];
    for easing in easings {
        assert_close(ease(easing, 0.0), 0.0, "start");
        assert_close(ease(easing, 1.0), 1.0, "end");
    }
}

#[test]
fn an_easing_bends_the_way_its_name_says() {
    assert_close(ease(Easing::Linear, 0.5), 0.5, "linear");
    assert!(ease(Easing::EaseIn, 0.5) < 0.5, "slow off the mark");
    assert!(ease(Easing::EaseOut, 0.5) > 0.5, "quick off the mark");
    assert_close(ease(Easing::EaseInOut, 0.5), 0.5, "symmetric");
    assert_close(ease(Easing::Step, 0.99), 0.0, "nothing until it arrives");
}

// ── Cycle position ────────────────────────────────────────────────────────────

#[test]
fn a_one_hertz_cycle_takes_a_second() {
    let at = |ms| cycle_position(1.0, Direction::Forward, 0.0, 1_000, ms);
    assert_close(at(1_000), 0.0, "at the anchor");
    assert_close(at(1_250), 0.25, "a quarter in");
    assert_close(at(1_500), 0.5, "half");
    assert_close(at(2_000), 0.0, "round again");
    assert_close(at(2_250), 0.25, "and on");
}

#[test]
fn phase_offsets_where_a_fixture_sits_in_the_cycle() {
    assert_close(cycle_position(1.0, Direction::Forward, 0.0, 0, 0), 0.0, "first");
    assert_close(cycle_position(1.0, Direction::Forward, 0.5, 0, 0), 0.5, "half behind");
}

#[test]
fn backward_runs_the_other_way() {
    assert_close(cycle_position(1.0, Direction::Backward, 0.0, 0, 250), 0.75, "backwards");
    assert_close(cycle_position(1.0, Direction::Forward, 0.0, 0, 250), 0.25, "forwards");
}

/// Clock skew between stations makes rendering a cue slightly before its own anchor
/// ordinary rather than exceptional, and it must wrap to the top of the cycle rather
/// than to a negative position.
#[test]
fn a_position_before_the_anchor_wraps_rather_than_going_negative() {
    assert_close(cycle_position(1.0, Direction::Forward, 0.0, 1_000, 750), 0.75, "wrapped");
}

/// A stopped master resolves to zero, and an effect on it holds where its phase puts
/// it rather than collapsing to the start of the cycle.
#[test]
fn a_rate_of_zero_holds_at_the_phase() {
    assert_close(cycle_position(0.0, Direction::Forward, 0.3, 0, 9_999_999), 0.3, "held");
}

/// Milliseconds are counted in i64 and only narrowed at the end. A show up for eight
/// hours is 28.8 million milliseconds from its anchor, well past where an f32 stops
/// being able to tell one millisecond from the next.
#[test]
fn a_show_that_has_been_up_for_hours_still_lands_on_the_right_millisecond() {
    let eight_hours_ms = 8 * 60 * 60 * 1_000;
    assert_close(
        cycle_position(1.0, Direction::Forward, 0.0, 0, eight_hours_ms + 250),
        0.25,
        "still a quarter in",
    );
}

// ── Values ────────────────────────────────────────────────────────────────────

fn a_sine(low: f32, high: f32, phase: f32) -> RunningEffect {
    RunningEffect {
        effect_id: Uuid::nil(),
        curve: Curve::Shape(Shape::Sine),
        rate_hz: 1.0,
        low: ParameterValue::Float(low),
        high: ParameterValue::Float(high),
        width: 0.5,
        direction: Direction::Forward,
        phase,
        t0: 0,
        source: EffectSource::Programmer,
    }
}

/// The reading the plan pins: a 1 Hz sine is at the top a quarter second in and at the
/// bottom three quarters in.
#[test]
fn a_one_hertz_sine_peaks_at_250_ms_and_troughs_at_750() {
    let effect = a_sine(0.0, 1.0, 0.0);
    assert_close(float(&effect_value_at(&effect, 250)), 1.0, "peak");
    assert_close(float(&effect_value_at(&effect, 750)), 0.0, "trough");
    assert_close(float(&effect_value_at(&effect, 0)), 0.5, "starts halfway");
}

#[test]
fn two_phases_half_a_cycle_apart_are_mirror_images() {
    let first = a_sine(0.0, 1.0, 0.0);
    let second = a_sine(0.0, 1.0, 0.5);
    for ms in [0, 125, 250, 375, 500] {
        let a = float(&effect_value_at(&first, ms));
        let b = float(&effect_value_at(&second, ms));
        assert!(close(a + b, 1.0), "at {ms} ms: {a} and {b} should sum to 1");
    }
}

#[test]
fn low_and_high_scale_the_shape_without_changing_it() {
    let effect = a_sine(0.2, 0.6, 0.0);
    assert_close(float(&effect_value_at(&effect, 250)), 0.6, "peaks at high");
    assert_close(float(&effect_value_at(&effect, 750)), 0.2, "troughs at low");
    assert_close(float(&effect_value_at(&effect, 0)), 0.4, "halfway between");
}

/// A boolean has nothing between its states, so it turns over at the halfway mark.
/// A square wave at the default duty then spends half the cycle on, which is what
/// somebody putting a chase on a relay is asking for.
#[test]
fn a_boolean_under_a_square_wave_spends_half_the_cycle_on() {
    let effect = RunningEffect {
        curve: Curve::Shape(Shape::Square),
        low: ParameterValue::Bool(false),
        high: ParameterValue::Bool(true),
        ..a_sine(0.0, 1.0, 0.0)
    };
    assert_eq!(effect_value_at(&effect, 0), ParameterValue::Bool(true));
    assert_eq!(effect_value_at(&effect, 250), ParameterValue::Bool(true));
    assert_eq!(effect_value_at(&effect, 500), ParameterValue::Bool(false));
    assert_eq!(effect_value_at(&effect, 750), ParameterValue::Bool(false));
}

// ── Steps ─────────────────────────────────────────────────────────────────────

fn rgb(r: f32, g: f32, b: f32) -> ParameterValue {
    ParameterValue::Color { r, g, b }
}

fn a_chase(easing: Easing) -> RunningEffect {
    RunningEffect {
        curve: Curve::Steps(vec![
            Step { at: 0.0, value: rgb(1.0, 0.0, 0.0), easing },
            Step { at: 1.0 / 3.0, value: rgb(0.0, 1.0, 0.0), easing },
            Step { at: 2.0 / 3.0, value: rgb(0.0, 0.0, 1.0), easing },
        ]),
        low: rgb(0.0, 0.0, 0.0),
        high: rgb(1.0, 1.0, 1.0),
        ..a_sine(0.0, 1.0, 0.0)
    }
}

/// The reason a step carries a value rather than a level: a chase can be three
/// colours, not three brightnesses of one.
#[test]
fn a_hard_chase_shows_each_step_from_where_it_starts() {
    let chase = a_chase(Easing::Step);
    assert_eq!(effect_value_at(&chase, 0), rgb(1.0, 0.0, 0.0), "red");
    assert_eq!(effect_value_at(&chase, 200), rgb(1.0, 0.0, 0.0), "still red");
    assert_eq!(effect_value_at(&chase, 400), rgb(0.0, 1.0, 0.0), "green");
    assert_eq!(effect_value_at(&chase, 700), rgb(0.0, 0.0, 1.0), "blue");
    assert_eq!(effect_value_at(&chase, 1_000), rgb(1.0, 0.0, 0.0), "round to red");
}

#[test]
fn a_linear_chase_crossfades_between_its_steps() {
    let chase = a_chase(Easing::Linear);
    // A sixth of a cycle in is halfway from the first step to the second.
    let ParameterValue::Color { r, g, b } = effect_value_at(&chase, 1_000 / 6) else { panic!("a colour") };
    assert_near(r, 0.5, "half out of red");
    assert_near(g, 0.5, "half into green");
    assert_close(b, 0.0, "no blue yet");
}

/// The last step eases into the first one, round the end of the cycle, rather than
/// having nowhere to go.
#[test]
fn the_last_step_wraps_round_to_the_first() {
    let chase = a_chase(Easing::Linear);
    let ParameterValue::Color { r, g, b } = effect_value_at(&chase, 5_000 / 6) else { panic!("a colour") };
    assert_near(b, 0.5, "half out of blue");
    assert_near(r, 0.5, "half into red");
    assert_close(g, 0.0, "green is behind us");
}

#[test]
fn a_step_list_renders_the_same_however_it_was_ordered() {
    let ordered = a_chase(Easing::Step);
    let Curve::Steps(steps) = ordered.curve.clone() else { panic!() };
    let shuffled = RunningEffect {
        curve: Curve::Steps(vec![steps[2].clone(), steps[0].clone(), steps[1].clone()]),
        ..ordered.clone()
    };
    for ms in [0, 200, 400, 700, 900] {
        assert_eq!(effect_value_at(&ordered, ms), effect_value_at(&shuffled, ms), "at {ms} ms");
    }
}

/// A step list with nothing in it has nothing to show, and holding the bottom of the
/// range is at least a defined answer rather than whatever the fixture last had.
#[test]
fn a_step_list_with_no_steps_holds_the_bottom_of_its_range() {
    let empty = RunningEffect { curve: Curve::Steps(vec![]), ..a_sine(0.25, 1.0, 0.0) };
    assert_eq!(effect_value_at(&empty, 500), ParameterValue::Float(0.25));
}


// ── Fades ─────────────────────────────────────────────────────────────────────
//
// A fade used to be measured against a monotonic `Instant` inside the engine, which
// made it the one piece of the arithmetic a browser could not reproduce. These pin
// the millisecond form to the numbers the `Instant` form gave.

fn a_fade(duration_ms: u32, easing: Easing) -> RunningFade {
    RunningFade {
        from: ParameterValue::Float(0.0),
        to: ParameterValue::Float(1.0),
        t0: 10_000,
        duration_ms,
        easing,
        cue_id: Uuid::nil(),
    }
}

#[test]
fn a_fade_runs_from_where_it_was_to_where_it_is_going() {
    let fade = a_fade(4_000, Easing::Linear);
    assert_close(float(&fade_value_at(&fade, 10_000)), 0.0, "at the start");
    assert_close(float(&fade_value_at(&fade, 11_000)), 0.25, "a quarter in");
    assert_close(float(&fade_value_at(&fade, 12_000)), 0.5, "halfway");
    assert_close(float(&fade_value_at(&fade, 14_000)), 1.0, "arrived");
}

/// Anything before the anchor is the delay, not a negative position.
#[test]
fn a_fade_holds_at_its_start_until_its_anchor() {
    let fade = a_fade(4_000, Easing::Linear);
    assert_close(float(&fade_value_at(&fade, 0)), 0.0, "long before");
    assert_close(float(&fade_value_at(&fade, 9_999)), 0.0, "a millisecond before");
    assert!(!fade_is_done(&fade, 9_999));
}

#[test]
fn a_fade_stays_arrived_once_it_is_over() {
    let fade = a_fade(4_000, Easing::Linear);
    assert!(fade_is_done(&fade, 14_000), "exactly at the end");
    assert!(fade_is_done(&fade, 900_000), "and long after");
    assert_close(float(&fade_value_at(&fade, 900_000)), 1.0, "held at the destination");
}

/// A zero-length fade is a snap, and must not divide by its own duration.
#[test]
fn a_fade_with_no_duration_arrives_immediately() {
    let fade = a_fade(0, Easing::Linear);
    assert_eq!(fade_progress(&fade, 10_000), 1.0);
    assert_close(float(&fade_value_at(&fade, 10_000)), 1.0, "snapped");
    assert!(fade_is_done(&fade, 10_000));
}

#[test]
fn a_fade_bends_the_way_its_easing_says() {
    let linear = fade_value_at(&a_fade(4_000, Easing::Linear), 11_000);
    let ease_in = fade_value_at(&a_fade(4_000, Easing::EaseIn), 11_000);
    let ease_out = fade_value_at(&a_fade(4_000, Easing::EaseOut), 11_000);
    assert!(float(&ease_in) < float(&linear), "ease-in starts slower");
    assert!(float(&ease_out) > float(&linear), "ease-out starts faster");
}

/// A show that has been up for hours is millions of milliseconds from its anchor,
/// and the position still has to land on the right one.
#[test]
fn a_fade_hours_into_a_show_still_lands_on_the_right_millisecond() {
    let mut fade = a_fade(4_000, Easing::Linear);
    fade.t0 = 6 * 60 * 60 * 1000;
    assert_close(float(&fade_value_at(&fade, fade.t0 + 1_000)), 0.25, "a quarter in");
}
