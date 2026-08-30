//! Tracing a shape, and running a fade, without being told the answer each time.
//!
//! A node that can do this is handed one description and then left to it. What it
//! needs in return is to agree with the console about what the description means,
//! exactly, to the millisecond — and the console is not here to ask.
//!
//! # The numeric table, again
//!
//! The shapes below are written out from the protocol documents rather than shared
//! with `pult-backend`, for the reason the crate exists at all: two implementations
//! that agree because they were both written from the spec prove something, and two
//! that agree because they are the same code prove nothing. `oh_curve.c` in the
//! firmware is the third. All three test suites assert the same numbers, and that is
//! the only thing holding them together.
//!
//! Sine is `0.5 + 0.5·sin(2πx)`: a cycle starts at half and peaks a quarter in.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── What a port says it can do ────────────────────────────────────────────────

/// The `effects` block on one port of `GET /api/v1/info`.
///
/// Its absence is the default and means the console renders every value itself, so
/// firmware that has never heard of any of this keeps working unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PortEffects {
    /// Shape names this port can trace: `sine`, `triangle`, `square`, `saw-up`,
    /// `saw-down`.
    #[serde(default)]
    pub shapes: Vec<String>,
    /// Whether a keyframe list is understood.
    #[serde(default)]
    pub steps: bool,
    /// Whether a `set` carrying `fade_ms` is run rather than jumped to.
    #[serde(default)]
    pub transitions: bool,
}

impl PortEffects {
    /// Everything, for a port that can do the lot.
    pub fn all() -> Self {
        PortEffects {
            shapes: ["sine", "triangle", "square", "saw-up", "saw-down"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            steps: true,
            transitions: true,
        }
    }

    /// What a port with two states can honestly claim: it can be chopped, and it can
    /// be stepped, and there is nothing in between for a sine to trace.
    pub fn switching() -> Self {
        PortEffects { shapes: vec!["square".into()], steps: true, transitions: false }
    }
}

// ── Curves ────────────────────────────────────────────────────────────────────

/// A shape's level at a cycle position, 0..1. Unknown names read as flat.
pub fn curve_level(shape: &str, width: f32, x: f32) -> f32 {
    match shape {
        "sine" => 0.5 + 0.5 * (std::f32::consts::TAU * x).sin(),
        "triangle" => {
            if x < 0.5 {
                x * 2.0
            } else {
                2.0 - x * 2.0
            }
        }
        "square" => {
            if x < width.clamp(0.0, 1.0) {
                1.0
            } else {
                0.0
            }
        }
        "saw-up" => x,
        "saw-down" => 1.0 - x,
        _ => 0.0,
    }
}

/// The shape of a transition, 0..1 in, 0..1 out. Unknown names read as linear.
pub fn ease(name: &str, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match name {
        "step" => {
            if t >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        "ease-in" => t * t,
        "ease-out" => t * (2.0 - t),
        "ease-in-out" => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - 2.0 * (1.0 - t) * (1.0 - t)
            }
        }
        _ => t,
    }
}

/// Where in its cycle an effect is at `now_ms`, 0..1.
///
/// The millisecond arithmetic is `i64` and `f64` until the very end. A node that has
/// been up for a few hours is millions of milliseconds from an anchor, and an `f32`
/// stops being able to tell one of those milliseconds from the next long before that.
pub fn cycle_position(rate_hz: f32, backward: bool, phase: f32, t0: i64, now_ms: i64) -> f32 {
    let cycles = (now_ms - t0) as f64 / 1000.0 * rate_hz as f64;
    let travelled = if backward { -cycles } else { cycles };
    (travelled + phase as f64).rem_euclid(1.0) as f32
}

// ── Payload arithmetic ────────────────────────────────────────────────────────
//
// A port's payload shape is the console's too, so blending happens on the payloads
// rather than on some intermediate the two ends would have to agree about
// separately: `{"value":..}`, `{"state":..}`, `{"r":..,"g":..,"b":..}`, `{"text":..}`.

/// Blend two port payloads. Anything without a midpoint turns over at halfway.
pub fn blend(low: &Value, high: &Value, level: f32) -> Value {
    let t = level.clamp(0.0, 1.0) as f64;

    if let (Some(a), Some(b)) = (low["value"].as_f64(), high["value"].as_f64()) {
        return json!({ "value": a + (b - a) * t });
    }
    if low.get("r").is_some() && high.get("r").is_some() {
        let mix = |k: &str| {
            let a = low[k].as_f64().unwrap_or(0.0);
            let b = high[k].as_f64().unwrap_or(0.0);
            (a + (b - a) * t).round()
        };
        return json!({ "r": mix("r"), "g": mix("g"), "b": mix("b") });
    }
    // A boolean spends half a square wave's cycle on, which is what somebody putting
    // a chase on a relay is asking for. Turning over at the first instant instead
    // would leave it on for all but a moment.
    if t >= 0.5 { high.clone() } else { low.clone() }
}

