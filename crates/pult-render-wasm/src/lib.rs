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
    track::{Track, TrackAt, TrackPoint, Transport},
    value::ParameterValue,
    Driving,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// What is acting on one parameter, as the page describes it.
///
/// The same layers [`Driving`] has, in the same priority order. `programmer` carries
/// only a plain held value: a programmer *shape* has already been resolved against its
/// speed master by the station and arrives as `effect`, which is what keeps
/// rate-following — the one part of this that needs the show — out of here.
#[derive(Default, Deserialize)]
pub struct DrivenBy {
    #[serde(default)]
    pub programmer: Option<ParameterValue>,
    /// A recording, spelled out inline.
    ///
    /// Here for the corpus, which describes one case per parameter and has nowhere to
    /// put an asset. A *page* never fills this in: it loads the whole track once with
    /// [`Evaluator::load_track`] and the transport comes from the timeline, because
    /// one recording covers hundreds of parameters and inlining it per parameter would
    /// cross the boundary with the same megabyte once per key.
    #[serde(default)]
    pub track: Option<TrackPlay>,
    #[serde(default)]
    pub effect: Option<RunningEffect>,
    #[serde(default)]
    pub fade: Option<RunningFade>,
    #[serde(default)]
    pub home: Option<ParameterValue>,
}

/// One parameter's change points and where the playhead is, written out in full.
#[derive(Deserialize)]
pub struct TrackPlay {
    pub points: Vec<TrackPoint>,
    pub anchor_ms: u64,
    pub position_at_anchor_ms: u64,
    pub rate: f32,
}

impl TrackPlay {
    fn at(&self) -> TrackAt<'_> {
        TrackAt {
            points: &self.points,
            transport: Transport {
                anchor_ms: self.anchor_ms,
                position_at_anchor_ms: self.position_at_anchor_ms,
                rate: self.rate,
            },
        }
    }
}

impl DrivenBy {
    fn driving(&self) -> Driving<'_> {
        Driving {
            programmer: self.programmer.as_ref(),
            track: self.track.as_ref().map(TrackPlay::at),
            effect: self.effect.as_ref(),
            fade: self.fade.as_ref(),
            home: self.home.as_ref(),
        }
    }
}

/// A decoded recording, with an index from the key a page names to the points.
///
/// The index is built once at load: a page watching four hundred parameters would
/// otherwise scan every key of every playing track once per parameter per frame.
struct Loaded {
    track: Track,
    at: HashMap<String, usize>,
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
        Some(ParameterValue::Color { r, g, b, .. }) => {
            out[0] = COLOR;
            out[1] = r;
            out[2] = g;
            out[3] = b;
        }
        Some(ParameterValue::Text(_)) => out[0] = TEXT,
    }
}

/// What each of a fixture's emitters should be at, for one colour.
///
/// A free function rather than a method, because the question is about a *fixture* and
/// a colour somebody is dragging, not about anything the show is driving: the colour
/// control asks it as the picker moves, before any of it has been written.
///
/// The same [`pult_render::color::mix`] the station's DMX connector calls, so the
/// per-emitter strip in the programmer shows the levels the lamps are actually at.
#[wasm_bindgen]
pub fn emitter_levels(color: JsValue, emitters: JsValue) -> Result<JsValue, JsValue> {
    let color: pult_render::Color = serde_wasm_bindgen::from_value(color)?;
    let emitters: Vec<pult_render::EmitterSpec> = serde_wasm_bindgen::from_value(emitters)?;
    Ok(pult_render::mix(&color, &emitters).serialize(&plain_javascript())?)
}

/// A serializer that answers with plain objects and arrays.
///
/// `serde_wasm_bindgen`'s default turns a map into a JS `Map`, which is right for a
/// round trip and wrong for a value a Svelte component reads: everything else crossing
/// this boundary is JSON-shaped, and one `Map` among them is a `.get()` where every
/// other line is a property access.
fn plain_javascript() -> serde_wasm_bindgen::Serializer {
    serde_wasm_bindgen::Serializer::json_compatible()
}

// ── The evaluator ─────────────────────────────────────────────────────────────

/// What is driving the rig, and which of it the page is showing.
#[wasm_bindgen]
#[derive(Default)]
pub struct Evaluator {
    /// Every parameter anything is driving, keyed `"<fixture id>/<key>"`.
    driving: HashMap<String, DrivenBy>,
    /// The recordings the page has fetched, by the sha they are stored under.
    ///
    /// Kept across a change to what is driving the rig, because a track is an asset
    /// and a cue being taken does not change one — which is the whole reason the bytes
    /// cross this boundary once rather than per push.
    tracks: HashMap<String, Loaded>,
    /// Which of them are running, and where each playhead is.
    playing: HashMap<String, Transport>,
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

