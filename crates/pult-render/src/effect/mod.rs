//! What is driving a parameter, and what it is asserting at a given moment.
//!
//! Pure, and deliberately so. Every consumer works the same value out for itself from
//! replicated state plus a moment, so nothing here may consult a clock, hold state
//! between calls, or accumulate. Give it the same inputs in two runtimes and it must
//! give the same value, or the screen will disagree with the lamps.
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
//!
//! # Two shapes that do not repeat
//!
//! [`Shape::Flicker`] and [`Shape::Lightning`] are not functions of the cycle position
//! alone. Lightning that strikes the same way every seven seconds stops being
//! lightning after the second strike, so they read the *absolute* cycle count and hash
//! it into the next knot or the next strike. They are still pure: the count comes from
//! the anchor and the moment, exactly as the position does, so every station and the
//! browser see the same flash. The hash is salted with the fixture's phase, so a
//! random spread makes every fixture flicker independently and an even spread makes
//! the whole rig strike at once, which is what lightning does.
//!
//! Neither is in the table a node shares. A node advertises the shapes it can run
//! itself, and it never advertises these, so the console streams values for them.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::value::{interpolate, ParameterValue};

// ── What is running ───────────────────────────────────────────────────────────

/// The shapes a value can trace.
///
/// The first five are evaluated over one cycle and repeat exactly. The last two read
/// the absolute cycle count as well, so they never repeat (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Shape {
    Sine,
    Triangle,
    Square,
    SawUp,
    SawDown,
    /// Smooth value noise, [`FLICKER_KNOTS`] new levels a cycle. A candle, a torch, a
    /// failing tube, depending on the rate and the range.
    Flicker,
    /// Dark, then a burst of one to three falling flashes at the top of the cycle,
    /// inside the first `width` of it. Not every cycle strikes.
    Lightning,
}

/// How many new levels [`Shape::Flicker`] picks per cycle.
pub const FLICKER_KNOTS: f64 = 8.0;

/// How often a cycle of [`Shape::Lightning`] strikes at all.
pub const LIGHTNING_STRIKES: f32 = 0.7;

impl Shape {
    /// Whether this shape reads the absolute cycle count and so never repeats.
    pub fn is_random(self) -> bool {
        matches!(self, Shape::Flicker | Shape::Lightning)
    }
}

/// How one value gives way to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Easing {
    /// No transition: hold, then jump.
    Step,
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

/// One keyframe of a step list.
///
/// The value is a real [`ParameterValue`] rather than a level, so a chase can be red,
/// green, blue rather than three brightnesses of one color. `easing` describes the
/// transition into the *next* step, which is why a hard chase is a list of steps that
/// all say [`Easing::Step`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Step {
    /// Where in the cycle this step begins, 0..1.
    pub at: f32,
    pub value: ParameterValue,
    pub easing: Easing,
}

/// What the cycle position is turned into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Curve {
    Shape(Shape),
    Steps(Vec<Step>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Direction {
    #[default]
    Forward,
    Backward,
}

/// Where a running effect came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum EffectSource {
    Programmer,
    Cue(Uuid),
}

/// What is actually running on one fixture parameter: rate resolved to Hz, anchor
/// decided, nothing left to look up.
///
/// This is the value a capable node is sent, and the value the GUI reads to draw a
/// dot on a waveform. It is LOCAL on the fixture, because it is a description of what
/// this station is currently rendering rather than a fact about the show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RunningEffect {
    pub effect_id: Uuid,
    pub curve: Curve,
    pub rate_hz: f32,
    pub low: ParameterValue,
    pub high: ParameterValue,
    pub width: f32,
    pub direction: Direction,
    pub phase: f32,
    pub t0: u64,
    pub source: EffectSource,
}

/// A fade the engine is part way through, described well enough that a node could do
/// it instead.
///
/// The engine interpolates this itself for everything that cannot be told about time.
/// A node that advertises `transitions` is handed the description and left to it, and
/// the console stops sending samples for that port until the fade is over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RunningFade {
    pub from: ParameterValue,
    pub to: ParameterValue,
    /// Console unix ms the movement starts, with any delay already added.
    pub t0: u64,
    pub duration_ms: u32,
    pub easing: Easing,
    pub cue_id: Uuid,
}

// ── Evaluating it ─────────────────────────────────────────────────────────────

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
    // `rem_euclid` rather than `%`: a cue rendered before its own anchor, which clock
    // skew between stations makes ordinary, must wrap to the top of the cycle rather
    // than to a negative position.
    traveled(rate_hz, direction, phase, t0, now_ms).rem_euclid(1.0) as f32
}

/// How many cycles an effect has traveled by `now_ms`, phase included, unwrapped.
///
/// [`cycle_position`] is this wrapped into one cycle. The two shapes that never repeat
/// need the whole number as well, to know *which* cycle they are in.
pub fn traveled(rate_hz: f32, direction: Direction, phase: f32, t0: u64, now_ms: u64) -> f64 {
    let elapsed_ms = now_ms as i64 - t0 as i64;
    let cycles = elapsed_ms as f64 / 1000.0 * rate_hz as f64;
    let along = match direction {
        Direction::Forward => cycles,
        Direction::Backward => -cycles,
    };
    along + phase as f64
}

