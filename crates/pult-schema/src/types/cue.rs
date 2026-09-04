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
    /// Fire at a specific SMPTE timecode position.
    Timecode { hours: u8, minutes: u8, seconds: u8, frames: u8 },
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
}
