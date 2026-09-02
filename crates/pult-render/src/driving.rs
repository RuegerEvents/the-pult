//! What is driving one parameter, and therefore what it is putting out.
//!
//! The console keeps *what is driving* a parameter — a fade anchored in time, an
//! effect anchored in time, the programmer over the top, the home value underneath —
//! and nobody keeps the answer. This is the function that turns the first into the
//! second, and it is the whole of the priority rule: the programmer wins, then an
//! effect, then a fade, then where the parameter rests.
//!
//! The order matters less than it looks, because the station only ever publishes the
//! winner of the two middle layers: a fade under an effect is not listed at all. What
//! the order is really for is the programmer, which is published separately because
//! it is SYNCED show data rather than something a station worked out.
//!
//! A fade that has arrived is deliberately still a fade. It is the only record of
//! where the parameter got to — nothing stores the number any more — and evaluating a
//! finished fade is exactly the constant it landed on.

use crate::effect::{fade_value_at, effect_value_at, RunningEffect, RunningFade};
use crate::value::ParameterValue;

/// The layers acting on one parameter, highest priority first.
///
/// Every field is optional and all four may be absent, which is a parameter nothing
/// has ever driven and whose type declares no default — the one case that has no
/// value at all.
#[derive(Debug, Default, Clone, Copy)]
pub struct Driving<'a> {
    /// A plain value the programmer is holding. A programmer *effect* is not here:
    /// the station resolves it against its speed master and publishes it as the
    /// running effect below, which is what keeps rate-following out of this crate.
    pub programmer: Option<&'a ParameterValue>,
    pub effect: Option<&'a RunningEffect>,
    pub fade: Option<&'a RunningFade>,
    pub home: Option<&'a ParameterValue>,
}

impl<'a> Driving<'a> {
    /// True when some layer is asserting something, home or not.
    pub fn is_empty(&self) -> bool {
        self.programmer.is_none()
            && self.effect.is_none()
            && self.fade.is_none()
            && self.home.is_none()
    }

    /// True when playback or the programmer is asserting something — that is, when
    /// this parameter is being driven rather than merely resting.
    pub fn is_driven(&self) -> bool {
        self.programmer.is_some() || self.effect.is_some() || self.fade.is_some()
    }
}

/// What this parameter is putting out at `now_ms`.
///
/// `None` only where nothing at all applies, which a caller reads as "this fixture
/// has no such parameter" rather than as a zero of some shape.
pub fn value_at(driving: &Driving<'_>, now_ms: u64) -> Option<ParameterValue> {
    if let Some(held) = driving.programmer {
        return Some(held.clone());
    }
    if let Some(effect) = driving.effect {
        return Some(effect_value_at(effect, now_ms));
    }
    if let Some(fade) = driving.fade {
        return Some(fade_value_at(fade, now_ms));
    }
    driving.home.cloned()
}

/// The console millisecond after which this parameter stops changing on its own, or
/// `None` if it never does.
///
/// What a consumer with a frame rate asks in order to stop paying for frames nobody
/// can see a difference in. An effect runs for ever, so it answers `None`; a fade
/// answers when it lands; a held value and a home value answer "already".
pub fn settles_at(driving: &Driving<'_>) -> Option<u64> {
    if driving.programmer.is_some() {
        return Some(0);
    }
    if driving.effect.is_some() {
        return None;
    }
    match driving.fade {
        Some(fade) => Some(fade.t0.saturating_add(fade.duration_ms as u64)),
        None => Some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Curve, Direction, Easing, EffectSource, Shape};
    use uuid::Uuid;

    fn a_fade(from: f32, to: f32, t0: u64, duration_ms: u32) -> RunningFade {
        RunningFade {
            from: ParameterValue::Float(from),
            to: ParameterValue::Float(to),
            t0,
            duration_ms,
            easing: Easing::Linear,
            cue_id: Uuid::nil(),
        }
    }

    fn an_effect(t0: u64) -> RunningEffect {
        RunningEffect {
            effect_id: Uuid::nil(),
            curve: Curve::Shape(Shape::SawUp),
            rate_hz: 1.0,
            low: ParameterValue::Float(0.0),
            high: ParameterValue::Float(1.0),
            width: 0.5,
            direction: Direction::Forward,
            phase: 0.0,
            t0,
            source: EffectSource::Programmer,
        }
    }

    #[test]
    fn nothing_driving_a_parameter_gives_its_home_value() {
        let home = ParameterValue::Float(0.25);
        let driving = Driving { home: Some(&home), ..Default::default() };
        assert_eq!(value_at(&driving, 1_000), Some(ParameterValue::Float(0.25)));
        assert!(!driving.is_driven());
    }

    #[test]
    fn a_parameter_nothing_can_say_anything_about_has_no_value() {
        assert_eq!(value_at(&Driving::default(), 1_000), None);
        assert!(Driving::default().is_empty());
    }

    #[test]
    fn a_fade_beats_the_home_value_and_moves_with_the_moment() {
        let home = ParameterValue::Float(0.0);
        let fade = a_fade(0.0, 1.0, 1_000, 1_000);
        let driving = Driving { fade: Some(&fade), home: Some(&home), ..Default::default() };
        assert_eq!(value_at(&driving, 1_500), Some(ParameterValue::Float(0.5)));
    }

    #[test]
    fn a_fade_that_has_arrived_is_still_where_the_parameter_is() {
        let fade = a_fade(0.0, 0.8, 1_000, 500);
        let driving = Driving { fade: Some(&fade), ..Default::default() };
        assert_eq!(value_at(&driving, 9_999_999), Some(ParameterValue::Float(0.8)));
    }

    #[test]
    fn an_effect_beats_a_fade_and_the_programmer_beats_both() {
        let fade = a_fade(0.0, 1.0, 0, 1_000);
        let effect = an_effect(0);
        let held = ParameterValue::Float(0.125);

        let over_fade = Driving { effect: Some(&effect), fade: Some(&fade), ..Default::default() };
        assert_eq!(value_at(&over_fade, 250), Some(ParameterValue::Float(0.25)));

        let over_everything = Driving { programmer: Some(&held), ..over_fade };
        assert_eq!(value_at(&over_everything, 250), Some(ParameterValue::Float(0.125)));
    }

    #[test]
    fn what_settles_says_when_there_is_nothing_more_to_draw() {
        let fade = a_fade(0.0, 1.0, 1_000, 400);
        let effect = an_effect(0);
        assert_eq!(settles_at(&Driving { fade: Some(&fade), ..Default::default() }), Some(1_400));
        assert_eq!(settles_at(&Driving { effect: Some(&effect), ..Default::default() }), None);
        assert_eq!(settles_at(&Driving::default()), Some(0));
    }
}
