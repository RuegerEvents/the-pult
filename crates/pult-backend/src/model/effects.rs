//! Turning a stored effect spec into something renderable.
//!
//! The arithmetic moved to `pult-render`, which the browser can compile too; what is
//! left here is the part that needs the show — which speed master an effect follows,
//! whether that master is running, and what anchors an effect carrying no anchor of
//! its own. Still pure: give it the same inputs on two consoles and it must give the
//! same answer, or the rig will not be in step with itself.
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
    effect::{EffectSource, EffectSpec, Rate, RunningEffect},
    speedmaster::{SpeedMaster, FALLBACK_BPM},
};

// The arithmetic itself lives in `pult-render`, so the browser evaluates the same
// numbers this station does rather than a TypeScript translation of them. Re-exported
// under the names this module has always used.
pub use pult_render::effect::{
    blend, curve_level, cycle_position, ease, effect_value_at as value_at, step_value,
};

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
