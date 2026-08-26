//! Cue playback: fades, active-cue tracking, and follow cues.
//!
//! [`Playback`] is a pure state machine. [`Playback::tick`] takes the current show
//! and a timestamp and returns the effects to apply. It never reads a clock and never
//! touches the engine, so a test can drive an entire fade in a few microseconds.
//!
//! The engine applies the effects it returns with LOCAL lifecycle. Live values and
//! active-cue flags are derived from cue state, and cue state is already replicated,
//! so every node computes the same output from the same input. Fanning the derived
//! values out to peers as well would be several hundred redundant messages a second
//! during a fade, and every node would be writing over every other node's copy.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use pult_schema::types::{
    cue::{Cue, FollowMode},
    fixture::{Fixture, ParameterKind, ParameterValue},
    sequence::Sequence,
};
use uuid::Uuid;

/// How often the engine ticks playback. 25 ms is 40 Hz, comfortably finer than
/// DMX's 44 Hz refresh and far finer than an operator can see.
pub const TICK: Duration = Duration::from_millis(25);

// ── Effects ───────────────────────────────────────────────────────────────────

/// A change playback wants made. The engine turns each one into a path write.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEffect {
    SetLiveValues { fixture_id: Uuid, values: HashMap<String, ParameterValue> },
    SetCueActive { cue_id: Uuid, is_active: bool },
    GoNext { sequence_id: Uuid },
}

// ── View ──────────────────────────────────────────────────────────────────────

/// What playback needs to see of the show on a given tick.
pub struct ShowView<'a> {
    pub sequences: &'a [Sequence],
    pub cues: HashMap<Uuid, &'a Cue>,
    pub fixtures: &'a [Fixture],
}

impl<'a> ShowView<'a> {
    pub fn new(sequences: &'a [Sequence], cues: &'a [Cue], fixtures: &'a [Fixture]) -> Self {
        Self { sequences, cues: cues.iter().map(|c| (c.id, c)).collect(), fixtures }
    }

    /// The cue a sequence is currently on, if any.
    fn active_cue(&self, sequence: &Sequence) -> Option<&'a Cue> {
        let index = sequence.active_cue_index?;
        let cue_id = sequence.cue_ids.get(index)?;
        self.cues.get(cue_id).copied()
    }

    fn live_value(&self, fixture_id: Uuid, key: &str) -> Option<ParameterValue> {
        self.fixtures
            .iter()
            .find(|f| f.id == fixture_id)?
            .live_values
            .get(key)
            .cloned()
    }
}

// ── Fades ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Fade {
    fixture_id: Uuid,
    key: String,
    from: ParameterValue,
    to: ParameterValue,
    /// When interpolation starts. Capture delay is already folded in.
    start: Instant,
    duration: Duration,
}

impl Fade {
    /// Position through the fade at `now`, 0.0 before it starts and 1.0 once done.
    fn progress(&self, now: Instant) -> f32 {
        if now < self.start {
            return 0.0;
        }
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.duration_since(self.start).as_secs_f32();
        (elapsed / self.duration.as_secs_f32()).min(1.0)
    }

    fn is_done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    fn value_at(&self, now: Instant) -> ParameterValue {
        interpolate(&self.from, &self.to, self.progress(now))
    }

    /// When this fade finishes, including any delay before it starts.
    fn ends_at(&self) -> Instant {
        self.start + self.duration
    }
}

// ── Playback ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct Playback {
    /// The cue each sequence is currently playing, so a change can be spotted.
    playing: HashMap<Uuid, Uuid>,
    /// Running fades, at most one per (fixture, parameter).
    fades: Vec<Fade>,
    /// Sequences with a follow cue due, and when.
    follows: HashMap<Uuid, Instant>,
    /// Last values handed to the engine, so unchanged fixtures are not rewritten.
    emitted: HashMap<Uuid, HashMap<String, ParameterValue>>,
}

