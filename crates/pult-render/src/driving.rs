//! What is driving one parameter, and therefore what it is putting out.
//!
//! The console keeps *what is driving* a parameter — a fade anchored in time, an
//! effect anchored in time, the programmer over the top, the home value underneath —
//! and nobody keeps the answer. This is the function that turns the first into the
//! second, and it is the whole of the priority rule: the programmer wins, then a
//! recorded track, then an effect, then a fade, then where the parameter rests.
//!
//! The order matters less than it looks, because the station only ever publishes the
//! winner of the two middle layers: a fade under an effect is not listed at all. What
//! the order is really for is the programmer, which is published separately because
//! it is SYNCED show data rather than something a station worked out.
//!
//! A fade that has arrived is deliberately still a fade. It is the only record of
//! where the parameter got to — nothing stores the number any more — and evaluating a
//! finished fade is exactly the constant it landed on.
//!
//! **Beside** the playback layer there can be more of it: an intensity that several
//! sequences are playing at once, merged highest-takes-precedence. The winner of the
//! playback layer is still `effect` or `fade`, and the others are `beside` — each one
//! with its own sequence's level — and what comes out is the highest of them. Only a
//! single number has a highest, so anything else ignores them. See
//! `pult_schema::types::sequence::Merge`.
//!
//! A **track** sits between the programmer and the effect, which is the one place it
//! can go: a recording is somebody else's console asserting a value, so it has to
//! beat the playback it was recorded over and lose to the operator standing at this
//! desk. It is also the only layer that can be present and say nothing — before its
//! first change point it has captured nothing, and what is under it shows through.

use serde::{Deserialize, Serialize};

use crate::effect::{fade_value_at, effect_value_at, RunningEffect, RunningFade};
use crate::track::TrackAt;
use crate::value::ParameterValue;

/// One more sequence playing an intensity beside the one that went last, which an HTP
/// merge keeps rather than replacing: an effect or a fade, whichever it is asserting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct HtpLayer {
    #[serde(default)]
    pub effect: Option<RunningEffect>,
    #[serde(default)]
    pub fade: Option<RunningFade>,
}

impl HtpLayer {
    /// What it is asserting at `now_ms`, before its sequence's level.
    pub fn value_at(&self, now_ms: u64) -> Option<ParameterValue> {
        match (&self.effect, &self.fade) {
            (Some(effect), _) => Some(effect_value_at(effect, now_ms)),
            (None, Some(fade)) => Some(fade_value_at(fade, now_ms)),
            (None, None) => None,
        }
    }

    /// When it stops changing on its own: never for an effect, when it lands for a fade.
    fn settles_at(&self) -> Option<u64> {
        match (&self.effect, &self.fade) {
            (Some(_), _) => None,
            (None, Some(fade)) => Some(fade.t0.saturating_add(fade.duration_ms as u64)),
            (None, None) => Some(0),
        }
    }
}

/// The most layers an intensity can be played by beside the winning one. Fixed so a
/// [`Driving`] stays `Copy` and building one allocates nothing; a fifth sequence on one
/// light past that is left out of the highest, which no show here comes near.
pub const MAX_BESIDE: usize = 4;

/// One of those layers as the evaluator takes it: what it asserts, and the level of the
/// sequence asserting it, `None` being full.
#[derive(Debug, Clone, Copy)]
pub struct Beside<'a> {
    pub layer: &'a HtpLayer,
    pub master: Option<f32>,
}

/// The layers acting on one parameter, highest priority first.
///
/// Every field is optional and all of them may be absent, which is a parameter
/// nothing has ever driven and whose type declares no default — the one case that has
/// no value at all.
#[derive(Debug, Default, Clone, Copy)]
pub struct Driving<'a> {
    /// A plain value the programmer is holding. A programmer *effect* is not here:
    /// the station resolves it against its speed master and publishes it as the
    /// running effect below, which is what keeps rate-following out of this crate.
    pub programmer: Option<&'a ParameterValue>,
    /// A playing recording, and where its playhead is. Present only while a timeline
    /// carrying it is running: a stopped track is not a layer, it is a file.
    pub track: Option<TrackAt<'a>>,
    pub effect: Option<&'a RunningEffect>,
    pub fade: Option<&'a RunningFade>,
    pub home: Option<&'a ParameterValue>,
    /// The level of the sequence whose effect or fade this is, where it is below full
    /// and the parameter is one a master scales.
    ///
    /// Applied only when that playback layer is the one that wins: a master is the
    /// sequence's own fader, so it dims what the sequence is doing and never the
    /// programmer, a recording or a resting value. Which sequence, and whether this key
    /// is one a master reaches, is the caller's to say — this crate knows neither what
    /// a sequence is nor which keys are intensities — and `None` is full.
    pub master: Option<f32>,
    /// The other sequences playing this intensity under an HTP merge, each at its own
    /// level. Empty everywhere else, which is nearly everywhere.
    pub beside: [Option<Beside<'a>>; MAX_BESIDE],
}