/// A shape's level at a cycle position, 0..1.
pub fn curve_level(shape: Shape, width: f32, x: f32) -> f32 {
    match shape {
        Shape::Sine => 0.5 + 0.5 * (core::f32::consts::TAU * x).sin(),
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
        // The first cycle of each, unsalted: what a waveform of one draws.
        Shape::Flicker | Shape::Lightning => shape_level(shape, width, x as f64, 0),
    }
}

/// A shape's level after `traveled` cycles, 0..1, for any shape.
///
/// The periodic five only look at where in the cycle that is. Flicker and lightning
/// also look at which cycle it is, and `salt` keeps two fixtures apart.
pub fn shape_level(shape: Shape, width: f32, traveled: f64, salt: u32) -> f32 {
    match shape {
        Shape::Flicker => {
            let along = traveled * FLICKER_KNOTS;
            let knot = along.floor();
            let t = (along - knot) as f32;
            let knot = knot as i64;
            let a = lattice(knot, salt);
            let b = lattice(knot + 1, salt);
            // Smoothstep between knots: a level that holds and then moves, rather than
            // the zigzag a straight line between random points draws.
            a + (b - a) * (t * t * (3.0 - 2.0 * t))
        }
        Shape::Lightning => {
            let cycle = traveled.floor();
            let x = (traveled - cycle) as f32;
            let cycle = cycle as i64;
            if lattice(cycle, salt ^ 0x5157) >= LIGHTNING_STRIKES {
                return 0.0;
            }
            let window = width.clamp(0.02, 1.0);
            if x >= window {
                return 0.0;
            }
            // One to three flashes, each sharper than it is long, in the window.
            let flashes = 1 + (lattice(cycle, salt ^ 0x0F1A) * 3.0).min(2.999) as i64;
            let length = window / flashes as f32;
            let j = ((x / length) as i64).min(flashes - 1);
            let t = (x - j as f32 * length) / length;
            let peak = 0.45 + 0.55 * lattice(cycle * 4 + j, salt ^ 0x9EAC);
            peak * (1.0 - t) * (1.0 - t)
        }
        periodic => curve_level(periodic, width, traveled.rem_euclid(1.0) as f32),
    }
}

/// A lattice point hashed to 0..1: SplitMix64's finalizer over the index and a salt.
///
/// Integer arithmetic only, so a page and a station agree to the bit. 24 bits of the
/// result are kept, which is every bit an `f32` mantissa can hold exactly.
fn lattice(index: i64, salt: u32) -> f32 {
    let mut z = (index as u64) ^ ((salt as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
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
/// Numbers and colors interpolate through [`interpolate`], the same arithmetic a
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
    order.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(core::cmp::Ordering::Equal));

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
    let mut traveled = x - step.at;
    if traveled < 0.0 {
        traveled += 1.0;
    }
    let t = ease(step.easing, (traveled / span).clamp(0.0, 1.0));
    Some(blend(&step.value, &next.value, t))
}

/// What one running effect is asserting at `now_ms`.
pub fn effect_value_at(effect: &RunningEffect, now_ms: u64) -> ParameterValue {
    let along = traveled(effect.rate_hz, effect.direction, effect.phase, effect.t0, now_ms);
    let x = along.rem_euclid(1.0) as f32;
    match &effect.curve {
        Curve::Shape(shape) => blend(
            &effect.low,
            &effect.high,
            shape_level(*shape, effect.width, along, effect.phase.to_bits()),
        ),
        // A step list that has lost its steps holds the bottom of its range, which is
        // dark for an intensity and off for a relay.
        Curve::Steps(steps) => step_value(steps, x).unwrap_or_else(|| effect.low.clone()),
    }
}

/// Position through a fade at `now_ms`, 0.0 before it starts and 1.0 once done.
pub fn fade_progress(fade: &RunningFade, now_ms: u64) -> f32 {
    if now_ms < fade.t0 {
        return 0.0;
    }
    if fade.duration_ms == 0 {
        return 1.0;
    }
    let elapsed = (now_ms - fade.t0) as f32;
    (elapsed / fade.duration_ms as f32).min(1.0)
}

/// Has this fade arrived by `now_ms`?
pub fn fade_is_done(fade: &RunningFade, now_ms: u64) -> bool {
    fade_progress(fade, now_ms) >= 1.0
}

/// What one running fade is asserting at `now_ms`.
pub fn fade_value_at(fade: &RunningFade, now_ms: u64) -> ParameterValue {
    interpolate(&fade.from, &fade.to, ease(fade.easing, fade_progress(fade, now_ms)))
}

#[cfg(test)]
mod tests;
