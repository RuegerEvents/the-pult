//! The numeric table, from the node's side.
//!
//! Deliberately the same numbers as `pult-backend`'s `model/effects/tests.rs` and the
//! firmware's `test_curve.c`, and deliberately not the same code. Two implementations
//! that agree because they were both written from the protocol documents prove the
//! documents are unambiguous; two that agree because they share a module prove
//! nothing at all. If one of these three suites is changed and the others are not,
//! what happens on the bench is a node drifting out of phase with the console driving
//! it, which is a great deal harder to find than a red test.

use super::*;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn assert_close(got: f32, want: f32, what: &str) {
    assert!(close(got, want), "{what}: got {got}, want {want}");
}

fn level(v: &Value) -> f32 {
    v["value"].as_f64().expect("a number payload") as f32
}

// ── Shapes ────────────────────────────────────────────────────────────────────

#[test]
fn a_sine_starts_halfway_up_and_peaks_a_quarter_in() {
    assert_close(curve_level("sine", 0.5, 0.0), 0.5, "start");
    assert_close(curve_level("sine", 0.5, 0.25), 1.0, "peak");
    assert_close(curve_level("sine", 0.5, 0.5), 0.5, "back through the middle");
    assert_close(curve_level("sine", 0.5, 0.75), 0.0, "trough");
}

#[test]
fn a_triangle_rises_over_the_first_half_and_falls_over_the_second() {
    assert_close(curve_level("triangle", 0.5, 0.0), 0.0, "start");
    assert_close(curve_level("triangle", 0.5, 0.25), 0.5, "half way up");
    assert_close(curve_level("triangle", 0.5, 0.5), 1.0, "peak");
    assert_close(curve_level("triangle", 0.5, 0.75), 0.5, "half way down");
}

#[test]
fn a_square_spends_width_of_its_cycle_high() {
    assert_close(curve_level("square", 0.5, 0.0), 1.0, "on at the top");
    assert_close(curve_level("square", 0.5, 0.49), 1.0, "still on");
    assert_close(curve_level("square", 0.5, 0.5), 0.0, "off at half");
    assert_close(curve_level("square", 0.25, 0.24), 1.0, "narrow, on");
    assert_close(curve_level("square", 0.25, 0.26), 0.0, "narrow, off");
}

#[test]
fn the_saws_run_opposite_ways() {
    assert_close(curve_level("saw-up", 0.5, 0.0), 0.0, "up starts low");
    assert_close(curve_level("saw-up", 0.5, 0.75), 0.75, "up rises");
    assert_close(curve_level("saw-down", 0.5, 0.0), 1.0, "down starts high");
    assert_close(curve_level("saw-down", 0.5, 0.75), 0.25, "down falls");
}

/// A shape this node has never heard of reads flat rather than wildly. It should
/// never be sent one — the console asks what the port can trace first — but a node
/// is not entitled to assume the thing driving it is well behaved.
#[test]
fn a_shape_this_node_does_not_know_reads_flat() {
    assert_close(curve_level("spiral", 0.5, 0.3), 0.0, "flat");
}

// ── Easings ───────────────────────────────────────────────────────────────────

#[test]
fn every_easing_runs_from_nothing_to_everything() {
    for name in ["step", "linear", "ease-in", "ease-out", "ease-in-out"] {
        assert_close(ease(name, 0.0), 0.0, name);
        assert_close(ease(name, 1.0), 1.0, name);
    }
}

#[test]
fn an_easing_bends_the_way_its_name_says() {
    assert_close(ease("linear", 0.5), 0.5, "linear");
    assert!(ease("ease-in", 0.5) < 0.5, "slow off the mark");
    assert!(ease("ease-out", 0.5) > 0.5, "quick off the mark");
    assert_close(ease("ease-in-out", 0.5), 0.5, "symmetric");
    assert_close(ease("step", 0.99), 0.0, "nothing until it arrives");
    assert_close(ease("wobble", 0.5), 0.5, "an unknown name is linear");
}

// ── Cycle position ────────────────────────────────────────────────────────────