impl<'a> Driving<'a> {
    /// The layers beside the winning one, from what a fixture carries under a key, each
    /// with the level its caller works out for it.
    pub fn beside_from(
        layers: &'a [HtpLayer],
        mut master: impl FnMut(&HtpLayer) -> Option<f32>,
    ) -> [Option<Beside<'a>>; MAX_BESIDE] {
        let mut out = [None; MAX_BESIDE];
        for (slot, layer) in out.iter_mut().zip(layers) {
            *slot = Some(Beside { layer, master: master(layer) });
        }
        out
    }
}

impl<'a> Driving<'a> {
    /// True when some layer is asserting something, home or not.
    pub fn is_empty(&self) -> bool {
        self.programmer.is_none()
            && self.track.is_none()
            && self.effect.is_none()
            && self.fade.is_none()
            && self.home.is_none()
            && self.beside.iter().all(Option::is_none)
    }

    /// True when playback, a recording or the programmer is asserting something —
    /// that is, when this parameter is being driven rather than merely resting.
    pub fn is_driven(&self) -> bool {
        self.programmer.is_some()
            || self.track.is_some()
            || self.effect.is_some()
            || self.fade.is_some()
            || self.beside.iter().any(Option::is_some)
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
    // A recording that has not reached its first change point is a layer that is
    // present and asserting nothing, so this falls through rather than returning.
    if let Some(recorded) = driving.track.and_then(|track| track.value_at(now_ms)) {
        return Some(recorded.clone());
    }
    let played = match (driving.effect, driving.fade) {
        (Some(effect), _) => Some(effect_value_at(effect, now_ms)),
        (None, Some(fade)) => Some(fade_value_at(fade, now_ms)),
        (None, None) => None,
    };
    let played = played.map(|value| match driving.master {
        Some(level) => value.scaled(level),
        None => value,
    });
    // Highest takes precedence over the sequences beside the winner: a level each, at
    // its own fader. Only a number has a highest.
    let played = driving.beside.iter().flatten().fold(played, |best, beside| {
        let Some(value) = beside.layer.value_at(now_ms) else { return best };
        let value = match beside.master {
            Some(level) => value.scaled(level),
            None => value,
        };
        match (&best, &value) {
            (None, _) => Some(value),
            (Some(ParameterValue::Float(a)), ParameterValue::Float(b)) if b > a => Some(value),
            _ => best,
        }
    });
    if played.is_some() {
        return played;
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
    // A playing recording is an effect as far as a frame rate is concerned: the next
    // change point can be anywhere, so there is no moment after which nothing more
    // will happen. Which is also why a stopped timeline hands over no track at all —
    // otherwise a settled show would never drop to its keep-alive again.
    if driving.track.is_some() {
        return None;
    }
    if driving.effect.is_some() {
        return None;
    }
    let mut settles = match driving.fade {
        Some(fade) => fade.t0.saturating_add(fade.duration_ms as u64),
        None => 0,
    };
    for beside in driving.beside.iter().flatten() {
        settles = settles.max(beside.layer.settles_at()?);
    }
    Some(settles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Curve, Direction, Easing, EffectSource, Shape};
    use crate::track::{TrackAt, TrackPoint, Transport};
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
    fn an_htp_layer_beside_the_winner_shows_where_it_is_higher_at_its_own_level() {
        // The base at 40% at full, and a sequence that went later at 100% on a fader at
        // 20%: the base shows. Bring that fader to 60% and it does.
        let base = HtpLayer { effect: None, fade: Some(a_fade(0.4, 0.4, 0, 0)) };
        let later = a_fade(1.0, 1.0, 0, 0);
        let layers = [base];
        let low = Driving {
            fade: Some(&later),
            master: Some(0.2),
            beside: Driving::beside_from(&layers, |_| None),
            ..Default::default()
        };
        assert_eq!(value_at(&low, 10), Some(ParameterValue::Float(0.4)));
        let high = Driving { master: Some(0.6), ..low };
        assert_eq!(value_at(&high, 10), Some(ParameterValue::Float(0.6)));
        // And the base's own fader scales the base.
        let base_down = Driving { beside: Driving::beside_from(&layers, |_| Some(0.5)), ..low };
        assert_eq!(value_at(&base_down, 10), Some(ParameterValue::Float(0.2)));
    }

    #[test]
    fn a_running_fade_beside_the_winner_keeps_a_parameter_from_settling() {
        let layers = [HtpLayer { effect: None, fade: Some(a_fade(0.0, 1.0, 1_000, 2_000)) }];
        let landed = a_fade(0.5, 0.5, 0, 0);
        let driving = Driving { fade: Some(&landed), beside: Driving::beside_from(&layers, |_| None), ..Default::default() };
        assert_eq!(settles_at(&driving), Some(3_000));
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

    // ── A recording as a layer ────────────────────────────────────────────────

    fn a_track(points: &[(u32, f32)], transport: Transport) -> (Vec<TrackPoint>, Transport) {
        (
            points
                .iter()
                .map(|(ms, v)| TrackPoint { release: false, ms: *ms, value: ParameterValue::Float(*v) })
                .collect(),
            transport,
        )
    }

    fn rolling(at: u64) -> Transport {
        Transport { anchor_ms: at, position_at_anchor_ms: 0, rate: 1.0 }
    }

    #[test]
    fn a_track_beats_a_fade_and_the_programmer_beats_it() {
        let (points, transport) = a_track(&[(0, 0.9)], rolling(1_000));
        let fade = a_fade(0.0, 0.1, 0, 10);
        let held = ParameterValue::Float(0.2);

        let over_fade = Driving {
            track: Some(TrackAt { points: &points, transport }),
            fade: Some(&fade),
            ..Default::default()
        };
        assert_eq!(value_at(&over_fade, 1_500), Some(ParameterValue::Float(0.9)));

        let over_everything = Driving { programmer: Some(&held), ..over_fade };
        assert_eq!(value_at(&over_everything, 1_500), Some(ParameterValue::Float(0.2)));
    }

    /// The one layer that can be present and assert nothing: a recording that starts
    /// a second in says nothing about the first second, and whatever is under it goes
    /// on showing.
    #[test]
    fn before_its_first_point_a_track_lets_what_is_under_it_through() {
        let (points, transport) = a_track(&[(1_000, 1.0)], rolling(0));
        let home = ParameterValue::Float(0.25);
        let driving = Driving {
            track: Some(TrackAt { points: &points, transport }),
            home: Some(&home),
            ..Default::default()
        };
        assert_eq!(value_at(&driving, 500), Some(ParameterValue::Float(0.25)));
        assert_eq!(value_at(&driving, 1_500), Some(ParameterValue::Float(1.0)));
        assert!(driving.is_driven(), "it is still a layer, even where it says nothing");
    }

    /// A master dims what its sequence is playing and nothing above or below it.
    #[test]
    fn a_master_scales_playback_and_only_playback() {
        let fade = a_fade(0.0, 0.8, 0, 0);
        let home = ParameterValue::Float(0.5);
        let played = Driving { fade: Some(&fade), home: Some(&home), master: Some(0.5), ..Default::default() };
        assert_eq!(value_at(&played, 10), Some(ParameterValue::Float(0.4)));

        let effect = an_effect(0);
        let over = Driving { effect: Some(&effect), ..played };
        assert_eq!(value_at(&over, 250), Some(ParameterValue::Float(0.125)), "an effect too");

        let held = ParameterValue::Float(0.9);
        let programmed = Driving { programmer: Some(&held), ..played };
        assert_eq!(value_at(&programmed, 10), Some(ParameterValue::Float(0.9)), "never the programmer");

        let resting = Driving { home: Some(&home), master: Some(0.0), ..Default::default() };
        assert_eq!(value_at(&resting, 10), Some(ParameterValue::Float(0.5)), "never home");

        let (points, transport) = a_track(&[(0, 0.7)], rolling(0));
        let recorded = Driving { track: Some(TrackAt { points: &points, transport }), ..played };
        assert_eq!(value_at(&recorded, 10), Some(ParameterValue::Float(0.7)), "never a take");
    }

    /// A playing recording has no moment after which nothing more happens, so a
    /// connector asking whether it may drop to its keep-alive is told no.
    #[test]
    fn a_playing_track_never_settles() {
        let (points, transport) = a_track(&[(0, 1.0)], rolling(0));
        let driving =
            Driving { track: Some(TrackAt { points: &points, transport }), ..Default::default() };
        assert_eq!(settles_at(&driving), None);
    }
}
