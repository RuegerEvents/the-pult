//! Resolving a stored spec into something renderable.
//!
//! The numeric table that used to live here moved to `pult-render` with the maths it
//! describes. What is left is the part that needs the show: which master an effect
//! follows, whether it is running, and what anchors an effect that carries no anchor.

use pult_schema::types::{
    effect::{Curve, Direction, EffectSource, EffectSpec, Rate, Shape, Spread},
    fixture::ParameterValue,
    speedmaster::SpeedMaster,
};
use uuid::Uuid;

use super::*;

fn assert_close(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() < 1e-4, "{what}: got {got}, want {want}");
}

fn float(v: &ParameterValue) -> f32 {
    match v {
        ParameterValue::Float(f) => *f,
        other => panic!("expected a float, got {other:?}"),
    }
}

fn a_spec(rate: Rate, t0: Option<u64>) -> EffectSpec {
    EffectSpec {
        effect_id: Uuid::nil(),
        curve: Curve::Shape(Shape::Sine),
        rate,
        low: ParameterValue::Float(0.0),
        high: ParameterValue::Float(1.0),
        width: 0.5,
        direction: Direction::Forward,
        phase: 0.0,
        spread: Spread::Even,
        t0,
    }
}

fn a_master(bpm: f32, multiplier: f32, running: bool, t0: u64) -> SpeedMaster {
    SpeedMaster { id: Uuid::nil(), name: "Chases".into(), bpm, multiplier, running, t0 }
}

#[test]
fn a_spec_with_no_anchor_takes_the_one_it_is_given() {
    let running = resolve(&a_spec(Rate::Hz(2.0), None), &[], 5_000, EffectSource::Programmer);
    assert_eq!(running.t0, 5_000, "the cue's went_at");
    assert_close(running.rate_hz, 2.0, "rate");

    let held = resolve(&a_spec(Rate::Hz(2.0), Some(9_000)), &[], 5_000, EffectSource::Programmer);
    assert_eq!(held.t0, 9_000, "its own anchor wins");
}

/// 120 bpm is two beats a second; halved, one cycle a second.
#[test]
fn a_master_at_120_bpm_halved_is_one_hertz() {
    let master = a_master(120.0, 0.5, true, 7_000);
    let spec = a_spec(Rate::Master { id: Uuid::nil(), multiplier: 1.0 }, Some(1_000));

    let running = resolve(&spec, &[master], 5_000, EffectSource::Programmer);
    assert_close(running.rate_hz, 1.0, "hz");
    assert_eq!(running.t0, 7_000, "the master's anchor, not the spec's or the cue's");
}

#[test]
fn the_effects_own_multiplier_rides_on_top_of_the_masters() {
    let master = a_master(120.0, 1.0, true, 0);
    let spec = a_spec(Rate::Master { id: Uuid::nil(), multiplier: 0.25 }, None);
    assert_close(resolve(&spec, &[master], 0, EffectSource::Programmer).rate_hz, 0.5, "hz");
}

/// Stopping a chase should freeze the look rather than turn the lights off, so a
/// stopped master renders every effect on it at its phase and leaves it there.
#[test]
fn a_stopped_master_holds_its_effects_where_they_are() {
    let master = a_master(120.0, 1.0, false, 0);
    let mut spec = a_spec(Rate::Master { id: Uuid::nil(), multiplier: 1.0 }, None);
    spec.phase = 0.25;

    let running = resolve(&spec, &[master], 0, EffectSource::Programmer);
    assert_close(running.rate_hz, 0.0, "stopped");
    assert_close(float(&value_at(&running, 500_000)), 1.0, "held at the peak, whenever we look");
}

/// A cue can outlive the master it names. Rendering nothing would stick the fixture
/// at whatever it last held, which reads as a fault; a defined default is wrong in a
/// way an operator can see.
#[test]
fn an_effect_naming_a_master_that_is_gone_still_renders() {
    let spec = a_spec(Rate::Master { id: Uuid::new_v4(), multiplier: 1.0 }, None);
    let running = resolve(&spec, &[], 3_000, EffectSource::Cue(Uuid::nil()));
    assert_close(running.rate_hz, 2.0, "120 bpm at 1.0");
    assert_eq!(running.t0, 3_000, "anchored on the cue");
}

/// The determinism claim, made concrete: a master edit rewrites tempo and anchor
/// together, so the same inputs give the same output no matter which station asks.
#[test]
fn re_resolving_after_a_tempo_edit_gives_a_new_rate_and_a_new_anchor() {
    let spec = a_spec(Rate::Master { id: Uuid::nil(), multiplier: 1.0 }, None);

    let before = resolve(&spec, &[a_master(120.0, 1.0, true, 1_000)], 0, EffectSource::Programmer);
    let after = resolve(&spec, &[a_master(60.0, 1.0, true, 4_000)], 0, EffectSource::Programmer);

    assert_close(before.rate_hz, 2.0, "before");
    assert_close(after.rate_hz, 1.0, "after");
    assert_eq!(after.t0, 4_000, "and measured from the tap that changed it");
}