#[test]
fn a_one_hertz_cycle_takes_a_second() {
    let at = |ms| cycle_position(1.0, false, 0.0, 1_000, ms);
    assert_close(at(1_000), 0.0, "at the anchor");
    assert_close(at(1_250), 0.25, "a quarter in");
    assert_close(at(1_500), 0.5, "half");
    assert_close(at(2_000), 0.0, "round again");
}

#[test]
fn backward_runs_the_other_way() {
    assert_close(cycle_position(1.0, true, 0.0, 0, 250), 0.75, "backwards");
    assert_close(cycle_position(1.0, false, 0.0, 0, 250), 0.25, "forwards");
}

/// The console's clock and this node's do not agree perfectly, so being asked for a
/// position slightly before an anchor is ordinary. It wraps to the top of the cycle.
#[test]
fn a_position_before_the_anchor_wraps_rather_than_going_negative() {
    assert_close(cycle_position(1.0, false, 0.0, 1_000, 750), 0.75, "wrapped");
}

#[test]
fn a_node_that_has_been_up_for_hours_still_lands_on_the_right_millisecond() {
    let eight_hours = 8 * 60 * 60 * 1_000;
    assert_close(cycle_position(1.0, false, 0.0, 0, eight_hours + 250), 0.25, "a quarter in");
}

// ── Effects ───────────────────────────────────────────────────────────────────

fn a_sine(phase: f32) -> Effect {
    Effect {
        id: "fx".into(),
        shape: Some("sine".into()),
        steps: vec![],
        rate_hz: 1.0,
        phase,
        backward: false,
        width: 0.5,
        low: json!({ "value": 0.0 }),
        high: json!({ "value": 1.0 }),
        t0: 0,
    }
}

/// The reading the console pins too: a 1 Hz sine is at the top a quarter second in.
#[test]
fn a_one_hertz_sine_peaks_at_250_ms_and_troughs_at_750() {
    let effect = a_sine(0.0);
    assert_close(level(&effect.sample(250)), 1.0, "peak");
    assert_close(level(&effect.sample(750)), 0.0, "trough");
    assert_close(level(&effect.sample(0)), 0.5, "starts halfway");
}

#[test]
fn two_phases_half_a_cycle_apart_are_mirror_images() {
    for ms in [0, 125, 250, 375, 500] {
        let a = level(&a_sine(0.0).sample(ms));
        let b = level(&a_sine(0.5).sample(ms));
        assert!(close(a + b, 1.0), "at {ms} ms: {a} and {b} should sum to 1");
    }
}

#[test]
fn low_and_high_scale_the_shape_without_changing_it() {
    let effect = Effect { low: json!({ "value": 0.2 }), high: json!({ "value": 0.6 }), ..a_sine(0.0) };
    assert_close(level(&effect.sample(250)), 0.6, "peaks at high");
    assert_close(level(&effect.sample(750)), 0.2, "troughs at low");
}

/// A relay under a square wave is on for half the cycle, not for all but an instant.
#[test]
fn a_relay_under_a_square_wave_spends_half_the_cycle_on() {
    let effect = Effect {
        shape: Some("square".into()),
        low: json!({ "state": false }),
        high: json!({ "state": true }),
        ..a_sine(0.0)
    };
    assert_eq!(effect.sample(0), json!({ "state": true }));
    assert_eq!(effect.sample(250), json!({ "state": true }));
    assert_eq!(effect.sample(500), json!({ "state": false }));
    assert_eq!(effect.sample(750), json!({ "state": false }));
}

#[test]
fn a_colour_blends_channel_by_channel() {
    let effect = Effect {
        low: json!({ "r": 0, "g": 0, "b": 0 }),
        high: json!({ "r": 255, "g": 100, "b": 0 }),
        ..a_sine(0.0)
    };
    // Half way up the sine, at the start of the cycle.
    assert_eq!(effect.sample(0), json!({ "r": 128.0, "g": 50.0, "b": 0.0 }));
}

// ── Steps ─────────────────────────────────────────────────────────────────────

fn a_chase(easing: &str) -> Effect {
    Effect {
        shape: None,
        steps: vec![
            Step { at: 0.0, value: json!({ "r": 255, "g": 0, "b": 0 }), easing: easing.into() },
            Step { at: 1.0 / 3.0, value: json!({ "r": 0, "g": 255, "b": 0 }), easing: easing.into() },
            Step { at: 2.0 / 3.0, value: json!({ "r": 0, "g": 0, "b": 255 }), easing: easing.into() },
        ],
        ..a_sine(0.0)
    }
}

