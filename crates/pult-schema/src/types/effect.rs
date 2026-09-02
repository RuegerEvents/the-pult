//! One periodic primitive.
//!
//! Everything the console has driven so far holds still until something moves it: a
//! cue fades a value from where it was to where it should be, and then nothing
//! happens. An effect is the other kind of instruction — a shape the value keeps
//! tracing, indefinitely, with no further messages.
//!
//! # Why one type covers sine and chase both
//!
//! A sine on intensity and a red-green-blue chase look like different features, and
//! on most consoles they are. They differ only in what a cycle position maps to: a
//! shape reads a level out of a function and scales it between [`EffectSpec::low`] and
//! [`EffectSpec::high`], while a step list looks the position up in its own
//! keyframes, which carry real values. Everything else — how fast, which way, where in
//! the cycle this fixture sits — is the same question in both cases, so it is asked
//! once, in one envelope.
//!
//! # Why phase is a number and not a rule
//!
//! [`EffectSpec::phase`] is this fixture's absolute offset into the cycle, worked out
//! when the effect was applied. [`Spread`] records *how* the operator asked for the
//! phases so the GUI can re-apply the same shape to a different selection, and the
//! engine never reads it. That keeps rendering a pure function of one entry rather
//! than of an entry plus its position in a selection that may since have changed.
//!
//! # Determinism
//!
//! Every station renders the same effect independently and they must agree. The
//! inputs are the spec, the speed master it may follow, the cue's `went_at`, and the
//! wall clock; all but the clock are replicated, and nothing accumulates per station.
//! A tempo change rewrites bpm and anchor together, so it is a bounded phase step
//! rather than a drift that grows.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fixture::ParameterValue;

// The shapes, the curves and what is actually running live in `pult-render`, because
// the browser has to evaluate them too and cannot depend on this crate. Re-exported
// here under the paths they have always had, so nothing else in the workspace moves.
pub use pult_render::effect::{
    blend, curve_level, cycle_position, ease, effect_value_at, fade_is_done, fade_progress,
    fade_value_at, step_value, Curve, Direction, Easing, EffectSource, RunningEffect, RunningFade,
    Shape, Step,
};

/// How fast, either said outright or borrowed from a speed master.
///
/// Stays here rather than moving to the evaluator: resolving a master into a rate
/// needs the `speed_masters` collection, which is show data. What the evaluator is
/// given is a [`RunningEffect`], whose rate is already a number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Rate {
    Hz(f32),
    /// The master supplies the tempo and the anchor; `multiplier` is this effect's own
    /// ratio on top of the master's.
    Master { id: Uuid, multiplier: f32 },
}

impl Default for Rate {
    fn default() -> Self {
        Rate::Hz(1.0)
    }
}

/// How per-fixture phases were derived from the selection order.
///
/// Kept so the GUI can re-apply the same arrangement to a new selection. The engine
/// never reads it: by the time an effect is running, the phase is already a number —
/// which is also why it stays here and not in the evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Spread {
    /// Every fixture in step.
    #[default]
    Even,
    /// One cycle spread across the selection.
    Linear,
    /// Symmetric about the middle of the selection.
    Centre,
    /// Linear, from the other end.
    Reversed,
    /// Mirrored in `n` wings.
    Wings(u8),
    /// `n` groups, each in step with itself.
    Groups(u8),
    Random { seed: u32 },
}