    /// Take a recording's bytes, decoded once.
    ///
    /// The page fetches `/assets/<sha>` and hands the bytes straight over — the same
    /// bytes the station reads, through the same [`pult_render::track`] parser, which
    /// is why there is no JSON form of a track anywhere. Loading one that is already
    /// here replaces it, so a re-fetch after a reconnect costs a decode and nothing
    /// else.
    pub fn load_track(&mut self, sha: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let track = pult_render::track::decode(bytes)
            .map_err(|e| JsValue::from_str(&format!("{sha}: {e}")))?;
        let at = track
            .keys
            .iter()
            .enumerate()
            .map(|(index, key)| (format!("{}/{}", key.fixture_id, key.key), index))
            .collect();
        self.tracks.insert(sha.to_string(), Loaded { track, at });
        Ok(())
    }

    /// This recording is running, anchored here.
    ///
    /// The transport is the timeline's own — `anchor_ms`, `position_at_anchor_ms` and
    /// `rate`, straight off the SYNCED row — so a page evaluates the same playhead the
    /// station's connectors do without either of them ticking.
    pub fn play_track(
        &mut self,
        sha: &str,
        anchor_ms: f64,
        position_at_anchor_ms: f64,
        rate: f32,
    ) {
        self.playing.insert(
            sha.to_string(),
            Transport {
                anchor_ms: anchor_ms.max(0.0) as u64,
                position_at_anchor_ms: position_at_anchor_ms.max(0.0) as u64,
                rate,
            },
        );
    }

    /// This recording is no longer running. The bytes stay loaded: stopping a
    /// timeline and starting it again must not cost a download.
    pub fn stop_track(&mut self, sha: &str) {
        self.playing.remove(sha);
    }

    /// Forget a recording entirely — the show no longer carries it.
    pub fn forget_track(&mut self, sha: &str) {
        self.playing.remove(sha);
        self.tracks.remove(sha);
    }

    /// What one parameter is putting out, with every layer composed.
    ///
    /// The tracks are looked up here rather than pushed in with the rest of what is
    /// driving the rig, because they change on a different clock: a cue rewrites
    /// `driving` and does not touch a recording, and a timeline starting rewrites no
    /// parameter at all.
    ///
    /// A key can be in more than one playing recording — two takes of the same song,
    /// both armed — and the **latest anchor wins**, which is the take that was started
    /// most recently. Layering them would be a merge rule nothing has decided.
    fn composed(&self, key: &str) -> Option<Driving<'_>> {
        let driven = self.driving.get(key);
        let recorded = self.track_for(key);
        match (driven, recorded) {
            (None, None) => None,
            (Some(driven), None) => Some(driven.driving()),
            (driven, track) => Some(Driving {
                track,
                ..driven.map(DrivenBy::driving).unwrap_or_default()
            }),
        }
    }

    fn track_for(&self, key: &str) -> Option<TrackAt<'_>> {
        let mut best: Option<(u64, TrackAt<'_>)> = None;
        for (sha, transport) in &self.playing {
            let Some(loaded) = self.tracks.get(sha) else { continue };
            let Some(index) = loaded.at.get(key) else { continue };
            let at = TrackAt { points: &loaded.track.keys[*index].points, transport: *transport };
            if best.is_none_or(|(anchor, _)| transport.anchor_ms >= anchor) {
                best = Some((transport.anchor_ms, at));
            }
        }
        best.map(|(_, at)| at)
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
        // Lifted out for the frame and put back after it. `composed` reads the
        // recordings as well as what is driving the rig, which is a borrow of the
        // whole evaluator — and holding a slice of `packed` mutably across that is the
        // one thing the borrow checker will not have. Taking it costs a moved vector
        // per frame rather than an allocation.
        let mut packed = std::mem::take(&mut self.packed);
        for (at, key) in self.watching.iter().enumerate() {
            let slot = &mut packed[at * STRIDE..(at + 1) * STRIDE];
            slot.fill(0.0);
            let value =
                self.composed(key).and_then(|driving| pult_render::value_at(&driving, now));
            pack(value, slot);
        }
        self.packed = packed;
        self.packed.clone()
    }

    /// One parameter's per-emitter overrides, for the colour control alone.
    ///
    /// Separate from [`Evaluator::evaluate`] for the reason [`Evaluator::text`] is: the
    /// packed answer is a fixed four floats, a map of names to levels does not fit in
    /// it, and widening the stride would make every frame of every rig pay for a panel
    /// that is open on one screen. The colour control asks for this; nothing else does.
    pub fn color_overrides(&self, key: &str, now_ms: f64) -> Result<JsValue, JsValue> {
        let now = now_ms.max(0.0) as u64;
        let overrides = match self.composed(key).and_then(|d| pult_render::value_at(&d, now)) {
            Some(ParameterValue::Color { overrides, .. }) => overrides,
            _ => Default::default(),
        };
        Ok(overrides.serialize(&plain_javascript())?)
    }

    /// One parameter's text, for the few that have one.
    ///
    /// Separate because a line of text does not fit in four floats, and putting a
    /// string channel beside the numbers would make every frame pay for a case that
    /// happens on a handful of displays.
    pub fn text(&self, key: &str, now_ms: f64) -> Option<String> {
        let now = now_ms.max(0.0) as u64;
        match self.composed(key).and_then(|d| pult_render::value_at(&d, now)) {
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

        pack(Some(ParameterValue::rgb(1.0, 0.5, 0.0)), &mut out);
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