// ── What is running on a port ─────────────────────────────────────────────────

/// One keyframe of a step list.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub at: f32,
    pub value: Value,
    pub easing: String,
}

/// A shape a port traces on its own, until told otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub id: String,
    pub shape: Option<String>,
    pub steps: Vec<Step>,
    pub rate_hz: f32,
    pub phase: f32,
    pub backward: bool,
    pub width: f32,
    pub low: Value,
    pub high: Value,
    pub t0: i64,
}

/// A move to somewhere, over a stated time, starting at a stated moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub from: Value,
    pub to: Value,
    pub t0: i64,
    pub duration_ms: u32,
    pub easing: String,
}

/// What a port is doing that a single value cannot describe.
#[derive(Debug, Clone, PartialEq)]
pub enum Motion {
    Effect(Effect),
    Transition(Transition),
}

impl Motion {
    /// What this port should be showing at `now_ms`.
    pub fn sample(&self, now_ms: i64) -> Value {
        match self {
            Motion::Effect(effect) => effect.sample(now_ms),
            Motion::Transition(t) => t.sample(now_ms).0,
        }
    }

    /// A one-line account for the panel and the logs.
    pub fn describe(&self) -> String {
        match self {
            Motion::Effect(e) => match &e.shape {
                Some(shape) => format!("{shape} {:.2} Hz", e.rate_hz),
                None => format!("{} steps, {:.2} Hz", e.steps.len(), e.rate_hz),
            },
            Motion::Transition(t) => format!("fade {} ms", t.duration_ms),
        }
    }
}

impl Effect {
    pub fn sample(&self, now_ms: i64) -> Value {
        let x = cycle_position(self.rate_hz, self.backward, self.phase, self.t0, now_ms);
        match &self.shape {
            Some(shape) => blend(&self.low, &self.high, curve_level(shape, self.width, x)),
            None => step_value(&self.steps, x).unwrap_or_else(|| self.low.clone()),
        }
    }
}

impl Transition {
    /// Where it is now, and whether it has arrived.
    pub fn sample(&self, now_ms: i64) -> (Value, bool) {
        if now_ms <= self.t0 {
            return (self.from.clone(), false);
        }
        if self.duration_ms == 0 {
            return (self.to.clone(), true);
        }
        let elapsed = (now_ms - self.t0) as f32;
        let progress = (elapsed / self.duration_ms as f32).min(1.0);
        (blend(&self.from, &self.to, ease(&self.easing, progress)), progress >= 1.0)
    }
}

/// The value a step list is showing at a cycle position.
pub fn step_value(steps: &[Step], x: f32) -> Option<Value> {
    if steps.is_empty() {
        return None;
    }
    // Sorted rather than trusted, so a list an operator dragged around renders the
    // same as one built in order.
    let mut order: Vec<&Step> = steps.iter().collect();
    order.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));

    let current = order.iter().rposition(|s| x >= s.at).unwrap_or(order.len() - 1);
    let step = order[current];
    let next = order[(current + 1) % order.len()];

    if step.easing == "step" || order.len() == 1 {
        return Some(step.value.clone());
    }

    let mut span = next.at - step.at;
    if span <= 0.0 {
        span += 1.0;
    }
    let mut travelled = x - step.at;
    if travelled < 0.0 {
        travelled += 1.0;
    }
    Some(blend(&step.value, &next.value, ease(&step.easing, (travelled / span).clamp(0.0, 1.0))))
}

// ── Reading a descriptor off the wire ─────────────────────────────────────────

