//! Rendering an effect: cycle position in, parameter value out.
//!
//! Pure, and deliberately so. Every station renders the same effect for itself from
//! replicated state plus the wall clock, so nothing here may consult a clock, hold
//! state between calls, or accumulate. Give it the same inputs on two consoles and it
//! must give the same value, or the rig will not be in step with itself.
//!
//! # The numeric table
//!
//! The shapes below are duplicated in two other places on purpose: the simulator in
//! `tools/openhaunt-node-sim` and the firmware's `oh_curve.c`. They share no code —
//! one is Rust, one is C on a chip with no floating-point unit — so what keeps them
//! honest is that all three test suites assert the same numbers. Changing a shape here
//! means changing it in three places or watching a node drift out of phase with the
//! console driving it.
//!
//! Sine is `0.5 + 0.5·sin(2πx)`, so a cycle starts at half and peaks a quarter in.
//! That is the phase convention the wire protocol assumes and the one a node has to
//! match.

use pult_schema::types::{
    effect::{
        Curve, Direction, Easing, EffectSource, EffectSpec, Rate, RunningEffect, Shape, Step,
    },
    fixture::ParameterValue,
    speedmaster::{SpeedMaster, FALLBACK_BPM},
};

use super::playback::interpolate;

/// Where in its cycle an effect is at `now_ms`, 0..1.
///
/// The millisecond arithmetic is done in `i64` and `f64` and only narrowed at the end.
/// A show that has been up for a few hours is millions of milliseconds from its
/// anchor, and an `f32` mantissa runs out of room to tell one of those milliseconds
/// from the next long before that.
pub fn cycle_position(
    rate_hz: f32,
    direction: Direction,
    phase: f32,
    t0: u64,
    now_ms: u64,
) -> f32 {
    let elapsed_ms = now_ms as i64 - t0 as i64;
    let cycles = elapsed_ms as f64 / 1000.0 * rate_hz as f64;
    let travelled = match direction {
        Direction::Forward => cycles,
        Direction::Backward => -cycles,
    };
    // `rem_euclid` rather than `%`: a cue rendered before its own anchor, which clock
    // skew between stations makes ordinary, must wrap to the top of the cycle rather
    // than to a negative position.
    (travelled + phase as f64).rem_euclid(1.0) as f32
}

/// A shape's level at a cycle position, 0..1.
pub fn curve_level(shape: Shape, width: f32, x: f32) -> f32 {
    match shape {
        Shape::Sine => 0.5 + 0.5 * (std::f32::consts::TAU * x).sin(),
        // Up over the first half, down over the second.
        Shape::Triangle => {
            if x < 0.5 {
                x * 2.0
            } else {
                2.0 - x * 2.0
            }
        }
        // `width` is the duty cycle: how much of the cycle is spent high.
        Shape::Square => {
            if x < width.clamp(0.0, 1.0) {
                1.0
            } else {
                0.0
            }
        }
        Shape::SawUp => x,
        Shape::SawDown => 1.0 - x,
    }
}

/// The shape of a transition, 0..1 in, 0..1 out.
pub fn ease(easing: Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        // Nothing in between: hold, then arrive.
        Easing::Step => {
            if t >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        Easing::Linear => t,
        Easing::EaseIn => t * t,
        Easing::EaseOut => t * (2.0 - t),
        Easing::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - 2.0 * (1.0 - t) * (1.0 - t)
            }
        }
    }
}

/// Blend `low` towards `high` by `level`.
///
/// Numbers and colours interpolate through [`interpolate`], the same arithmetic a
/// fade uses. The two that cannot be blended are decided at the halfway mark rather
/// than at the end: a boolean under a square wave should spend half its cycle on, and
/// [`interpolate`]'s rule for a fade — switch immediately, so a cue does not look
/// late — would leave it on for all but the first instant.
pub fn blend(low: &ParameterValue, high: &ParameterValue, level: f32) -> ParameterValue {
    match (low, high) {
        (ParameterValue::Bool(a), ParameterValue::Bool(b)) => {
            ParameterValue::Bool(if level >= 0.5 { *b } else { *a })
        }
        (a @ ParameterValue::Text(_), b @ ParameterValue::Text(_)) => {
            if level >= 0.5 {
                b.clone()
            } else {
                a.clone()
            }
        }
        (a, b) => interpolate(a, b, level),
    }
}

