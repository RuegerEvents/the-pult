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
    effect::{EffectSource, Easing, RunningEffect, RunningFade},
    fixture::{home_value_by_key, Fixture, FixtureType, ParameterValue},
    programmer::ProgrammerValue,
    sequence::Sequence,
    speedmaster::SpeedMaster,
};

/// The `Fixture::live_values` map key for a parameter, re-exported from the schema
/// where it lives now: the browser and the command-line plugin derive the same key,
/// and one of them being right is the only version of this worth having.
pub use pult_schema::types::fixture::parameter_key;
use uuid::Uuid;

use super::effects;

mod programmer;
use programmer::{Key, Overlay};

/// How often the engine ticks playback. 25 ms is 40 Hz, comfortably finer than
/// DMX's 44 Hz refresh and far finer than an operator can see.
pub const TICK: Duration = Duration::from_millis(25);

// ── Effects ───────────────────────────────────────────────────────────────────

/// A change playback wants made. The engine turns each one into a path write.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEffect {
    SetLiveValues { fixture_id: Uuid, values: HashMap<String, ParameterValue> },
    /// What is periodic on this fixture right now, keyed by parameter key.
    ///
    /// Not an instruction to change anything: the values are already being written by
    /// `SetLiveValues`. This is the description of *why* they are moving, so an output
    /// plugin can hand the shape to a node that can trace it and stop sending samples.
    SetLiveEffects { fixture_id: Uuid, effects: HashMap<String, RunningEffect> },
    /// The fades this station is part way through, described so a node could run one
    /// unattended instead.
    SetLiveFades { fixture_id: Uuid, fades: HashMap<String, RunningFade> },
    SetCueActive { cue_id: Uuid, is_active: bool },
    GoNext { sequence_id: Uuid, at: u64 },
}

// ── View ──────────────────────────────────────────────────────────────────────

/// What playback needs to see of the show on a given tick.
pub struct ShowView<'a> {
    pub sequences: &'a [Sequence],
    pub cues: HashMap<Uuid, &'a Cue>,
    pub fixtures: &'a [Fixture],
    /// The same fixtures, by id.
    ///
    /// Built once a tick because everything that walks the rig then has to look one
    /// up: `emit` for every fixture that moved, `live_value` for every key a fade or
    /// an effect starts on. Scanning the slice for each of those made the tick
    /// quadratic in the size of the rig, which nothing noticed while a settled show
    /// stopped ticking — and an effect never lets it settle.
    by_id: HashMap<Uuid, &'a Fixture>,
    /// The types those fixtures were patched as, by id.
    ///
    /// Here for one question: where does a parameter rest when nothing is driving it.
    /// A handful of rows where `fixtures` is thousands, so this is not the per-tick
    /// cost that a rig of movers is.
    types_by_id: HashMap<Uuid, &'a FixtureType>,
    /// What the programmer is holding. Replicated show state like everything else
    /// here, so every node computes the same overridden output for itself.
    pub programmer: &'a [ProgrammerValue],
    /// The tempos effects can follow. Replicated for the same reason.
    pub speed_masters: &'a [SpeedMaster],
    /// How long a parameter takes to reach its home value. Show data, so two stations
    /// letting go of one rig let go of it together.
    pub home_fade_ms: u32,
}

impl<'a> ShowView<'a> {
    pub fn new(
        sequences: &'a [Sequence],
        cues: &'a [Cue],
        fixtures: &'a [Fixture],
        fixture_types: &'a [FixtureType],
        programmer: &'a [ProgrammerValue],
        speed_masters: &'a [SpeedMaster],
        home_fade_ms: u32,
    ) -> Self {
        Self {
            sequences,
            cues: cues.iter().map(|c| (c.id, c)).collect(),
            fixtures,
            by_id: fixtures.iter().map(|f| (f.id, f)).collect(),
            types_by_id: fixture_types.iter().map(|t| (t.id, t)).collect(),
            programmer,
            speed_masters,
            home_fade_ms,
        }
    }

