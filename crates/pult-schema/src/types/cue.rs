use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::effect::{Easing, EffectSpec};
use super::fixture::{ParameterKind, ParameterValue};
use crate::PultSchema;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FollowMode {
    /// Wait for the operator to press Go.
    Manual,
    /// Auto-fire after the previous cue completes, plus a delay.
    FollowAfter { delay_ms: u32 },
    // There was a `Timecode { hours, minutes, seconds, frames }` here, unimplemented
    // since task 3 and deliberately waiting for this design rather than getting a
    // stopgap. It is gone rather than implemented: a position written on a cue is a
    // clock a cue cannot see, and the same fact written as a `timelines` event is a
    // list an operator can read, reorder and drag. One song Going cues in three
    // sequences was the case it could never have carried.
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParameterCapture {
    pub fixture_id: Uuid,
    pub parameter_kind: ParameterKind,
    pub value: ParameterValue,
    /// How long this capture takes when the parameter is going *up*, and when it is
    /// going nowhere a console can rank — a colour, a relay. Zero means the cue's.
    pub fade_in_ms: u32,
    /// The same, for a parameter coming *down*. Zero means the cue's out time, and a
    /// cue with no out time either means the in time: a show that never says
    /// otherwise fades one way in both directions, as it always has.
    pub fade_out_ms: u32,
    pub delay_in_ms: u32,
    /// A periodic instruction instead of a destination. When this is set the capture
    /// asserts a shape rather than a value, and `value` is only what the parameter
    /// falls back to if the effect cannot be rendered.
    ///
    /// Defaulted rather than migrated: a cue stored before effects existed has no
    /// `effect` key, and `captures` is one JSON column with nothing to alter.
    #[serde(default)]
    pub effect: Option<EffectSpec>,
    /// The shape of this capture's own fade. `None` means the cue's, which means the
    /// show's default for this parameter's group — the same three steps the fade
    /// *times* take, and resolved in one place,
    /// [`crate::types::show::FadeCurves::resolve`].
    ///
    /// A capture stored before there was anything above it to inherit from says
    /// `Linear` outright and keeps saying it, which is the honest reading: that show
    /// ran linear, and a curve appearing in it because a default changed underneath
    /// would be this console rewriting somebody's cue.
    #[serde(default)]
    pub easing: Option<Easing>,
    /// The preset this capture is a reference to, if it is one.
    ///
    /// **Reference first, literal beside it.** `value` stays the copy taken when the
    /// capture was stored and is never rewritten by a preset edit; it is what plays
    /// when the preset has been deleted or does not name this fixture. So deleting a
    /// preset cascades nothing, and Ctrl-Z of the delete restores every link at once
    /// because no link was ever broken.
    ///
    /// Resolved in exactly one place, [`ParameterCapture::value_in`].
    #[serde(default)]
    pub preset: Option<Uuid>,
}

impl ParameterCapture {
    /// What this capture actually asserts: the preset it names where that resolves,
    /// and the literal it was stored as otherwise.
    ///
    /// The one resolution, called by `start_capture`, by the playback pass and by the
    /// paperwork RPC. Two of them would disagree about exactly the cue somebody had
    /// pointed at a palette they then deleted.
    pub fn value_in<'a>(
        &'a self,
        presets: &std::collections::HashMap<Uuid, &'a super::preset::Preset>,
    ) -> &'a ParameterValue {
        self.preset
            .and_then(|id| presets.get(&id))
            .and_then(|preset| preset.value_for(self.fixture_id, &self.parameter_kind))
            .unwrap_or(&self.value)
    }
}

