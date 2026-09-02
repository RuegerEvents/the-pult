//! The console's evaluator, for a browser.
//!
//! One implementation, compiled twice. The station links [`pult_render`] natively; a
//! page links this, which is the same crate through a thin boundary. There is no
//! TypeScript translation of the arithmetic anywhere, and that is the point: easings,
//! curves, step lists, spread, phase, direction, width, master rates, priority and
//! home fallback are a large enough surface that two implementations would drift, and
//! the visible form of that drift is the screen disagreeing with the lamps.
//!
//! # The shape of the boundary
//!
//! A crossing per fixture per frame would replace a protocol cost with a boundary
//! cost, which is the mistake being fixed one level up. So the page hands over what is
//! *driving* the rig when that changes, says once which parameters it is showing, and
//! then asks for all of them at a moment: one `f64` in, one `Float32Array` out, per
//! frame, whatever is on screen.
//!
//! The page also does the naming. A parameter is identified by `"<fixture id>/<key>"`,
//! built by the same `parameterKey` the browser already uses for programmer entry ids
//! and map keys — so nothing here has to know what a `ParameterKind` is, and there is
//! no second spelling of the key to disagree about.

use std::collections::HashMap;

use pult_render::{
    effect::{RunningEffect, RunningFade},
    value::ParameterValue,
    Driving,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

/// What is acting on one parameter, as the page describes it.
///
/// The same four layers [`Driving`] has, in the same priority order. `programmer`
/// carries only a plain held value: a programmer *shape* has already been resolved
/// against its speed master by the station and arrives as `effect`, which is what
/// keeps rate-following — the one part of this that needs the show — out of here.
#[derive(Default, Deserialize)]
pub struct DrivenBy {
    #[serde(default)]
    pub programmer: Option<ParameterValue>,
    #[serde(default)]
    pub effect: Option<RunningEffect>,
    #[serde(default)]
    pub fade: Option<RunningFade>,
    #[serde(default)]
    pub home: Option<ParameterValue>,
}

impl DrivenBy {
    fn driving(&self) -> Driving<'_> {
        Driving {
            programmer: self.programmer.as_ref(),
            effect: self.effect.as_ref(),
            fade: self.fade.as_ref(),
            home: self.home.as_ref(),
        }
    }
}

// ── The packed answer ─────────────────────────────────────────────────────────

/// Nothing applies: this parameter has no value at all.
pub const NONE: f32 = 0.0;
pub const FLOAT: f32 = 1.0;
pub const INT: f32 = 2.0;
pub const BOOL: f32 = 3.0;
pub const COLOR: f32 = 4.0;
/// A line of text, which does not fit in four floats. Ask [`Evaluator::text`] for it.
pub const TEXT: f32 = 5.0;

/// Four floats per parameter: a tag and up to three components.
///
/// Fixed width rather than variable, so the page indexes into the answer by position
/// and never parses it. Four because a colour has three components and everything
/// else has one; the waste is a few kilobytes on a rig nobody is looking at all of.
pub const STRIDE: usize = 4;

fn pack(value: Option<ParameterValue>, out: &mut [f32]) {
    match value {
        None => out[0] = NONE,
        Some(ParameterValue::Float(v)) => {
            out[0] = FLOAT;
            out[1] = v;
        }
        Some(ParameterValue::Int(v)) => {
            out[0] = INT;
            out[1] = v as f32;
        }
        Some(ParameterValue::Bool(on)) => {
            out[0] = BOOL;
            out[1] = if on { 1.0 } else { 0.0 };
        }
        Some(ParameterValue::Color { r, g, b }) => {
            out[0] = COLOR;
            out[1] = r;
            out[2] = g;
            out[3] = b;
        }
        Some(ParameterValue::Text(_)) => out[0] = TEXT,
    }
}

// ── The evaluator ─────────────────────────────────────────────────────────────

/// What is driving the rig, and which of it the page is showing.
#[wasm_bindgen]
#[derive(Default)]
pub struct Evaluator {
    /// Every parameter anything is driving, keyed `"<fixture id>/<key>"`.
    driving: HashMap<String, DrivenBy>,
    /// What the page is showing, in the order its answers come back.
    watching: Vec<String>,
    /// Reused between frames, so drawing does not allocate.
    packed: Vec<f32>,
}

#[wasm_bindgen]
impl Evaluator {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Evaluator {
        Evaluator::default()
    }

    /// Replace everything that is driving the rig.
    ///
    /// Called when the show changes, never per frame. The page passes an object keyed
    /// `"<fixture id>/<key>"`, each value a [`DrivenBy`].
    pub fn set_driving(&mut self, driving: JsValue) -> Result<(), JsValue> {
        self.driving = serde_wasm_bindgen::from_value(driving)?;
        Ok(())
    }

    /// Replace what is driving one parameter, leaving the rest alone.
    ///
    /// A cue taken over a rig of thousands arrives as one row at a time, and rebuilding
    /// the whole map per row would make taking a cue quadratic in the size of the rig.
    pub fn set_one(&mut self, key: &str, driven_by: JsValue) -> Result<(), JsValue> {
        if driven_by.is_undefined() || driven_by.is_null() {
            self.driving.remove(key);
            return Ok(());
        }
        self.driving.insert(key.to_string(), serde_wasm_bindgen::from_value(driven_by)?);
        Ok(())
    }