    /// One fixture, by id.
    pub(super) fn fixture(&self, id: Uuid) -> Option<&'a Fixture> {
        self.by_id.get(&id).copied()
    }

    /// The sequence a cue is playing under, and when that sequence last went.
    fn anchor_for(&self, sequence: &Sequence, fallback: u64) -> u64 {
        sequence.went_at.unwrap_or(fallback)
    }

    /// The cue a sequence is currently on, if any.
    fn active_cue(&self, sequence: &Sequence) -> Option<&'a Cue> {
        let index = sequence.active_cue_index?;
        let cue_id = sequence.cue_ids.get(index)?;
        self.cues.get(cue_id).copied()
    }

    pub(super) fn live_value(&self, fixture_id: Uuid, key: &str) -> Option<ParameterValue> {
        self.fixture(fixture_id)?.live_values.get(key).cloned()
    }

    /// Where a parameter rests when nothing is driving it: this fixture's own
    /// override, or what its type declares.
    ///
    /// Looked up by key rather than by kind, because everything on this side of the
    /// engine already holds the key — a fade, a held programmer entry — and going back
    /// to a kind to come forward to the same string again would be a second place for
    /// the two to disagree.
    pub(super) fn home_value(&self, fixture_id: Uuid, key: &str) -> Option<ParameterValue> {
        let fixture = self.fixture(fixture_id)?;
        let fixture_type = self.types_by_id.get(&fixture.fixture_type_id).copied();
        home_value_by_key(fixture, fixture_type, key)
    }

    /// Every parameter any cue of this sequence captures: what it could drive.
    ///
    /// Read from the show rather than remembered, so that two stations answer it the
    /// same however much of the sequence each of them has watched run.
    fn captured_by(&self, sequence: &Sequence) -> Vec<Key> {
        let mut keys: Vec<Key> = sequence
            .cue_ids
            .iter()
            .filter_map(|id| self.cues.get(id))
            .flat_map(|cue| cue.captures.iter())
            .map(|c| (c.fixture_id, parameter_key(&c.parameter_kind)))
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// The same, for every sequence that is on — optionally leaving one out, which is
    /// how a sequence being taken off asks what the others still want.
    fn captured_by_the_sequences_that_are_on(
        &self,
        except: Option<Uuid>,
    ) -> std::collections::HashSet<Key> {
        self.sequences
            .iter()
            .filter(|s| s.active_cue_index.is_some() && Some(s.id) != except)
            .flat_map(|s| self.captured_by(s))
            .collect()
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
    /// The same instant on the wall clock, so a node that can run the fade itself can
    /// be told when it began rather than how far through it is.
    t0_ms: u64,
    easing: Easing,
    cue_id: Uuid,
}

impl Fade {
    /// Whether this fade has arrived, and what it is asserting, both measured on the
    /// wall clock rather than the monotonic one.
    ///
    /// The browser has to work the same numbers out and has only the wall clock to do
    /// it with, so the station goes through the same evaluator rather than keeping a
    /// second copy of the arithmetic. `t0_ms` and `start` are the same instant
    /// expressed twice, so this is what it always computed.
    fn is_done(&self, now_ms: u64) -> bool {
        pult_render::fade_is_done(&self.running(), now_ms)
    }

    fn value_at(&self, now_ms: u64) -> ParameterValue {
        pult_render::fade_value_at(&self.running(), now_ms)
    }

    /// This fade, described well enough for somebody else to run it.
    fn running(&self) -> RunningFade {
        RunningFade {
            from: self.from.clone(),
            to: self.to.clone(),
            t0: self.t0_ms,
            duration_ms: self.duration.as_millis() as u32,
            easing: self.easing,
            cue_id: self.cue_id,
        }
    }

    /// When this fade finishes, including any delay before it starts.
    fn ends_at(&self) -> Instant {
        self.start + self.duration
    }
}

// ── Playback ──────────────────────────────────────────────────────────────────

/// Values a tick wants written, gathered before anything is emitted.
///
/// Fades and the programmer both write here, and only then is each fixture's map
/// compared against what was last handed to the engine. Emitting from each in turn
/// would have the second overwrite the first's map with a stale copy of the fixture.
type Changes = HashMap<Uuid, HashMap<String, ParameterValue>>;

#[derive(Default)]
pub struct Playback {
    /// The cue each sequence is currently playing, so a change can be spotted.
    playing: HashMap<Uuid, Uuid>,
    /// Running fades, at most one per (fixture, parameter).
    fades: Vec<Fade>,
    /// Effects a cue is asserting, at most one per (fixture, parameter). The
    /// programmer's own live in the overlay, because that is what decides precedence.
    effects: HashMap<Key, RunningEffect>,
    /// What was last handed to the engine as `live_effects` and `live_fades`, so an
    /// unchanged description is not written again forty times a second.
    ///
    /// Unlike `emit`, which deliberately keeps no such record, nothing else in the
    /// system writes these two fields: they are LOCAL and this is their only writer,
    /// so there is no other hand for the cache to be wrong about.
    motion: HashMap<Uuid, (HashMap<String, RunningEffect>, HashMap<String, RunningFade>)>,
    /// Sequences with a follow cue due, and when.
    follows: HashMap<Uuid, Instant>,
    /// What the programmer is holding over playback.
    overlay: Overlay,
}

impl Playback {
    /// True while a fade is running or a follow cue is pending. The engine skips the
    /// tick entirely when this is false and nothing in the show has changed.
    pub fn has_work(&self) -> bool {
        !self.fades.is_empty()
            || !self.effects.is_empty()
            || !self.follows.is_empty()
            || self.overlay.has_work()
    }

    /// One pass. `now` measures durations, `wall_ms` places them on the console clock.
    ///
    /// Two clocks because they answer different questions. A fade's progress is an
    /// elapsed duration and `Instant` is the only clock that cannot go backwards
    /// under it. An effect's phase is a position on a clock every station shares, and
    /// only the wall clock is shared.
    pub fn tick(
        &mut self,
        now: Instant,
        wall_ms: u64,
        view: &ShowView<'_>,
    ) -> Vec<PlaybackEffect> {
        let mut effects = Vec::new();
        self.track_cue_changes(now, wall_ms, view, &mut effects);

        let mut changes = Changes::new();
        self.advance_fades(wall_ms, &mut changes);
        // After the fades, so a cue asserting an effect on a key wins over a fade the
        // same cue started on it; before the overlay, so the programmer still covers
        // both.
        self.render_effects(wall_ms, &mut changes);
        self.overlay.assert(view, wall_ms, &mut changes);
        self.emit(view, changes, &mut effects);
        self.emit_motion(view, &mut effects);

        self.fire_due_follows(now, wall_ms, &mut effects);
        effects
    }

    /// Spot sequences that moved to a different cue, and start that cue's fades.
    fn track_cue_changes(
        &mut self,
        now: Instant,
        wall_ms: u64,
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
                // A cue that is no longer up stops asserting its effects. Its fades are
                // left to finish, because a fade has somewhere to arrive and an effect
                // does not.
                self.effects.retain(|_, fx| fx.source != EffectSource::Cue(cue_id));
            }
            self.follows.remove(&sequence.id);

            let Some(cue_id) = next else {
                // No cue active means the sequence was taken off — the only way to
                // reach it, now that Go at the last cue stays there. So everything it
                // was driving and nothing else is still driving goes home.
                self.playing.remove(&sequence.id);
                self.release_sequence(now, wall_ms, sequence, view);
                continue;
            };
            self.playing.insert(sequence.id, cue_id);
            effects.push(PlaybackEffect::SetCueActive { cue_id, is_active: true });

            if let Some(cue) = view.cues.get(&cue_id) {
                let anchor = view.anchor_for(sequence, wall_ms);
                self.start_cue(now, wall_ms, anchor, cue, view, sequence.id);
            }
        }
    }

    /// Put back everything a sequence that has just been taken off was driving.
    ///
    /// **What it could drive, read from the show** — the parameters captured by any of
    /// its cues — rather than what this station has watched it write. The obvious
    /// alternative is to remember, per sequence, the keys it has actually touched
    /// since it went on, and that memory is per station: a console that joined at the
    /// interval never ran act one and would take fewer parameters home than the
    /// console that did, which is two rigs looking different with no way back. Reading
    /// the cues is stateless and identical everywhere, and it is right for the same
    /// reason — a parameter no cue of any live sequence captures is a parameter
    /// nothing is driving.
    ///
    /// Two exceptions, both of which err towards leaving a value alone. A parameter
    /// another sequence that is still on *could* drive is not touched, even if that
    /// sequence has not reached the cue that drives it. And a parameter the programmer
    /// holds is the operator's; the overlay puts it back on release, and what it puts
    /// back is by then the home value, since nothing else is asserting it.
    fn release_sequence(
        &mut self,
        now: Instant,
        wall_ms: u64,
        sequence: &Sequence,
        view: &ShowView<'_>,
    ) {
        let mine = view.captured_by(sequence);
        if mine.is_empty() {
            return;
        }
        let still_driven = view.captured_by_the_sequences_that_are_on(Some(sequence.id));

        let duration = Duration::from_millis(view.home_fade_ms as u64);
        for (fixture_id, key) in mine {
            if still_driven.contains(&(fixture_id, key.clone())) {
                continue;
            }
            // Its own fades and effects stop: they were this sequence asserting
            // something, and it has been told to stop asserting. A cue *changing*
            // leaves a fade to finish because it has somewhere to arrive; a sequence
            // going off does not.
            self.fades.retain(|f| !(f.fixture_id == fixture_id && f.key == key));
            self.effects.remove(&(fixture_id, key.clone()));

            let Some(to) = view.home_value(fixture_id, &key) else {
                continue;
            };
            let from = view.live_value(fixture_id, &key).unwrap_or_else(|| to.clone());
            if from == to {
                continue;
            }

            self.fades.push(Fade {
                fixture_id,
                key,
                from,
                to,
                start: now,
                duration,
                t0_ms: wall_ms,
                easing: Easing::Linear,
                // No cue is doing this. A node told about the movement is told about a
                // movement, and the panel that asks "is this my cue's fade" gets no.
                cue_id: Uuid::nil(),
                // A zero duration lands on the next tick, which is what a show that
                // has not asked for a home time wants: releasing has always snapped.
            });
        }
    }

    /// Begin whatever this cue asks of every parameter it captures.
    ///
    /// The anchor is the sequence's `went_at`, not this station's idea of now. Two
    /// consoles process the same Go milliseconds apart, and a fade started from each
    /// one's own arrival would leave them visibly out of step for the length of the
    /// fade; an effect anchored that way would stay out of step for good.
    fn start_cue(
        &mut self,
        now: Instant,
        wall_ms: u64,
        anchor: u64,
        cue: &Cue,
        view: &ShowView<'_>,
        sequence_id: Uuid,
    ) {
        // Where the anchor falls on the monotonic clock, which is what fades measure
        // against. A cue that went before this station got the message started in the
        // past, so the fade is already part way through.
        let anchor_instant = shift(now, anchor as i64 - wall_ms as i64);
        let mut latest_end = now;

        for capture in &cue.captures {
            let key = parameter_key(&capture.parameter_kind);
            let at = (capture.fixture_id, key.clone());

            // Whichever this capture asserts, it takes the key off the other.
            self.fades.retain(|f| !(f.fixture_id == capture.fixture_id && f.key == key));
            self.effects.remove(&at);

            if let Some(spec) = &capture.effect {
                self.effects.insert(
                    at,
                    effects::resolve(
                        spec,
                        view.speed_masters,
                        anchor,
                        EffectSource::Cue(cue.id),
                    ),
                );
                continue;
            }

            let delay = Duration::from_millis(capture.delay_in_ms as u64);

            // Fade from wherever the parameter is now, so re-cueing mid-fade is
            // smooth — and from where it rests when nothing has ever driven it, which
            // is the fixture's answer rather than a zero of the right shape.
            let from = self
                .fades
                .iter()
                .find(|f| f.fixture_id == capture.fixture_id && f.key == key)
                .map(|f| f.value_at(wall_ms))
                .or_else(|| view.live_value(capture.fixture_id, &key))
                .or_else(|| view.home_value(capture.fixture_id, &key))
                // A fixture whose type has gone: nothing can say where it rests, so
                // the cue lands rather than fading from a zero nobody vouched for.
                .unwrap_or_else(|| capture.value.clone());

            // A capture's own time wins; zero means "use the cue's". Which of the two
            // the parameter takes is decided by where it is going — a split fade is
            // the whole reason a cue has an out time as well as an in time.
            let up = if capture.fade_in_ms > 0 { capture.fade_in_ms } else { cue.fade_in_ms };
            let down = match (capture.fade_out_ms, cue.fade_out_ms) {
                (0, 0) => up,
                (0, cue_out) => cue_out,
                (own, _) => own,
            };
            let duration = Duration::from_millis(
                if descending(&from, &capture.value) { down } else { up } as u64,
            );

            let fade = Fade {
                fixture_id: capture.fixture_id,
                key: key.clone(),
                from,
                to: capture.value.clone(),
                start: anchor_instant + delay,
                duration,
                t0_ms: anchor + capture.delay_in_ms as u64,
                easing: capture.easing,
                cue_id: cue.id,
            };
            latest_end = latest_end.max(fade.ends_at());

            self.fades.push(fade);
        }

        if let FollowMode::FollowAfter { delay_ms } = cue.follow_mode {
            // "After the previous cue completes, plus a delay": the fade has to land first.
            self.follows.insert(sequence_id, latest_end + Duration::from_millis(delay_ms as u64));
        }
        // Timecode follows need a timecode source, which does not exist yet.
    }

    /// Move every running fade forward.
    ///
    /// A fade under a key the programmer holds keeps running: it does not reach the
    /// output, but it does say where that key would land if the value were released,
    /// so it is recorded rather than dropped.
    fn advance_fades(&mut self, wall_ms: u64, changes: &mut Changes) {
        if self.fades.is_empty() {
            return;
        }

        let advanced: Vec<(Uuid, String, ParameterValue)> = self
            .fades
            .iter()
            .filter(|fade| wall_ms >= fade.t0_ms) // anything else is still inside its delay
            .map(|fade| (fade.fixture_id, fade.key.clone(), fade.value_at(wall_ms)))
            .collect();

        for (fixture_id, key, value) in advanced {
            match self.overlay.covering(fixture_id, &key, wall_ms) {
                Some(held) => {
                    self.overlay.note_beneath(fixture_id, &key, value);
                    changes.entry(fixture_id).or_default().insert(key, held);
                }
                None => {
                    changes.entry(fixture_id).or_default().insert(key, value);
                }
            }
        }

        self.fades.retain(|f| !f.is_done(wall_ms));
    }

    /// Render every effect a cue is asserting.
    ///
    /// The same arrangement as a fade under a held key: the value is worked out and
    /// recorded as what is underneath, so releasing the programmer lands on where the
    /// effect has got to by then rather than where it was when the key was grabbed.
    fn render_effects(&mut self, wall_ms: u64, changes: &mut Changes) {
        if self.effects.is_empty() {
            return;
        }

        let rendered: Vec<(Uuid, String, ParameterValue)> = self
            .effects
            .iter()
            .map(|(key, effect)| {
                (key.0, key.1.clone(), effects::value_at(effect, wall_ms))
            })
            .collect();

        for (fixture_id, key, value) in rendered {
            match self.overlay.covering(fixture_id, &key, wall_ms) {
                Some(held) => {
                    self.overlay.note_beneath(fixture_id, &key, value);
                    changes.entry(fixture_id).or_default().insert(key, held);
                }
                None => {
                    changes.entry(fixture_id).or_default().insert(key, value);
                }
            }
        }
    }

    /// Hand the tick's changes to the engine, one effect per fixture that moved.
    ///
    /// What was written last tick is not remembered, because the show itself is the
    /// better record of it: a flow action or a device input can write a live value
    /// between two ticks, and a cache of playback's own writes would report that
    /// fixture as already correct and never put it back.
    fn emit(&self, view: &ShowView<'_>, changes: Changes, effects: &mut Vec<PlaybackEffect>) {
        for (fixture_id, changed) in changes {
            // A fixture that has left the rig has nowhere for a value to land.
            let Some(fixture) = view.fixture(fixture_id) else {
                continue;
            };
            // Merge onto what the fixture already has, so a fade for one parameter
            // does not drop the others.
            let mut values = fixture.live_values.clone();
            values.extend(changed);

            if values == fixture.live_values {
                continue;
            }
            effects.push(PlaybackEffect::SetLiveValues { fixture_id, values });
        }
    }

    /// Say what is moving, and why, for the plugins and panels that cannot work it out.
    ///
    /// Only the winner per key is listed. A plain programmer value over a cue effect
    /// produces no entry at all, which is how a node holding a shape gets told to stop
    /// tracing it and take a value instead. A fade under a hold or under an effect is
    /// likewise not listed: it is still running, but it is not what anybody is seeing.
    fn emit_motion(&mut self, view: &ShowView<'_>, out: &mut Vec<PlaybackEffect>) {
        let mut per_fixture: HashMap<Uuid, (HashMap<String, RunningEffect>, HashMap<String, RunningFade>)> =
            HashMap::new();

        for (key, effect) in &self.effects {
            if self.overlay.holds(key.0, &key.1) {
                continue;
            }
            per_fixture.entry(key.0).or_default().0.insert(key.1.clone(), effect.clone());
        }
        // The programmer writes last here too, so its effect covers the cue's.
        for (key, effect) in self.overlay.held_effects() {
            per_fixture.entry(key.0).or_default().0.insert(key.1.clone(), effect.clone());
        }

        for fade in &self.fades {
            let covered = self.overlay.holds(fade.fixture_id, &fade.key)
                || self.effects.contains_key(&(fade.fixture_id, fade.key.clone()));
            if covered {
                continue;
            }
            per_fixture
                .entry(fade.fixture_id)
                .or_default()
                .1
                .insert(fade.key.clone(), fade.running());
        }

        // Every fixture that had motion last tick has to be considered too, or one
        // that has just stopped would keep its last description for ever.
        let considered: Vec<Uuid> = view
            .fixtures
            .iter()
            .map(|f| f.id)
            .filter(|id| per_fixture.contains_key(id) || self.motion.contains_key(id))
            .collect();

        for fixture_id in considered {
            let (effects, fades) = per_fixture.remove(&fixture_id).unwrap_or_default();
            let previous = self.motion.get(&fixture_id);

            if previous.map(|(e, f)| (e, f)) != Some((&effects, &fades)) {
                out.push(PlaybackEffect::SetLiveEffects {
                    fixture_id,
                    effects: effects.clone(),
                });
                out.push(PlaybackEffect::SetLiveFades { fixture_id, fades: fades.clone() });
            }

            if effects.is_empty() && fades.is_empty() {
                self.motion.remove(&fixture_id);
            } else {
                self.motion.insert(fixture_id, (effects, fades));
            }
        }

        // A fixture that has left the rig takes its record with it.
        self.motion.retain(|id, _| view.by_id.contains_key(id));
    }

    fn fire_due_follows(&mut self, now: Instant, wall_ms: u64, effects: &mut Vec<PlaybackEffect>) {
        let due: Vec<Uuid> = self
            .follows
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(seq_id, _)| *seq_id)
            .collect();
        for sequence_id in due {
            self.follows.remove(&sequence_id);
            // The follow fires at one instant; carrying it means every station anchors
            // the cue it fires there, rather than wherever its own actor got to.
            effects.push(PlaybackEffect::GoNext { sequence_id, at: wall_ms });
        }
    }
}

/// `now`, moved by a signed number of milliseconds.
///
/// A cue anchored before this process started cannot be placed on the monotonic clock
/// at all, and `now` is then the closest thing to the truth there is: the fade is
/// treated as beginning here rather than panicking or running backwards.
fn shift(now: Instant, by_ms: i64) -> Instant {
    if by_ms >= 0 {
        now + Duration::from_millis(by_ms as u64)
    } else {
        now.checked_sub(Duration::from_millis((-by_ms) as u64)).unwrap_or(now)
    }
}

// ── Parameter helpers ─────────────────────────────────────────────────────────

/// Is this capture asking the parameter to come down rather than to go up?
///
/// Which decides whether it takes the out time or the in time. Only values with an
/// order to be on can answer: a colour has three of them and no agreed way to rank
/// them, a relay has none at all, and a fixture whose parameter changes kind
/// mid-show is a mistake rather than a direction. All of those take the in time,
/// which is the one time they have always taken — so a show that never sets an out
/// time runs exactly as it did.
fn descending(from: &ParameterValue, to: &ParameterValue) -> bool {
    use ParameterValue::*;
    match (from, to) {
        (Float(a), Float(b)) => b < a,
        (Int(a), Int(b)) => b < a,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