impl Playback {
    /// True while a fade is running or a follow cue is pending. The engine skips the
    /// tick entirely when this is false and nothing in the show has changed.
    pub fn has_work(&self) -> bool {
        !self.fades.is_empty() || !self.follows.is_empty()
    }

    pub fn tick(&mut self, now: Instant, view: &ShowView<'_>) -> Vec<PlaybackEffect> {
        let mut effects = Vec::new();
        self.track_cue_changes(now, view, &mut effects);
        self.advance_fades(now, view, &mut effects);
        self.fire_due_follows(now, &mut effects);
        effects
    }

    /// Spot sequences that moved to a different cue, and start that cue's fades.
    fn track_cue_changes(
        &mut self,
        now: Instant,
        view: &ShowView<'_>,
        effects: &mut Vec<PlaybackEffect>,
    ) {
        let live: HashMap<Uuid, Option<Uuid>> = view
            .sequences
            .iter()
            .map(|s| (s.id, view.active_cue(s).map(|c| c.id)))
            .collect();

        // Sequences that have gone away stop playing whatever they were on.
        self.playing.retain(|seq_id, cue_id| {
            let gone = !live.contains_key(seq_id);
            if gone {
                effects.push(PlaybackEffect::SetCueActive { cue_id: *cue_id, is_active: false });
            }
            !gone
        });

        for sequence in view.sequences {
            let next = live.get(&sequence.id).copied().flatten();
            let previous = self.playing.get(&sequence.id).copied();
            if next == previous {
                continue;
            }

            if let Some(cue_id) = previous {
                effects.push(PlaybackEffect::SetCueActive { cue_id, is_active: false });
            }
            self.follows.remove(&sequence.id);

            let Some(cue_id) = next else {
                // Off the end of the sequence. Live values hold where they are;
                // a light does not go dark because the operator ran out of cues.
                self.playing.remove(&sequence.id);
                continue;
            };
            self.playing.insert(sequence.id, cue_id);
            effects.push(PlaybackEffect::SetCueActive { cue_id, is_active: true });

            if let Some(cue) = view.cues.get(&cue_id) {
                self.start_cue(now, cue, view, sequence.id);
            }
        }
    }

    /// Begin fading every parameter this cue captures.
    fn start_cue(&mut self, now: Instant, cue: &Cue, view: &ShowView<'_>, sequence_id: Uuid) {
        let mut latest_end = now;

        for capture in &cue.captures {
            let key = parameter_key(&capture.parameter_kind);
            // A capture's own fade time wins; zero means "use the cue's".
            let duration = Duration::from_millis(
                if capture.fade_in_ms > 0 { capture.fade_in_ms } else { cue.fade_in_ms } as u64,
            );
            let start = now + Duration::from_millis(capture.delay_in_ms as u64);

            // Fade from wherever the parameter is now, so re-cueing mid-fade is smooth.
            let from = self
                .fades
                .iter()
                .find(|f| f.fixture_id == capture.fixture_id && f.key == key)
                .map(|f| f.value_at(now))
                .or_else(|| view.live_value(capture.fixture_id, &key))
                .unwrap_or_else(|| zero_like(&capture.value));

            let fade = Fade {
                fixture_id: capture.fixture_id,
                key: key.clone(),
                from,
                to: capture.value.clone(),
                start,
                duration,
            };
            latest_end = latest_end.max(fade.ends_at());

            self.fades.retain(|f| !(f.fixture_id == capture.fixture_id && f.key == key));
            self.fades.push(fade);
        }

        if let FollowMode::FollowAfter { delay_ms } = cue.follow_mode {
            // "After the previous cue completes, plus a delay": the fade has to land first.
            self.follows.insert(sequence_id, latest_end + Duration::from_millis(delay_ms as u64));
        }
        // Timecode follows need a timecode source, which does not exist yet.
    }