/// Parse an `output/<n>/effect` body. `None` for `{"clear": true}` or nonsense.
pub fn parse_effect(body: &Value) -> Option<Effect> {
    if body["clear"].as_bool() == Some(true) {
        return None;
    }
    let curve = &body["curve"];
    let shape = curve["shape"].as_str().map(str::to_string);
    let steps: Vec<Step> = curve["steps"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|s| {
                    Some(Step {
                        at: s["at"].as_f64()? as f32,
                        value: s.get("value")?.clone(),
                        easing: s["easing"].as_str().unwrap_or("linear").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if shape.is_none() && steps.is_empty() {
        return None;
    }

    Some(Effect {
        id: body["id"].as_str().unwrap_or_default().to_string(),
        shape,
        steps,
        rate_hz: body["rate"].as_f64().unwrap_or(0.0) as f32,
        phase: body["phase"].as_f64().unwrap_or(0.0) as f32,
        backward: body["direction"].as_str() == Some("backward"),
        width: body["width"].as_f64().unwrap_or(0.5) as f32,
        low: body.get("low").cloned().unwrap_or_else(|| json!({ "value": 0.0 })),
        high: body.get("high").cloned().unwrap_or_else(|| json!({ "value": 1.0 })),
        t0: body["t0"].as_i64().unwrap_or(0),
    })
}

/// The timing on a `set`, if it carries any.
///
/// A `set` with none of these keys is the fast path this node has always had: apply
/// it and be done. That is what makes the timing additive rather than a new protocol.
pub fn parse_transition(body: &Value, from: Value, now_ms: i64) -> Option<Transition> {
    let fade_ms = body["fade_ms"].as_u64();
    let delay_ms = body["delay_ms"].as_u64().unwrap_or(0);
    if fade_ms.is_none() && body.get("curve").is_none() && body.get("t0").is_none() {
        return None;
    }
    let mut to = body.clone();
    if let Some(map) = to.as_object_mut() {
        for key in ["fade_ms", "delay_ms", "curve", "t0"] {
            map.remove(key);
        }
    }
    Some(Transition {
        from,
        to,
        t0: body["t0"].as_i64().unwrap_or(now_ms) + delay_ms as i64,
        duration_ms: fade_ms.unwrap_or(0) as u32,
        easing: body["curve"].as_str().unwrap_or("linear").to_string(),
    })
}

// ── The console's clock ───────────────────────────────────────────────────────

/// Where the console says what time it is. Not under any node's serial: every node on
/// one broker has to agree with every other about when a cycle started.
pub const CLOCK_TOPIC: &str = "openhaunt/clock";

/// What this node thinks the console's clock reads, relative to its own.
///
/// The console publishes `openhaunt/clock` once a second, retained. The estimate is
/// smoothed rather than taken outright because the error being corrected is one-way
/// network latency, which varies; a jump straight to each sample would jog a running
/// effect by however much that varied.
/// How far each live sample moves the estimate. A fifth: quick enough to settle in
/// a few seconds, slow enough that one late message is not a visible jolt.
pub const SMOOTHING: f64 = 0.2;

/// The most a single correction may move the estimate, in milliseconds.
///
/// Without it, one wildly late message would step every running effect by its whole
/// error at once. With it, a genuine large offset still arrives, just over a few
/// seconds. The firmware's `oh_clock_sync.c` uses the same number, and it has to:
/// two nodes on one broker correcting at different rates would drift apart from each
/// other for as long as the correction took.
pub const MAX_SLEW_MS: i64 = 50;

#[derive(Debug, Default)]
pub struct ClockOffset {
    offset_ms: Option<i64>,
    last_seq: Option<u64>,
}

impl ClockOffset {
    /// Take one sample. `retained` marks one the broker replayed on subscribe.
    pub fn feed(&mut self, console_ms: i64, seq: u64, local_ms: i64, retained: bool) {
        let sample = console_ms - local_ms;

        // A `seq` that went backwards means the broker restarted, or this is a
        // retained message from before a gap. Smoothing towards it would drag the
        // estimate through a stale value, so it starts again instead.
        if self.last_seq.is_some_and(|last| seq < last) {
            self.offset_ms = Some(sample);
            self.last_seq = Some(seq);
            return;
        }
        self.last_seq = Some(seq);

        match self.offset_ms {
            // A retained sample was published at an unknown time in the past, so it
            // only seeds: it is a starting point, never a correction.
            None => self.offset_ms = Some(sample),
            Some(_) if retained => {}
            Some(current) => {
                let step =
                    ((sample - current) as f64 * SMOOTHING) as i64;
                self.offset_ms = Some(current + step.clamp(-MAX_SLEW_MS, MAX_SLEW_MS));
            }
        }
    }

    /// The console's clock now, given this node's own.
    pub fn console_now(&self, local_ms: i64) -> i64 {
        local_ms + self.offset_ms.unwrap_or(0)
    }

    pub fn offset_ms(&self) -> Option<i64> {
        self.offset_ms
    }
}

/// Every port that is tracing something, for the snapshot.
pub fn describe_all(motions: &BTreeMap<u8, Motion>) -> BTreeMap<String, Value> {
    motions
        .iter()
        .map(|(port, motion)| {
            (port.to_string(), json!({ "summary": motion.describe() }))
        })
        .collect()
}

#[cfg(test)]
mod tests;
