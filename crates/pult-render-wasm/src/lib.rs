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
    /// The playing sequence's level, where it dims this parameter. See
    /// [`Driving::master`].
    #[serde(default)]
    pub master: Option<f32>,
    /// The sequence whose cue is driving this intensity, where one is: so a level a
    /// playing take recorded for it can be put over `master` frame by frame, the way
    /// the station's connectors do.
    #[serde(default)]
    pub sequence: Option<String>,
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
            master: self.master,
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
/// and never parses it. Four because a color has three components and everything
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

/// What each of a fixture's emitters should be at, for one color.
///
/// A free function rather than a method, because the question is about a *fixture* and
/// a color somebody is dragging, not about anything the show is driving: the color
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

/// What each of a list of effects is asserting at `now_ms`, packed like
/// [`Evaluator::evaluate`]: [`STRIDE`] floats each, in the order given.
///
/// For the Effects panel's preview, which shows what an effect *will* do across a
/// selection before anything is applied: nothing it draws is driving the rig, so it
/// does not belong in the evaluator's own map. The arithmetic is the one the lamps
/// take, which is the whole reason the preview asks here rather than working it out.
#[wasm_bindgen]
pub fn effect_values(effects: JsValue, now_ms: f64) -> Result<Vec<f32>, JsValue> {
    let effects: Vec<RunningEffect> = serde_wasm_bindgen::from_value(effects)?;
    let mut out = vec![0.0; effects.len() * STRIDE];
    let now = now_ms.max(0.0) as u64;
    for (i, effect) in effects.iter().enumerate() {
        pack(
            Some(pult_render::effect_value_at(effect, now)),
            &mut out[i * STRIDE..(i + 1) * STRIDE],
        );
    }
    Ok(out)
}

/// One frame of a strip the node draws itself, as the node draws it: three bytes a
/// pixel, brightness applied. See `pult_render::pixfx` and its corpus.
///
/// Plain arguments rather than an object, because the rig view asks this per strip per
/// animation frame and a deserialized struct would be the most expensive part of it.
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub fn pixel_effect(
    effect: &str,
    color: &[u8],
    color2: &[u8],
    brightness: f32,
    speed: f32,
    size: f32,
    reverse: bool,
    mirror: bool,
    segments: i32,
    now_ms: f64,
    count: u16,
) -> Vec<u8> {
    let rgb = |c: &[u8]| [c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0), c.get(2).copied().unwrap_or(0)];
    let params = pult_render::pixfx::PixelParams {
        effect: pult_render::pixfx::PixelEffect::parse(effect),
        color: rgb(color),
        color2: rgb(color2),
        brightness,
        speed,
        size,
        reverse,
        mirror,
        segments,
    };
    let now = now_ms as i64;
    pult_render::pixfx::render(&params, pult_render::pixfx::phase_at(speed, now), now, count)
}

/// A shape's level after `traveled` cycles, 0..1, for drawing its waveform.
///
/// Every shape, including the two that never repeat, which a drawing written in
/// TypeScript would otherwise have to hash for itself.
#[wasm_bindgen]
pub fn shape_level(shape: JsValue, width: f32, traveled: f64, salt: u32) -> Result<f32, JsValue> {
    let shape: pult_render::Shape = serde_wasm_bindgen::from_value(shape)?;
    Ok(pult_render::shape_level(shape, width, traveled, salt))
}