    /// Forget every parameter of one fixture — unpatched, or off the end of what the
    /// page is showing.
    pub fn forget_fixture(&mut self, fixture_id: &str) {
        let prefix = format!("{fixture_id}/");
        self.driving.retain(|key, _| !key.starts_with(&prefix));
    }

    /// Say which parameters will be asked for, and in what order the answers come.
    ///
    /// Once per change to what is on screen, rather than per frame. It is what makes a
    /// frame one crossing instead of one per fixture: a rig of two thousand with forty
    /// on screen watches forty and pays for forty.
    pub fn watch(&mut self, keys: JsValue) -> Result<(), JsValue> {
        self.watching = serde_wasm_bindgen::from_value(keys)?;
        self.packed = vec![0.0; self.watching.len() * STRIDE];
        Ok(())
    }

    /// How many parameters are being watched.
    #[wasm_bindgen(getter)]
    pub fn watched(&self) -> usize {
        self.watching.len()
    }

    /// Every watched parameter at one console millisecond.
    ///
    /// Four floats each, in the order [`Evaluator::watch`] was given: a tag, then up
    /// to three components. `now_ms` is an `f64` because that is what a browser's
    /// clock is, and every millisecond a show will ever run in is exact in one.
    pub fn evaluate(&mut self, now_ms: f64) -> Vec<f32> {
        let now = now_ms.max(0.0) as u64;
        for (at, key) in self.watching.iter().enumerate() {
            let slot = &mut self.packed[at * STRIDE..(at + 1) * STRIDE];
            slot.fill(0.0);
            let value = self
                .driving
                .get(key)
                .and_then(|driven| pult_render::value_at(&driven.driving(), now));
            pack(value, slot);
        }
        self.packed.clone()
    }

    /// One parameter's text, for the few that have one.
    ///
    /// Separate because a line of text does not fit in four floats, and putting a
    /// string channel beside the numbers would make every frame pay for a case that
    /// happens on a handful of displays.
    pub fn text(&self, key: &str, now_ms: f64) -> Option<String> {
        let now = now_ms.max(0.0) as u64;
        match self.driving.get(key).and_then(|d| pult_render::value_at(&d.driving(), now)) {
            Some(ParameterValue::Text(text)) => Some(text),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_render::effect::Easing;

    fn a_fade(from: f32, to: f32, t0: u64, duration_ms: u32) -> RunningFade {
        RunningFade {
            from: ParameterValue::Float(from),
            to: ParameterValue::Float(to),
            t0,
            duration_ms,
            easing: Easing::Linear,
            cue_id: Default::default(),
        }
    }

    /// The packing, which is the only thing this crate adds to the evaluator.
    #[test]
    fn a_value_packs_into_four_floats_with_its_kind_in_front() {
        let mut out = [0.0f32; STRIDE];

        pack(Some(ParameterValue::Float(0.25)), &mut out);
        assert_eq!(out, [FLOAT, 0.25, 0.0, 0.0]);

        pack(Some(ParameterValue::Color { r: 1.0, g: 0.5, b: 0.0 }), &mut out);
        assert_eq!(out, [COLOR, 1.0, 0.5, 0.0]);

        pack(Some(ParameterValue::Bool(true)), &mut out);
        assert_eq!(out, [BOOL, 1.0, 0.5, 0.0], "only what its kind uses is written");

        pack(None, &mut out);
        assert_eq!(out[0], NONE, "and nothing driving it says so rather than reading zero");
    }

    /// The same arithmetic the station runs, reached the way a page reaches it.
    #[test]
    fn a_watched_fade_moves_between_two_evaluations_of_one_description() {
        let mut evaluator = Evaluator::default();
        evaluator.driving.insert(
            "spot/Intensity".into(),
            DrivenBy { fade: Some(a_fade(0.0, 1.0, 1_000, 4_000)), ..Default::default() },
        );
        evaluator.watching = vec!["spot/Intensity".into()];
        evaluator.packed = vec![0.0; STRIDE];

        assert_eq!(evaluator.evaluate(2_000.0), vec![FLOAT, 0.25, 0.0, 0.0]);
        assert_eq!(evaluator.evaluate(3_000.0), vec![FLOAT, 0.5, 0.0, 0.0]);
        assert_eq!(evaluator.evaluate(9_000.0), vec![FLOAT, 1.0, 0.0, 0.0]);
    }

    /// A parameter nothing is driving reads as absent rather than as zero, which for a
    /// dimmer would be a light the page has decided to turn off.
    #[test]
    fn an_unwatched_or_undriven_parameter_reads_as_absent() {
        let mut evaluator = Evaluator::default();
        evaluator.watching = vec!["nobody/Intensity".into()];
        evaluator.packed = vec![0.0; STRIDE];
        assert_eq!(evaluator.evaluate(1_000.0)[0], NONE);
    }
}