/// A periodic instruction for one fixture parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectSpec {
    /// Shared by every capture and entry that came from one apply, so the GUI can
    /// gather a selection's worth of specs back into one editable effect.
    pub effect_id: Uuid,
    pub curve: Curve,
    pub rate: Rate,
    /// The bottom of a shape's travel. Ignored by a step list, which carries values.
    pub low: ParameterValue,
    /// The top of a shape's travel. Ignored by a step list.
    pub high: ParameterValue,
    /// Duty cycle for [`Shape::Square`], 0..1.
    pub width: f32,
    pub direction: Direction,
    /// This fixture's offset into the cycle, 0..1.
    pub phase: f32,
    pub spread: Spread,
    /// Console unix ms that phase 0 is anchored to.
    ///
    /// `Some` in the programmer, set when the operator applied it. `None` in a stored
    /// capture, where the anchor is the cue's `went_at` and so is decided afresh on
    /// every Go. A [`Rate::Master`] ignores this either way: the master's own anchor
    /// is what keeps every effect on one master in step.
    pub t0: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step chase carries real values, not levels, which is the whole reason
    /// `Curve` is an enum rather than a shape plus a flag.
    #[test]
    fn a_step_list_round_trips_with_its_own_values() {
        let spec = EffectSpec {
            effect_id: Uuid::nil(),
            curve: Curve::Steps(vec![
                Step {
                    at: 0.0,
                    value: ParameterValue::rgb(1.0, 0.0, 0.0),
                    easing: Easing::Step,
                },
                Step {
                    at: 0.5,
                    value: ParameterValue::rgb(0.0, 1.0, 0.0),
                    easing: Easing::Linear,
                },
            ]),
            rate: Rate::Master { id: Uuid::nil(), multiplier: 2.0 },
            low: ParameterValue::Float(0.0),
            high: ParameterValue::Float(1.0),
            width: 0.5,
            direction: Direction::Backward,
            phase: 0.75,
            spread: Spread::Wings(2),
            t0: Some(1_756_550_400_123),
        };

        let back: EffectSpec = serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
        assert_eq!(back, spec);

        let Curve::Steps(steps) = back.curve else { panic!("still a step list") };
        assert_eq!(steps[1].value, ParameterValue::rgb(0.0, 1.0, 0.0));
        assert_eq!(steps[0].easing, Easing::Step, "a hard chase holds and jumps");
    }

    /// Externally tagged, like `FixturePosition` and `FixtureAddress`, so the wire
    /// form names the variant rather than carrying a discriminant field.
    #[test]
    fn the_enums_name_their_variant_on_the_wire() {
        assert_eq!(serde_json::to_value(Rate::Hz(0.5)).unwrap(), serde_json::json!({ "Hz": 0.5 }));
        assert_eq!(
            serde_json::to_value(Curve::Shape(Shape::SawUp)).unwrap(),
            serde_json::json!({ "Shape": "SawUp" }),
        );
        assert_eq!(
            serde_json::to_value(Spread::Groups(4)).unwrap(),
            serde_json::json!({ "Groups": 4 }),
        );
        assert_eq!(
            serde_json::to_value(EffectSource::Cue(Uuid::nil())).unwrap(),
            serde_json::json!({ "Cue": "00000000-0000-0000-0000-000000000000" }),
        );
    }

    #[test]
    fn a_running_effect_and_a_running_fade_round_trip() {
        let effect = RunningEffect {
            effect_id: Uuid::nil(),
            curve: Curve::Shape(Shape::Triangle),
            rate_hz: 1.5,
            low: ParameterValue::Float(0.2),
            high: ParameterValue::Float(0.9),
            width: 0.5,
            direction: Direction::Forward,
            phase: 0.5,
            t0: 1_000,
            source: EffectSource::Programmer,
        };
        let back: RunningEffect =
            serde_json::from_value(serde_json::to_value(&effect).unwrap()).unwrap();
        assert_eq!(back, effect);

        let fade = RunningFade {
            from: ParameterValue::Float(0.0),
            to: ParameterValue::Float(1.0),
            t0: 2_000,
            duration_ms: 3_000,
            easing: Easing::EaseInOut,
            cue_id: Uuid::nil(),
        };
        let back: RunningFade = serde_json::from_value(serde_json::to_value(&fade).unwrap()).unwrap();
        assert_eq!(back, fade);
    }

    /// The defaults matter: they are what an older showfile and an unset field both
    /// read back as.
    #[test]
    fn the_defaults_are_the_unsurprising_ones() {
        assert_eq!(Easing::default(), Easing::Linear);
        assert_eq!(Direction::default(), Direction::Forward);
        assert_eq!(Spread::default(), Spread::Even);
        assert_eq!(Rate::default(), Rate::Hz(1.0));
    }
}