/// A single lighting state snapshot with timing information.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "cues")]
pub struct Cue {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// Fractional cue number (1.0, 1.5, 2.0) — allows insertions.
    #[pult(lifecycle = PERSISTED)]
    pub number: f64,
    #[pult(lifecycle = PERSISTED)]
    pub captures: Vec<ParameterCapture>,
    #[pult(lifecycle = PERSISTED)]
    pub follow_mode: FollowMode,
    /// What every capture of this cue takes on the way up, unless it says its own.
    #[pult(lifecycle = PERSISTED)]
    pub fade_in_ms: u32,
    /// And on the way down. Zero is not "snap": it means this cue does not split its
    /// fade, and everything takes the in time in both directions.
    #[pult(lifecycle = PERSISTED)]
    pub fade_out_ms: u32,
    /// What shape this cue's captures fade on, unless one of them says its own.
    /// `None` is the show's default for each parameter's group, which is what every
    /// cue nobody has opened this control on means.
    ///
    /// One curve rather than one per direction, where the times are one each. A
    /// split *time* is what a designer asks for constantly — a look that builds
    /// slowly and snaps away — and a curve that eased on the way up and ran linear
    /// on the way down is a distinction nobody has asked for, so it stays one until
    /// somebody does.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub easing: Option<Easing>,
    /// True when this cue is currently being executed (output is active).
    #[pult(lifecycle = SYNCED)]
    pub is_active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::effect::{Curve, Direction, Rate, Shape, Spread};

    #[test]
    fn a_capture_carrying_an_effect_round_trips() {
        let capture = ParameterCapture {
            fixture_id: Uuid::nil(),
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(0.0),
            fade_in_ms: 0,
            fade_out_ms: 0,
            delay_in_ms: 0,
            effect: Some(EffectSpec {
                effect_id: Uuid::nil(),
                curve: Curve::Shape(Shape::Sine),
                rate: Rate::Hz(0.5),
                low: ParameterValue::Float(0.0),
                high: ParameterValue::Float(1.0),
                width: 0.5,
                direction: Direction::Forward,
                phase: 0.25,
                spread: Spread::Linear,
                // A stored capture never carries an anchor: the cue's `went_at` is it.
                t0: None,
            }),
            easing: Some(Easing::EaseInOut),
            preset: None,
        };

        let back: ParameterCapture =
            serde_json::from_value(serde_json::to_value(&capture).unwrap()).unwrap();
        let effect = back.effect.expect("survives the round trip");
        assert_eq!(effect.curve, Curve::Shape(Shape::Sine));
        assert_eq!(effect.rate, Rate::Hz(0.5));
        assert_eq!(effect.phase, 0.25);
        assert_eq!(effect.t0, None);
        assert_eq!(back.easing, Some(Easing::EaseInOut));
    }

    #[test]
    fn a_capture_that_names_no_curve_inherits_and_one_that_names_linear_keeps_it() {
        // The two shapes a stored capture can have. A cue written before there was
        // anything to inherit from carries `"easing": "Linear"` and goes on running
        // linear; one written since may carry no key at all, and takes the cue's.
        let older: ParameterCapture = serde_json::from_value(serde_json::json!({
            "fixture_id": Uuid::nil(),
            "parameter_kind": "Pan",
            "value": serde_json::to_value(ParameterValue::Float(0.5)).unwrap(),
            "fade_in_ms": 0,
            "fade_out_ms": 0,
            "delay_in_ms": 0,
            "easing": "Linear",
        }))
        .unwrap();
        assert_eq!(older.easing, Some(Easing::Linear), "said so, and still says so");

        let inheriting: ParameterCapture = serde_json::from_value(serde_json::json!({
            "fixture_id": Uuid::nil(),
            "parameter_kind": "Pan",
            "value": serde_json::to_value(ParameterValue::Float(0.5)).unwrap(),
            "fade_in_ms": 0,
            "fade_out_ms": 0,
            "delay_in_ms": 0,
        }))
        .unwrap();
        assert_eq!(inheriting.easing, None, "nothing said: the cue's, then the show's");
    }