    /// Move every running fade forward and emit the fixtures whose values changed.
    fn advance_fades(
        &mut self,
        now: Instant,
        view: &ShowView<'_>,
        effects: &mut Vec<PlaybackEffect>,
    ) {
        if self.fades.is_empty() {
            return;
        }

        let mut touched: HashMap<Uuid, HashMap<String, ParameterValue>> = HashMap::new();
        for fade in &self.fades {
            if now < fade.start {
                continue; // still inside its delay
            }
            touched.entry(fade.fixture_id).or_default().insert(fade.key.clone(), fade.value_at(now));
        }

        for (fixture_id, changes) in touched {
            // Merge onto what the fixture already has, so a fade for one parameter
            // does not drop the others.
            let mut values = view
                .fixtures
                .iter()
                .find(|f| f.id == fixture_id)
                .map(|f| f.live_values.clone())
                .unwrap_or_default();
            values.extend(changes);

            if self.emitted.get(&fixture_id) == Some(&values) {
                continue;
            }
            self.emitted.insert(fixture_id, values.clone());
            effects.push(PlaybackEffect::SetLiveValues { fixture_id, values });
        }

        self.fades.retain(|f| !f.is_done(now));
    }

    fn fire_due_follows(&mut self, now: Instant, effects: &mut Vec<PlaybackEffect>) {
        let due: Vec<Uuid> = self
            .follows
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(seq_id, _)| *seq_id)
            .collect();
        for sequence_id in due {
            self.follows.remove(&sequence_id);
            effects.push(PlaybackEffect::GoNext { sequence_id });
        }
    }
}

// ── Parameter helpers ─────────────────────────────────────────────────────────

/// The dark, home, or off value of the same kind as `like`. Where a fade starts when
/// the fixture has no recorded value for that parameter yet.
fn zero_like(like: &ParameterValue) -> ParameterValue {
    match like {
        ParameterValue::Float(_) => ParameterValue::Float(0.0),
        ParameterValue::Int(_) => ParameterValue::Int(0),
        ParameterValue::Color { .. } => ParameterValue::Color { r: 0.0, g: 0.0, b: 0.0 },
        ParameterValue::Bool(_) => ParameterValue::Bool(false),
        ParameterValue::Text(_) => ParameterValue::Text(String::new()),
    }
}

/// The `Fixture::live_values` map key for a parameter.
pub fn parameter_key(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Intensity => "Intensity".into(),
        ParameterKind::ColorRgb => "ColorRgb".into(),
        ParameterKind::Pan => "Pan".into(),
        ParameterKind::Tilt => "Tilt".into(),
        ParameterKind::GoboIndex => "GoboIndex".into(),
        ParameterKind::Raw(channel) => format!("Raw:{channel}"),
        ParameterKind::Switch(n) => format!("Switch:{n}"),
        ParameterKind::Contact(n) => format!("Contact:{n}"),
        ParameterKind::Temperature => "Temperature".into(),
        ParameterKind::Humidity => "Humidity".into(),
        ParameterKind::AirQuality => "AirQuality".into(),
        ParameterKind::Text => "Text".into(),
    }
}

/// Blend two parameter values. Values that cannot be blended, and values of
/// different kinds, snap to the target when the fade completes.
fn interpolate(from: &ParameterValue, to: &ParameterValue, t: f32) -> ParameterValue {
    use ParameterValue::*;
    match (from, to) {
        (Float(a), Float(b)) => Float(a + (b - a) * t),
        (Int(a), Int(b)) => Int((*a as f32 + (*b as f32 - *a as f32) * t).round() as i32),
        (Color { r: r0, g: g0, b: b0 }, Color { r: r1, g: g1, b: b1 }) => Color {
            r: r0 + (r1 - r0) * t,
            g: g0 + (g1 - g0) * t,
            b: b0 + (b1 - b0) * t,
        },
        // A boolean has nothing between its two states, so it switches at the start
        // of the fade rather than at the end, where it would look like a late cue.
        (Bool(a), Bool(b)) => Bool(if t > 0.0 { *b } else { *a }),
        (a, b) => {
            if t >= 1.0 {
                b.clone()
            } else {
                a.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests;