/// The value a step list is showing at a cycle position.
///
/// Steps are read in the order they are given and the one whose `at` the position has
/// most recently passed is the current one; its `easing` describes the way into the
/// step after it, wrapping round the end of the cycle. An empty list has nothing to
/// show, which is the caller's problem.
pub fn step_value(steps: &[Step], x: f32) -> Option<ParameterValue> {
    if steps.is_empty() {
        return None;
    }
    // Sorted by position rather than trusted, so a step list an operator has dragged
    // around renders the same as one built in order.
    let mut order: Vec<&Step> = steps.iter().collect();
    order.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));

    let current = order.iter().rposition(|s| x >= s.at).unwrap_or(order.len() - 1);
    let step = order[current];
    let next = order[(current + 1) % order.len()];

    if step.easing == Easing::Step || order.len() == 1 {
        return Some(step.value.clone());
    }

    // The span to the next step, wrapping past the end of the cycle.
    let mut span = next.at - step.at;
    if span <= 0.0 {
        span += 1.0;
    }
    let mut travelled = x - step.at;
    if travelled < 0.0 {
        travelled += 1.0;
    }
    let t = ease(step.easing, (travelled / span).clamp(0.0, 1.0));
    Some(blend(&step.value, &next.value, t))
}

/// What one running effect is asserting at `now_ms`.
pub fn value_at(effect: &RunningEffect, now_ms: u64) -> ParameterValue {
    let x = cycle_position(effect.rate_hz, effect.direction, effect.phase, effect.t0, now_ms);
    match &effect.curve {
        Curve::Shape(shape) => {
            blend(&effect.low, &effect.high, curve_level(*shape, effect.width, x))
        }
        // A step list that has lost its steps holds the bottom of its range, which is
        // dark for an intensity and off for a relay.
        Curve::Steps(steps) => step_value(steps, x).unwrap_or_else(|| effect.low.clone()),
    }
}

/// Turn a stored spec into something renderable: rate in Hz, anchor decided.
///
/// `fallback_t0` is what anchors an effect that carries no anchor of its own — the
/// cue's `went_at` for a stored capture. A [`Rate::Master`] ignores both and takes the
/// master's, which is the whole point of a master: every effect following it starts
/// its cycle at the same instant.
pub fn resolve(
    spec: &EffectSpec,
    masters: &[SpeedMaster],
    fallback_t0: u64,
    source: EffectSource,
) -> RunningEffect {
    let (rate_hz, t0) = match spec.rate {
        Rate::Hz(hz) => (hz, spec.t0.unwrap_or(fallback_t0)),
        Rate::Master { id, multiplier } => match masters.iter().find(|m| m.id == id) {
            Some(master) => {
                // A stopped master holds every effect on it where it is, at
                // `curve(phase)`, rather than dropping them: stopping a chase should
                // freeze the look, not turn the lights off.
                let hz = if master.running {
                    master.bpm / 60.0 * master.multiplier * multiplier
                } else {
                    0.0
                };
                (hz, master.t0)
            }
            // The master is gone: deleted, or this showfile was written somewhere
            // else. Rendering nothing would stick the fixture at whatever it last
            // held, which reads as a fault; a defined default is wrong in a way an
            // operator can see and fix.
            None => (FALLBACK_BPM / 60.0 * multiplier, spec.t0.unwrap_or(fallback_t0)),
        },
    };

    RunningEffect {
        effect_id: spec.effect_id,
        curve: spec.curve.clone(),
        rate_hz,
        low: spec.low.clone(),
        high: spec.high.clone(),
        width: spec.width,
        direction: spec.direction,
        phase: spec.phase,
        t0,
        source,
    }
}

#[cfg(test)]
mod tests;