/// A recording outlined for drawing: per key, where it starts and stops and its level
/// along the way, at most `budget` levels each — [`pult_render::outline`] over the
/// same parse the station plays from.
///
/// Free rather than on [`Evaluator`] because a lane draws takes whether or not their
/// timeline is running, and the evaluator holds only what is driving the rig.
#[wasm_bindgen]
pub fn track_outline(bytes: &[u8], budget: u32) -> Result<JsValue, JsValue> {
    let track = pult_render::track::decode(bytes).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(pult_render::outline(&track, budget as usize).serialize(&plain_javascript())?)
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
    /// Running takes whose sequence levels do not play: their timeline is recording a
    /// new pass, and the hand on the fader is what is being recorded.
    levels_muted: std::collections::HashSet<String>,
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
        levels: bool,
    ) {
        if levels {
            self.levels_muted.remove(sha);
        } else {
            self.levels_muted.insert(sha.to_string());
        }
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
        self.levels_muted.remove(sha);
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
    fn composed(&self, key: &str, now: u64) -> Option<Driving<'_>> {
        let driven = self.driving.get(key);
        let recorded = self.track_for(key);
        let mut composed = match (driven, recorded) {
            (None, None) => None,
            (Some(driven), None) => Some(driven.driving()),
            (driven, track) => Some(Driving {
                track,
                ..driven.map(DrivenBy::driving).unwrap_or_default()
            }),
        }?;
        // A level a playing take recorded for this parameter's sequence, over its fader.
        if let Some(sequence) = driven.and_then(|d| d.sequence.as_deref()) {
            let level = self.level_for(sequence).and_then(|at| at.value_at(now));
            if let Some(ParameterValue::Float(level)) = level {
                let level = level.clamp(0.0, 1.0);
                composed.master = (level < 1.0).then_some(level);
            }
        }
        Some(composed)
    }

    /// A sequence's level as the playing takes record it, the latest started winning.
    fn level_for(&self, sequence: &str) -> Option<TrackAt<'_>> {
        self.track_among(&format!("{sequence}/{}", pult_render::SEQUENCE_LEVEL_KEY), true)
    }

    fn track_for(&self, key: &str) -> Option<TrackAt<'_>> {
        self.track_among(key, false)
    }

    fn track_among(&self, key: &str, levels: bool) -> Option<TrackAt<'_>> {
        let mut best: Option<(u64, TrackAt<'_>)> = None;
        for (sha, transport) in &self.playing {
            if levels && self.levels_muted.contains(sha) {
                continue;
            }
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
                self.composed(key, now).and_then(|driving| pult_render::value_at(&driving, now));
            pack(value, slot);
        }
        self.packed = packed;
        self.packed.clone()
    }

    /// Which of the watched parameters a recording is asserting at this moment.
    ///
    /// The fixture sheet's green, and the one layer a page cannot work out for
    /// itself: a fade and an effect are rows on the fixture, and a take is bytes that
    /// only ever cross this boundary. So it is asked for rather than derived, and it
    /// is a *read* — `TrackAt::value_at`, which `composed` already uses, so there is
    /// no second answer to when a recording is speaking.
    ///
    /// Which matters because **a track says nothing before its first point**: a key a
    /// rolling take merely carries is not a key it is asserting, and coloring one
    /// green would be the sheet claiming a source the lamps do not have.
    ///
    /// Asked only while a sheet is open and a timeline is running, so a settled
    /// console pays nothing for it.
    pub fn recording(&self, now_ms: f64) -> Vec<String> {
        let now = now_ms.max(0.0) as u64;
        self.watching
            .iter()
            .filter(|key| self.track_for(key).is_some_and(|at| at.value_at(now).is_some()))
            .cloned()
            .collect()
    }

    /// One parameter's per-emitter overrides, for the color control alone.
    ///
    /// Separate from [`Evaluator::evaluate`] for the reason [`Evaluator::text`] is: the
    /// packed answer is a fixed four floats, a map of names to levels does not fit in
    /// it, and widening the stride would make every frame of every rig pay for a panel
    /// that is open on one screen. The color control asks for this; nothing else does.
    pub fn color_overrides(&self, key: &str, now_ms: f64) -> Result<JsValue, JsValue> {
        let now = now_ms.max(0.0) as u64;
        let overrides = match self.composed(key, now).and_then(|d| pult_render::value_at(&d, now)) {
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
        match self.composed(key, now).and_then(|d| pult_render::value_at(&d, now)) {
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

    /// The sheet's green, and the rule it rests on: a rolling take carrying a key is
    /// not the same as one asserting it, because a track says nothing before its
    /// first point.
    #[test]
    fn a_recording_is_reported_only_where_it_is_actually_asserting() {
        use pult_render::track::{Track, TrackKey, TrackPoint};

        // `uuid` is `pult-render`'s dependency and not this crate's, so the id is taken
        // off the track rather than written — which is a dependency for one test.
        let track = Track {
            keys: vec![
                TrackKey {
                    fixture_id: Default::default(),
                    key: "Intensity".into(),
                    points: vec![TrackPoint { release: false, ms: 2_000, value: ParameterValue::Float(0.6) }],
                },
                TrackKey { fixture_id: Default::default(), key: "Pan".into(), points: vec![] },
            ],
        };
        let fixture = track.keys[0].fixture_id;
        let mut evaluator = Evaluator::default();
        evaluator.load_track("sha", &pult_render::track::encode(&track)).unwrap();
        evaluator.play_track("sha", 10_000.0, 0.0, 1.0, true);
        evaluator.watching =
            vec![format!("{fixture}/Intensity"), format!("{fixture}/Pan"), "nobody/Tilt".into()];

        assert!(
            evaluator.recording(11_000.0).is_empty(),
            "a second into a take, before the first point, the recording says nothing"
        );
        assert_eq!(
            evaluator.recording(13_000.0),
            vec![format!("{fixture}/Intensity")],
            "and once past it, exactly the key it is asserting"
        );
    }

    /// A level a playing take recorded for a sequence dims the intensity that
    /// sequence's cue is driving, frame by frame — and not while the timeline is
    /// recording a new pass, when the fader is what is being recorded.
    #[test]
    fn a_recorded_sequence_level_dims_what_its_cue_is_driving() {
        use pult_render::track::{Track, TrackKey, TrackPoint};

        let track = Track {
            keys: vec![TrackKey {
                fixture_id: Default::default(),
                key: pult_render::SEQUENCE_LEVEL_KEY.into(),
                points: vec![TrackPoint { release: false, ms: 1_000, value: ParameterValue::Float(0.5) }],
            }],
        };
        let sequence = track.keys[0].fixture_id.to_string();
        let mut evaluator = Evaluator::default();
        evaluator.load_track("sha", &pult_render::track::encode(&track)).unwrap();
        evaluator.driving.insert(
            "lamp/Intensity".into(),
            DrivenBy {
                home: Some(ParameterValue::Float(1.0)),
                sequence: Some(sequence),
                ..DrivenBy::default()
            },
        );
        evaluator.watching = vec!["lamp/Intensity".into()];
        evaluator.packed = vec![0.0; STRIDE];

        evaluator.play_track("sha", 10_000.0, 0.0, 1.0, true);
        assert_eq!(evaluator.evaluate(10_500.0)[1], 1.0, "before the take's first point, the fader");
        assert_eq!(evaluator.evaluate(11_500.0)[1], 1.0, "home is not played by a sequence, so not dimmed");

        evaluator.driving.get_mut("lamp/Intensity").unwrap().fade = Some(pult_render::RunningFade {
            from: ParameterValue::Float(1.0),
            to: ParameterValue::Float(1.0),
            t0: 0,
            duration_ms: 0,
            easing: Default::default(),
            cue_id: Default::default(),
        });
        assert_eq!(evaluator.evaluate(11_500.0)[1], 0.5, "a cue's fade, at the recorded level");

        evaluator.play_track("sha", 10_000.0, 0.0, 1.0, false);
        assert_eq!(evaluator.evaluate(11_500.0)[1], 1.0, "recording a new pass: the fader decides");
    }
}