#[test]
fn a_hard_chase_shows_each_step_from_where_it_starts() {
    let chase = a_chase("step");
    assert_eq!(chase.sample(0), json!({ "r": 255, "g": 0, "b": 0 }), "red");
    assert_eq!(chase.sample(200), json!({ "r": 255, "g": 0, "b": 0 }), "still red");
    assert_eq!(chase.sample(400), json!({ "r": 0, "g": 255, "b": 0 }), "green");
    assert_eq!(chase.sample(700), json!({ "r": 0, "g": 0, "b": 255 }), "blue");
    assert_eq!(chase.sample(1_000), json!({ "r": 255, "g": 0, "b": 0 }), "round to red");
}

#[test]
fn a_linear_chase_crossfades_between_its_steps() {
    let value = a_chase("linear").sample(1_000 / 6);
    // A third of a second is 333.33 ms and the clock counts whole ones, so this
    // lands a fifth of a percent off halfway, and should.
    let r = value["r"].as_f64().unwrap();
    let g = value["g"].as_f64().unwrap();
    assert!((r - 128.0).abs() < 3.0, "half out of red: {r}");
    assert!((g - 128.0).abs() < 3.0, "half into green: {g}");
    assert_eq!(value["b"], 0.0, "no blue yet");
}

#[test]
fn the_last_step_wraps_round_to_the_first() {
    let value = a_chase("linear").sample(5_000 / 6);
    let b = value["b"].as_f64().unwrap();
    let r = value["r"].as_f64().unwrap();
    assert!((b - 128.0).abs() < 3.0, "half out of blue: {b}");
    assert!((r - 128.0).abs() < 3.0, "half into red: {r}");
}

#[test]
fn a_step_list_renders_the_same_however_it_was_ordered() {
    let ordered = a_chase("step");
    let shuffled = Effect {
        steps: vec![
            ordered.steps[2].clone(),
            ordered.steps[0].clone(),
            ordered.steps[1].clone(),
        ],
        ..ordered.clone()
    };
    for ms in [0, 200, 400, 700, 900] {
        assert_eq!(ordered.sample(ms), shuffled.sample(ms), "at {ms} ms");
    }
}

// ── Parsing ───────────────────────────────────────────────────────────────────

#[test]
fn a_descriptor_off_the_wire_becomes_something_renderable() {
    let effect = parse_effect(&json!({
        "id": "6f0c",
        "curve": { "shape": "sine" },
        "rate": 0.5,
        "phase": 0.25,
        "direction": "forward",
        "width": 0.5,
        "low": { "value": 0 },
        "high": { "value": 1 },
        "t0": 1_756_550_400_123i64,
    }))
    .expect("parses");

    assert_eq!(effect.shape.as_deref(), Some("sine"));
    assert_close(effect.rate_hz, 0.5, "rate");
    assert_close(effect.phase, 0.25, "phase");
    assert!(!effect.backward);
    assert_eq!(effect.t0, 1_756_550_400_123);
}

#[test]
fn a_clear_is_not_an_effect() {
    assert!(parse_effect(&json!({ "clear": true })).is_none());
    assert!(parse_effect(&json!({ "curve": {} })).is_none(), "nor is a curve of nothing");
}

/// The fast path. A `set` with no timing keys at all is what every node has always
/// received, and it must not be turned into a zero-length transition.
#[test]
fn a_plain_set_carries_no_timing() {
    assert!(parse_transition(&json!({ "value": 1.0 }), json!({ "value": 0.0 }), 0).is_none());
}