    /// A capture stored before presets existed has no `preset` key, and reads as one
    /// that names none — which is the whole of what "adding a field" has to mean for
    /// a showfile this generation still opens.
    #[test]
    fn an_older_capture_loads_with_no_preset() {
        let older: ParameterCapture = serde_json::from_value(serde_json::json!({
            "fixture_id": Uuid::nil(),
            "parameter_kind": "Intensity",
            "value": serde_json::to_value(ParameterValue::Float(0.5)).unwrap(),
            "fade_in_ms": 0,
            "fade_out_ms": 0,
            "delay_in_ms": 0,
        }))
        .unwrap();
        assert_eq!(older.preset, None);
    }

    /// The three ways a capture resolves: no reference, a reference that resolves,
    /// and a reference that does not — which is a preset deleted, or one that does
    /// not name this fixture. The literal is the answer in two of them, which is why
    /// deleting a preset cascades nothing.
    #[test]
    fn value_in_resolves_three_ways() {
        use crate::types::preset::{Preset, PresetValue};
        use std::collections::HashMap;

        let fixture = Uuid::from_u128(10);
        let preset = Preset {
            id: Uuid::from_u128(1),
            name: "Warm".into(),
            values: vec![PresetValue {
                fixture_id: fixture,
                parameter_kind: ParameterKind::Intensity,
                value: ParameterValue::Float(0.8),
            }],
        };
        let index: HashMap<Uuid, &Preset> = [(preset.id, &preset)].into_iter().collect();

        let capture = |preset: Option<Uuid>, fixture_id: Uuid| ParameterCapture {
            fixture_id,
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(0.2),
            fade_in_ms: 0,
            fade_out_ms: 0,
            delay_in_ms: 0,
            effect: None,
            easing: None,
            preset,
        };

        assert_eq!(
            capture(None, fixture).value_in(&index),
            &ParameterValue::Float(0.2),
            "no reference: the literal"
        );
        assert_eq!(
            capture(Some(preset.id), fixture).value_in(&index),
            &ParameterValue::Float(0.8),
            "a reference that resolves: the preset"
        );
        assert_eq!(
            capture(Some(preset.id), Uuid::from_u128(99)).value_in(&index),
            &ParameterValue::Float(0.2),
            "a preset that does not name this fixture: the literal it was stored as"
        );
        assert_eq!(
            capture(Some(Uuid::from_u128(404)), fixture).value_in(&HashMap::new()),
            &ParameterValue::Float(0.2),
            "a preset that is gone: the literal, and nothing cascaded"
        );
    }
}

/// The latest capture of every key over a run of cues.
///
/// **The whole of what "a cue is the stack up to it" means**, as one function. Playback
/// uses it to decide what a Go asserts; the paperwork's `paperwork.cueValues` RPC uses it
/// to answer what a rig would look like in a given state, for a rendered viewport on a
/// sheet. Written once because those two are the same question asked for different
/// reasons, and two implementations of it would disagree about exactly the cue somebody
/// had built by tracking a value forward three cues.
///
/// `through` is the cue ids in order, up to and including the one being asked about.
/// Anything not in `cues` is skipped rather than refused: a sequence naming a cue that
/// has been deleted is a show mid-edit, not a reason to answer nothing.
pub fn tracked_through<'a>(
    through: impl IntoIterator<Item = &'a Uuid>,
    cues: impl Fn(&Uuid) -> Option<&'a Cue>,
) -> Vec<(&'a Cue, &'a ParameterCapture)> {
    // Insertion-ordered rather than a plain map, so the answer is the same every time it
    // is asked. A `HashMap`'s iteration order is not, and a picture on a sheet that
    // resolved two captures in a different order on a different run would be a document
    // that changes when nothing changed.
    let mut order: Vec<(Uuid, String)> = Vec::new();
    let mut latest: std::collections::HashMap<(Uuid, String), (&Cue, &ParameterCapture)> =
        std::collections::HashMap::new();

    for id in through {
        let Some(cue) = cues(id) else { continue };
        for capture in &cue.captures {
            let key = (capture.fixture_id, super::fixture::parameter_key(&capture.parameter_kind));
            if latest.insert(key.clone(), (cue, capture)).is_none() {
                order.push(key);
            }
        }
    }
    order.into_iter().filter_map(|key| latest.remove(&key)).collect()
}