#[test]
fn a_timed_set_reaches_its_destination_at_delay_plus_fade() {
    let transition = parse_transition(
        &json!({ "value": 1.0, "fade_ms": 3_000, "delay_ms": 500, "curve": "linear", "t0": 1_000 }),
        json!({ "value": 0.0 }),
        0,
    )
    .expect("timing");

    assert_eq!(transition.t0, 1_500, "the stated start plus the delay");
    assert_eq!(transition.duration_ms, 3_000);
    assert_eq!(transition.to, json!({ "value": 1.0 }), "the timing keys are not part of the value");

    let (waiting, done) = transition.sample(1_200);
    assert_close(level(&waiting), 0.0, "still inside its delay");
    assert!(!done);

    let (half, done) = transition.sample(3_000);
    assert_close(level(&half), 0.5, "half way");
    assert!(!done);

    let (arrived, done) = transition.sample(4_500);
    assert_close(level(&arrived), 1.0, "there");
    assert!(done, "and finished");
}

// ── The console's clock ───────────────────────────────────────────────────────

#[test]
fn the_first_live_sample_is_taken_outright_and_later_ones_are_smoothed() {
    let mut clock = ClockOffset::default();
    // The console is 1000 ms ahead of this node's clock.
    clock.feed(11_000, 1, 10_000, false);
    assert_eq!(clock.offset_ms(), Some(1_000), "taken outright");

    // A sample that disagrees by 100 ms moves the estimate a fifth of the way.
    clock.feed(12_100, 2, 11_000, false);
    assert_eq!(clock.offset_ms(), Some(1_020), "smoothed, not jumped");

    clock.feed(13_100, 3, 12_000, false);
    assert_eq!(clock.offset_ms(), Some(1_036), "and again");

    assert_eq!(clock.console_now(20_000), 21_036, "which is what the shapes are timed against");
}

/// A retained message was published at an unknown time in the past, so it is a
/// starting point and never a correction: smoothing towards one would drag a good
/// estimate towards a stale number.
#[test]
fn a_retained_sample_seeds_but_never_corrects() {
    let mut clock = ClockOffset::default();
    clock.feed(11_000, 1, 10_000, true);
    assert_eq!(clock.offset_ms(), Some(1_000), "seeded");

    clock.feed(90_000, 2, 10_000, true);
    assert_eq!(clock.offset_ms(), Some(1_000), "and left alone");

    // A live one does correct it. Five hundred milliseconds out is more than one
    // step may cover, so it moves by the slew limit and would reach the rest over
    // the next few seconds.
    clock.feed(11_500, 3, 10_000, false);
    assert_eq!(clock.offset_ms(), Some(1_000 + MAX_SLEW_MS), "a live one corrects");
}

/// The broker restarted and replayed its retained clock, so `seq` went backwards.
/// Smoothing towards that would drag the estimate through a number from before the
/// gap; starting again is the only honest answer.
#[test]
fn a_sequence_that_goes_backwards_starts_the_estimate_again() {
    let mut clock = ClockOffset::default();
    clock.feed(11_000, 40, 10_000, false);
    clock.feed(12_000, 41, 11_000, false);
    assert_eq!(clock.offset_ms(), Some(1_000));

    clock.feed(20_500, 1, 20_000, false);
    assert_eq!(clock.offset_ms(), Some(500), "taken outright, not smoothed towards");
}

/// Before any sample has arrived a node has no better idea than its own clock. It
/// renders against that rather than refusing to render, and the first sample fixes it.
#[test]
fn with_no_sample_yet_the_nodes_own_clock_is_the_answer() {
    let clock = ClockOffset::default();
    assert_eq!(clock.console_now(5_000), 5_000);
    assert_eq!(clock.offset_ms(), None);
}

/// One wildly late message must not step every running effect by its whole error at
/// once. The firmware limits the same correction to the same number, and it has to:
/// two nodes on one broker correcting at different rates would drift apart from each
/// other for as long as the correction took.
#[test]
fn a_large_correction_arrives_gradually_rather_than_as_a_jolt() {
    let mut clock = ClockOffset::default();
    clock.feed(10_000, 1, 10_000, false);
    assert_eq!(clock.offset_ms(), Some(0));

    clock.feed(20_000, 2, 10_000, false);
    assert_eq!(clock.offset_ms(), Some(MAX_SLEW_MS), "a fifth of ten seconds, clamped");

    clock.feed(20_000, 3, 10_000, false);
    assert_eq!(clock.offset_ms(), Some(2 * MAX_SLEW_MS), "and again on the next");
}
