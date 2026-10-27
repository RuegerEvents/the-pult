//! Cue playback: what is driving each parameter, active-cue tracking, follow cues.
//!
//! [`Playback`] is a pure state machine. [`Playback::pass`] takes the current show and
//! a moment and returns the changes to apply. It never reads a clock and never touches
//! the engine, so a test can drive a whole act in a few microseconds.
//!
//! **It decides what is driving a parameter; it does not work out any values.** The
//! fades and effects it publishes are anchored descriptions, and whoever needs a
//! number — an output connector on its own frame, a browser on its own refresh —
//! evaluates one for the moment it is asking about. So a pass happens when the *show*
//! changes rather than at a rate, and a fade in progress is not a pass at all.
//!
//! Which is also why a fade that has landed is kept rather than dropped. Nothing
//! stores the number it landed on any more, so the finished fade is the only record of
//! where the parameter got to — and evaluating one is exactly that constant.
//!
//! What it publishes lands with LOCAL lifecycle. Every station derives the same
//! descriptions from the same replicated cue state, so fanning them out to peers would
//! be sending each console a slower copy of what it has already computed.

use std::collections::HashMap;

use pult_schema::types::{
    cue::{Cue, FollowMode},
    effect::{EffectSource, Easing, RunningEffect, RunningFade},
    fixture::{home_value_by_key, Fixture, FixtureType, ParameterValue},
    preset::{preset_index, Preset},
    programmer::ProgrammerValue,
    sequence::{Masters, Merge, Sequence},
    show::{FadeCurves, FadeGroup},
    speedmaster::SpeedMaster,
};

/// The map key for a parameter, re-exported from the schema where it lives now: the
/// browser and the command-line plugin derive the same key, and one of them being
/// right is the only version of this worth having.
pub use pult_schema::types::fixture::parameter_key;
use pult_render::HtpLayer;
use uuid::Uuid;

use super::effects;

mod programmer;
use programmer::{held_by_the_programmer, Held, Key};

// ── Effects ───────────────────────────────────────────────────────────────────

/// A change playback wants made. The engine turns each one into a path write.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEffect {
    /// What is periodic on this fixture right now, keyed by parameter key.
    ///
    /// A description rather than a value: the shape and its anchor, from which anyone
    /// holding the row works out what the parameter is at any moment they like. Which
    /// is also what lets an output plugin hand the shape to a node that can trace it
    /// and then say nothing more.
    SetLiveEffects { fixture_id: Uuid, effects: HashMap<String, RunningEffect> },
    /// The fades on this fixture, described so a node could run one unattended.
    ///
    /// Including the ones that have arrived. A landed fade is a constant function of
    /// time, and it is the console's only record of where that parameter is.
    SetLiveFades { fixture_id: Uuid, fades: HashMap<String, RunningFade> },
    /// What other sequences are playing beside the winner on an intensity, under an
    /// HTP merge. See `Fixture::live_htp`.
    SetLiveHtp { fixture_id: Uuid, layers: HashMap<String, Vec<HtpLayer>> },
    SetCueActive { cue_id: Uuid, is_active: bool },
    GoNext { sequence_id: Uuid, at: u64 },
}

// ── View ──────────────────────────────────────────────────────────────────────

/// What playback needs to see of the show on a given pass.
pub struct ShowView<'a> {
    pub sequences: &'a [Sequence],
    pub cues: HashMap<Uuid, &'a Cue>,
    pub fixtures: &'a [Fixture],
    /// The same fixtures, by id.
    ///
    /// Built once a pass because everything that walks the rig then has to look one
    /// up: every key a fade or an effect starts on, every fixture whose motion is
    /// republished. Scanning the slice for each of those made a pass quadratic in the
    /// size of the rig.
    by_id: HashMap<Uuid, &'a Fixture>,
    /// The types those fixtures were patched as, by id.
    ///
    /// Here for one question: where does a parameter rest when nothing is driving it.
    /// A handful of rows where `fixtures` is thousands, so this is not the per-pass
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
    /// What shape a fade has when neither the capture nor the cue says one. Show
    /// data for the same reason `home_fade_ms` is.
    pub fade_curves: FadeCurves,
    /// The presets a capture may be a reference to, by id.
    ///
    /// Here and not resolved by the caller because a capture resolves in one place,
    /// [`ParameterCapture::value_in`], and a pass that took resolved values would
    /// have decided the question somewhere else.
    pub presets: HashMap<Uuid, &'a Preset>,
    /// The sequences' playback faders, by the cues those sequences hold.
    pub masters: Masters,
    /// Which sequence each cue belongs to, which is how a fade or an effect says whose
    /// it is.
    sequence_of_cue: HashMap<Uuid, Uuid>,
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
        fade_curves: FadeCurves,
        presets: &'a [Preset],
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
            fade_curves,
            presets: preset_index(presets),
            masters: Masters::of(sequences),
            sequence_of_cue: sequences.iter().flat_map(|s| s.cue_ids.iter().map(move |c| (*c, s.id))).collect(),
        }
    }

    /// The sequences' levels as they are this moment, where the caller knows more than
    /// the faders: a playing take that records a level puts it over its fader, and a
    /// fade a Go starts has to begin from what that level is showing.
    pub fn with_levels(mut self, masters: Masters) -> Self {
        self.masters = masters;
        self
    }

    /// The sequence a cue is in.
    fn sequence_of(&self, cue_id: Uuid) -> Option<Uuid> {
        self.sequence_of_cue.get(&cue_id).copied()
    }

    fn sequence(&self, id: Uuid) -> Option<&'a Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }

    /// Whether this sequence is on: on a cue.
    fn is_on(&self, id: Uuid) -> bool {
        self.sequence(id).is_some_and(|s| s.active_cue_index.is_some())
    }

    /// The capture a sequence that is on is tracking for this key: the latest one over
    /// its stack up to the cue it is on, and the cue it belongs to.
    fn tracked_capture(
        &self,
        sequence: &'a Sequence,
        at: &Key,
    ) -> Option<(&'a Cue, &'a pult_schema::types::cue::ParameterCapture)> {
        let index = sequence.active_cue_index?;
        let mut found = None;
        for id in sequence.cue_ids.iter().take(index + 1) {
            let Some(cue) = self.cues.get(id).copied() else { continue };
            for capture in &cue.captures {
                if capture.fixture_id == at.0 && parameter_key(&capture.parameter_kind) == at.1 {
                    found = Some((cue, capture));
                }
            }
        }
        found
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

    /// The type a fixture is patched as, if this station has the row.
    fn fixture_type(&self, fixture: &Fixture) -> Option<&'a FixtureType> {
        self.types_by_id.get(&fixture.fixture_type_id).copied()
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

/// A fade, and which parameter it is on.
///
/// The description is a [`RunningFade`] and nothing else — no monotonic anchor beside
/// the console one, no cached progress. Playback does not evaluate fades any more, so
/// there is nothing left for a second clock to be more accurate about, and a fade
/// carrying one instant rather than two is a fade that means the same thing on every
/// station and in a browser.
#[derive(Debug, Clone)]
struct Fade {
    fixture_id: Uuid,
    key: String,
    running: RunningFade,
}

impl Fade {
    /// The console millisecond this fade lands on.
    fn ends_at(&self) -> u64 {
        self.running.t0.saturating_add(self.running.duration_ms as u64)
    }
}

/// A parameter parked where it is: a fade of no length, from a value to itself.
///
/// What replaces a value that used to simply stay in a map. An effect that stops
/// without anything taking its key, or a flow setting a parameter outright, both leave
/// the parameter asserting one number for ever — and one number for ever is a fade
/// that has already landed.
fn parked(fixture_id: Uuid, key: String, value: ParameterValue, at: u64) -> Fade {
    Fade {
        fixture_id,
        key,
        running: RunningFade {
            from: value.clone(),
            to: value,
            t0: at,
            duration_ms: 0,
            easing: Easing::Step,
            // Nobody's cue is doing this, so nothing claims it.
            cue_id: Uuid::nil(),
        },
    }
}

// ── Playback ──────────────────────────────────────────────────────────────────

/// What a fixture was last handed: its effects, its fades, and what is beside them.
type Motion = (HashMap<String, RunningEffect>, HashMap<String, RunningFade>, HashMap<String, Vec<HtpLayer>>);

#[derive(Default)]
pub struct Playback {
    /// The cue each sequence is currently playing, so a change can be spotted.
    playing: HashMap<Uuid, Uuid>,
    /// What is driving each (fixture, parameter) as a fade — including the ones that
    /// have arrived, which is what makes this the record of where the rig is.
    fades: Vec<Fade>,
    /// Effects a cue is asserting, at most one per (fixture, parameter). The
    /// programmer's are worked out from its entries on each pass, because that is
    /// where the precedence between the two is decided.
    effects: HashMap<Key, RunningEffect>,
    /// What other sequences that are on go on playing on an intensity an HTP sequence
    /// went over, oldest first, each with the sequence it is. The fade or effect above
    /// is the latest; the light shows the highest of them all. See [`Merge`].
    beside: HashMap<Key, Vec<(Uuid, HtpLayer)>>,
    /// What was last handed to the engine as `live_effects` and `live_fades`, so an
    /// unchanged description is not written again.
    ///
    /// Nothing else in the system writes those two fields: they are LOCAL and this is
    /// their only writer, so there is no other hand for the cache to be wrong about.
    motion: HashMap<Uuid, Motion>,
    /// Sequences with a follow cue due, and the console millisecond it is due at.
    follows: HashMap<Uuid, u64>,
    /// Keys a stopping recording was holding, and what it was holding them at.
    ///
    /// Queued rather than acted on where it is noticed, because letting go of a key is
    /// a change to what playback is asserting and this object is the only thing
    /// allowed to make one. Drained at the top of the next pass, which the engine asks
    /// for immediately.
    releasing_tracks: Vec<(Key, ParameterValue)>,
}

impl Playback {
    /// The next console millisecond at which a pass would do something on its own.
    ///
    /// Only a follow cue answers. A fade needs no pass to progress — nobody is storing
    /// what it is worth — and an effect never ends, so neither of them is a reason to
    /// wake the engine up. This is what a station with a show up and no follows
    /// pending sleeps on: nothing.
    pub fn next_deadline(&self) -> Option<u64> {
        self.follows.values().copied().min()
    }

    /// True while something is outstanding that a pass would act on.
    pub fn has_work(&self) -> bool {
        !self.follows.is_empty() || !self.releasing_tracks.is_empty()
    }

    /// A recording has stopped, and these are the keys it was holding.
    ///
    /// The value is where the track had got to at the moment it stopped, sampled by
    /// the caller — which has the decoded asset and this does not.
    pub fn release_tracks(&mut self, keys: Vec<(Uuid, String, ParameterValue)>) {
        self.releasing_tracks
            .extend(keys.into_iter().map(|(fixture_id, key, value)| ((fixture_id, key), value)));
    }

    /// True when nothing is driving anything and nothing is remembered.
    ///
    /// What the engine asks before reading the rig. A pass is O(rig) because it has
    /// to look at every fixture that could have motion on it, and a show with nothing
    /// running has none — so patching two thousand fixtures into an idle show should
    /// not cost two thousand walks of the rig it is building.
    pub fn is_idle(&self) -> bool {
        self.playing.is_empty()
            && self.fades.is_empty()
            && self.effects.is_empty()
            && self.beside.is_empty()
            && self.motion.is_empty()
            && self.follows.is_empty()
            && self.releasing_tracks.is_empty()
    }

    /// What one parameter is putting out at `wall_ms`, from playback's own layers.
    ///
    /// The same stack a connector or a browser evaluates, read from what this object
    /// is about to publish rather than from the show it published to last time — so a
    /// cue asking where a parameter is mid-pass gets this pass's answer.
    fn value_at(&self, view: &ShowView<'_>, at: &Key, wall_ms: u64) -> Option<ParameterValue> {
        let fixture = view.fixture(at.0)?;
        let held = held_by_the_programmer(view, at);
        // Playback's own memory first, then what the fixture already carries. The two
        // agree everywhere except inside a pass — playback publishes onto the fixture
        // and nothing else writes those two fields — so the fallback matters for the
        // case where this playback has never seen the cue that put it there.
        let driving = pult_render::Driving {
            programmer: match &held {
                Some(Held::Value(value)) => Some(value),
                _ => None,
            },
            // Playback works out what it is *about to publish*, and a recording is
            // not one of its layers: a track is asserted at the connector and in the
            // browser out of an asset this pass has never read.
            track: None,
            effect: match &held {
                Some(Held::Effect(effect)) => Some(effect),
                _ => self.effects.get(at).or_else(|| fixture.live_effects.get(&at.1)),
            },
            fade: self
                .fades
                .iter()
                .find(|f| f.fixture_id == at.0 && f.key == at.1)
                .map(|f| &f.running)
                .or_else(|| fixture.live_fades.get(&at.1)),
            home: None,
            master: None,
            // The winner alone: what a fade starting here starts from, and what a
            // stopped effect parks at, are that sequence's own.
            beside: Default::default(),
        };
        // What is on stage, the sequence's fader included: a fade starting from
        // here is starting from what the audience is looking at.
        let driving = pult_render::Driving {
            master: view.masters.over(&at.1, driving.effect, driving.fade),
            ..driving
        };
        pult_render::value_at(&driving, wall_ms)
            .or_else(|| home_value_by_key(fixture, view.fixture_type(fixture), &at.1))
    }

    /// One pass, placed at `wall_ms` on the console clock.
    ///
    /// Run when the show changes or a follow comes due, and at no other time. What it
    /// returns is a set of descriptions, not a set of values: nothing here works out
    /// what any parameter is worth.
    pub fn pass(&mut self, wall_ms: u64, view: &ShowView<'_>) -> Vec<PlaybackEffect> {
        let mut effects = Vec::new();
        self.let_go_of_stopped_tracks(wall_ms, view);
        self.track_cue_changes(wall_ms, view, &mut effects);
        self.emit_motion(view, &mut effects);
        self.fire_due_follows(wall_ms, &mut effects);
        effects
    }

    /// **An edit to a cue that is standing moves the rig.**
    ///
    /// Which is what Update means on every other desk: change the value, press Update,
    /// and the stage keeps looking the way you set it rather than snapping back to what
    /// the cue said a moment ago. It is also the whole reason a palette is worth having
    /// — change *warm* and the look on stage changes, rather than changing the next
    /// time somebody takes the cue.
    ///
    /// One rule for both, because they are one thing: a standing fade whose cue's
    /// capture no longer agrees with where the fade is going is *restarted* from where
    /// the parameter is, over the show's own home fade.
    ///
    /// Restarted rather than rewritten in place, and the difference matters: a fade
    /// half way through with its `to` swapped would jump, because `from` is where it
    /// started and the eased position between them would move. Fading from
    /// `value_at(now)` is the same thing `release_key` does, for the same reason.
    ///
    /// The home fade is the duration on purpose. The capture's own time is how long
    /// *taking the cue* takes; this is not a cue being taken, it is a value moving
    /// under one, and the show already has one figure for "a parameter is going
    /// somewhere nobody pressed Go for".
    ///
    /// Only run when the cues or the presets have actually moved — the caller gates it
    /// — and the index is built only for the cues the standing fades name, so a rig of
    /// thousands costs a hash lookup each rather than a scan.
    fn repoint_edited_captures(&mut self, wall_ms: u64, view: &ShowView<'_>) {
        // Cue, then fixture, then key — three levels rather than one tuple, so a fade
        // is looked up by the two ids and the key string it already holds and nothing
        // is allocated per fade.
        //
        // **All three, and the fixture is the one that matters.** A cue captures the
        // same key for every fixture in it, so an index keyed by the parameter alone
        // collapses a whole system into whichever lamp came last — and then every fade
        // in that system is compared against a value that is not its own and restarted
        // to it. Which is a cue edit moving twenty lights nobody touched, and is what
        // `demo.sh --demo theater` found.
        type ByFixture<'c> =
            HashMap<Uuid, HashMap<String, &'c pult_schema::types::cue::ParameterCapture>>;
        let mut wanted: HashMap<Uuid, ByFixture<'_>> = HashMap::new();
        for fade in &self.fades {
            if fade.running.cue_id.is_nil() || wanted.contains_key(&fade.running.cue_id) {
                continue;
            }
            let Some(cue) = view.cues.get(&fade.running.cue_id) else { continue };
            let mut by_fixture: ByFixture<'_> = HashMap::new();
            for capture in &cue.captures {
                // A cue that says one fixture's key twice is a show mid-edit; the last
                // one is what `start_capture` would have applied.
                by_fixture
                    .entry(capture.fixture_id)
                    .or_default()
                    .insert(parameter_key(&capture.parameter_kind), capture);
            }
            wanted.insert(cue.id, by_fixture);
        }

        // Which fades want moving, worked out before any of them is touched: starting
        // one changes what `value_at` answers for the next.
        let mut moves: Vec<(Key, Uuid, ParameterValue, ParameterValue)> = Vec::new();
        for fade in &self.fades {
            if fade.running.cue_id.is_nil() {
                continue;
            }
            let Some(capture) = wanted
                .get(&fade.running.cue_id)
                .and_then(|by_fixture| by_fixture.get(&fade.fixture_id))
                .and_then(|by_key| by_key.get(fade.key.as_str()))
            else {
                continue;
            };
            let to = capture.value_in(&view.presets);
            if *to == fade.running.to {
                continue;
            }
            let at: Key = (fade.fixture_id, fade.key.clone());
            let from = self
                .value_at(view, &at, wall_ms)
                .map(|showing| unmastered(view, &at.1, fade.running.cue_id, showing))
                .unwrap_or_else(|| fade.running.from.clone());
            moves.push((at, fade.running.cue_id, from, to.clone()));
        }

        for ((fixture_id, key), cue_id, from, to) in moves {
            self.fades.retain(|f| !(f.fixture_id == fixture_id && f.key == key));
            let easing = view.fade_curves.for_key(&key);
            self.fades.push(Fade {
                fixture_id,
                key,
                running: RunningFade {
                    from,
                    to,
                    t0: wall_ms,
                    duration_ms: view.home_fade_ms,
                    easing,
                    // Still that cue's: the cue is asserting the same key, and a sheet
                    // asking "is this my cue's fade" is still owed a yes.
                    cue_id,
                },
            });
        }

        self.release_dropped_captures(wall_ms, view);
        self.start_new_captures(wall_ms, view);
    }

    /// **And a capture a standing cue lost lets its key go.** Taking a value out of the
    /// cue on stage left its fade where it was, holding the light at a value no cue said
    /// any more until the next Go. Now the key goes where the stack says: to what another
    /// live sequence's tracked stack captures, started the way a Go starts it, or home —
    /// over `home_fade_ms`, from where it is. A key an earlier cue of the *same* sequence
    /// still tracks is `start_new_captures`' to restart, and is left to it.
    fn release_dropped_captures(&mut self, wall_ms: u64, view: &ShowView<'_>) {
        let captured = |cue_id: Uuid, at: &Key| {
            view.cues.get(&cue_id).is_none_or(|cue| {
                cue.captures.iter().any(|c| c.fixture_id == at.0 && parameter_key(&c.parameter_kind) == at.1)
            })
        };
        let mut dropped: Vec<(Key, Uuid)> = self
            .fades
            .iter()
            .filter(|f| !f.running.cue_id.is_nil())
            .map(|f| ((f.fixture_id, f.key.clone()), f.running.cue_id))
            .filter(|(at, cue_id)| !captured(*cue_id, at))
            .collect();
        dropped.extend(self.effects.iter().filter_map(|(at, fx)| match fx.source {
            EffectSource::Cue(cue_id) if !captured(cue_id, at) => Some((at.clone(), cue_id)),
            _ => None,
        }));
        if dropped.is_empty() {
            return;
        }

        let mut live: Vec<&Sequence> = view.sequences.iter().filter(|s| s.active_cue_index.is_some()).collect();
        live.sort_by_key(|s| s.went_at.unwrap_or(0));
        let mut wanted: HashMap<Key, (&Sequence, &Cue, &pult_schema::types::cue::ParameterCapture)> = HashMap::new();
        for sequence in live {
            let Some(active) = sequence.active_cue_index else { continue };
            let through = sequence.cue_ids.iter().take(active + 1);
            for (cue, capture) in pult_schema::types::cue::tracked_through(through, |id| view.cues.get(id).copied()) {
                wanted.insert((capture.fixture_id, parameter_key(&capture.parameter_kind)), (sequence, cue, capture));
            }
        }

        for (at, was) in dropped {
            match wanted.get(&at) {
                // The same sequence still tracks it from an earlier cue: restarted there.
                Some((sequence, _, _)) if sequence.cue_ids.contains(&was) => {}
                // Another live sequence says it: taken over from where it is.
                Some((sequence, cue, capture)) => {
                    let anchor = if capture.effect.is_some() { sequence.went_at.unwrap_or(wall_ms) } else { wall_ms };
                    let (cue, capture) = (*cue, *capture);
                    self.start_capture(wall_ms, anchor, cue, cue, capture, view);
                }
                None => self.release_key(wall_ms, at, view, view.home_fade_ms),
            }
        }
    }

    /// **And a capture a standing cue gained is started.** Repointing reaches only a
    /// fade that already exists, so a key stored into the cue on stage for the first
    /// time — the Store dialog Update opens for the keys no cue was driving — had
    /// nothing to move. It went live only after the sequence was taken off and on
    /// again, and when the programmer cleared, the light fell back to home (found at
    /// the gig, 2026-10-25).
    ///
    /// What should be driving each key is worked out the way a Go works it out: the
    /// latest capture over each live sequence's tracked stack, the sequence that went
    /// most recently winning a contested key. A key is started when nothing of
    /// playback's is driving it, or when an *earlier* cue of the same sequence still
    /// is — the edit made a later cue say it. Anything another sequence or a release is
    /// doing is left alone, and so is a key already heading where it should.
    ///
    /// Started through `start_capture`, from where the parameter is: during Update or a
    /// Store that is the programmer's value, which is the value being stored, so the
    /// light does not move.
    fn start_new_captures(&mut self, wall_ms: u64, view: &ShowView<'_>) {
        let mut live: Vec<&Sequence> = view.sequences.iter().filter(|s| s.active_cue_index.is_some()).collect();
        live.sort_by_key(|s| s.went_at.unwrap_or(0));
        let mut wanted: HashMap<Key, (&Sequence, &Cue, &pult_schema::types::cue::ParameterCapture)> = HashMap::new();
        for sequence in live {
            let Some(active) = sequence.active_cue_index else { continue };
            let through = sequence.cue_ids.iter().take(active + 1);
            for (cue, capture) in pult_schema::types::cue::tracked_through(through, |id| view.cues.get(id).copied()) {
                wanted.insert((capture.fixture_id, parameter_key(&capture.parameter_kind)), (sequence, cue, capture));
            }
        }

        let mut starts: Vec<(&Sequence, &Cue, &pult_schema::types::cue::ParameterCapture)> = Vec::new();
        for (at, (sequence, cue, capture)) in &wanted {
            if let Some(effect) = self.effects.get(at) {
                // An effect is driving it; only one of this sequence's earlier cues
                // gives it up to the capture now standing.
                match effect.source {
                    EffectSource::Cue(id) if id != cue.id && sequence.cue_ids.contains(&id) => {}
                    _ => continue,
                }
            } else if let Some(fade) = self.fades.iter().find(|f| f.fixture_id == at.0 && f.key == at.1) {
                let ours = fade.running.cue_id == cue.id;
                let earlier = !fade.running.cue_id.is_nil() && sequence.cue_ids.contains(&fade.running.cue_id);
                let heading_there = capture.effect.is_none() && fade.running.to == *capture.value_in(&view.presets);
                if ours || !earlier || heading_there {
                    continue;
                }
            }
            starts.push((sequence, cue, capture));
        }
        for (sequence, cue, capture) in starts {
            // An effect keeps the phase the cue's other effects took from its Go; a fade
            // runs from now.
            let anchor = if capture.effect.is_some() { sequence.went_at.unwrap_or(wall_ms) } else { wall_ms };
            self.start_capture(wall_ms, anchor, cue, cue, capture, view);
        }
    }

    /// The same pass, told whether the cues or the presets have moved since the last one.
    pub fn pass_after_edits(
        &mut self,
        wall_ms: u64,
        view: &ShowView<'_>,
        edited: bool,
    ) -> Vec<PlaybackEffect> {
        if edited {
            self.repoint_edited_captures(wall_ms, view);
        }
        self.pass(wall_ms, view)
    }

    /// Put back what a stopped recording was holding.
    ///
    /// A track sits above playback and below the programmer, so when one stops the
    /// parameter has to get from where the recording left it to whatever playback
    /// holds underneath — over the show's own `home_fade_ms`, because letting go is a
    /// move like any other and a head that snaps reads as a fault.
    ///
    /// Two of these rules are the ones a sequence release already follows and one is
    /// its own. **A parameter the programmer holds is untouched**, because it is the
    /// operator's and the overlay puts it back itself. **An effect underneath is left
    /// alone**: an effect beats a fade, so a release fade under one would be invisible,
    /// and the effect showing through at once is what actually happens.
    ///
    /// And the destination is the stack beneath **evaluated at the moment the release
    /// lands**, not at the moment it starts. A cue's fade still in flight underneath
    /// cannot be joined — the model has one fade per key — so this is the honest
    /// approximation: the parameter arrives where playback would have had it, rather
    /// than arriving where playback *was* and then being wrong for the rest of the
    /// cue.
    fn let_go_of_stopped_tracks(&mut self, wall_ms: u64, view: &ShowView<'_>) {
        let releasing = std::mem::take(&mut self.releasing_tracks);
        for (at, from) in releasing {
            if held_by_the_programmer(view, &at).is_some() {
                continue;
            }
            if self.effects.contains_key(&at)
                || view
                    .fixture(at.0)
                    .is_some_and(|fixture| fixture.live_effects.contains_key(&at.1))
            {
                continue;
            }
            let lands_at = wall_ms.saturating_add(view.home_fade_ms as u64);
            let Some(to) = self.value_at(view, &at, lands_at) else { continue };
            if to == from {
                continue;
            }
            let (fixture_id, key) = at;
            self.fades.retain(|f| !(f.fixture_id == fixture_id && f.key == key));
            let easing = view.fade_curves.for_key(&key);
            self.fades.push(Fade {
                fixture_id,
                key,
                running: RunningFade {
                    from,
                    to,
                    t0: wall_ms,
                    duration_ms: view.home_fade_ms,
                    easing,
                    cue_id: Uuid::nil(),
                },
            });
        }
    }

    /// Drive one parameter to a value outright, with nothing to fade from.
    ///
    /// A flow action setting a parameter, which is the one writer besides cues and the
    /// programmer. It parks the value as a landed fade, so it survives exactly as long
    /// as it used to survive in a map — until a cue, a release or another action takes
    /// the key. Last writer wins, which is a design question and not a bug to fix in
    /// passing.
    pub fn set_parameter(&mut self, fixture_id: Uuid, key: String, value: ParameterValue, at: u64) {
        self.fades.retain(|f| !(f.fixture_id == fixture_id && f.key == key));
        self.effects.remove(&(fixture_id, key.clone()));
        self.fades.push(parked(fixture_id, key, value, at));
    }

    /// Spot sequences that moved to a different cue, and start that cue's fades.
    fn track_cue_changes(
        &mut self,
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
                //
                // An effect *has* nowhere to arrive, so what it was showing when it
                // stopped is parked: stopping a chase freezes the look, which is what
                // it did when the value was simply left sitting in a map.
                self.park_stopped_effects(view, wall_ms, |fx| {
                    fx.source == EffectSource::Cue(cue_id)
                });
            }
            self.follows.remove(&sequence.id);

            let Some(cue_id) = next else {
                // No cue active means the sequence was taken off — the only way to
                // reach it, now that Go at the last cue stays there. So everything it
                // was driving and nothing else is still driving goes home.
                self.playing.remove(&sequence.id);
                let leaving = previous.and_then(|id| view.cues.get(&id).copied());
                self.release_sequence(wall_ms, sequence, view, leaving);
                continue;
            };
            self.playing.insert(sequence.id, cue_id);
            effects.push(PlaybackEffect::SetCueActive { cue_id, is_active: true });

            if let (Some(cue), Some(index)) = (view.cues.get(&cue_id), sequence.active_cue_index) {
                let anchor = view.anchor_for(sequence, wall_ms);
                let leaving = previous.and_then(|id| view.cues.get(&id).copied());
                self.take_cue(wall_ms, anchor, index, cue, view, sequence, leaving);
            }
        }
    }

    /// Take a cue: the stack up to and including it, which is what a cue *is* in a
    /// tracking stack.
    ///
    /// A cue that names four channels is not a look of four channels. It is the look
    /// of every cue before it with those four changed, and the console that takes it
    /// has to arrive at that look whichever cue it was on before. Going forward one
    /// step, that is nothing new — everything an earlier cue set is still where it
    /// left it. Jumping forward, the cues in between are applied on the way. And
    /// jumping *back* is the case that used to be wrong: a parameter that a later cue
    /// brought in was left where that cue put it, so going back from cue 5 to cue 1
    /// in the Theater demo left the side booms on, because nothing before cue 4
    /// mentions them. Now it goes home, since no cue at or before cue 1 has it.
    ///
    /// Three kinds of key, worked out from the stack rather than remembered:
    ///
    /// - **This cue's own captures** start as they always did, with the cue's times.
    /// - **A capture tracked in from an earlier cue** — the latest cue at or before
    ///   this one that has the key — is left alone if it is already what is driving
    ///   the parameter, and otherwise started with *this* cue's times, since this is
    ///   the cue being taken.
    /// - **A key only cues after this one capture** goes home over this cue's down
    ///   time, unless another sequence that is on could drive it.
    fn take_cue(
        &mut self,
        wall_ms: u64,
        anchor: u64,
        index: usize,
        cue: &Cue,
        view: &ShowView<'_>,
        sequence: &Sequence,
        leaving: Option<&Cue>,
    ) {
        // The latest capture of every key, over the stack up to here.
        let mut tracked: HashMap<Key, (&Cue, &pult_schema::types::cue::ParameterCapture)> =
            HashMap::new();
        for id in sequence.cue_ids.iter().take(index + 1) {
            let Some(earlier) = view.cues.get(id) else { continue };
            for capture in &earlier.captures {
                let key = (capture.fixture_id, parameter_key(&capture.parameter_kind));
                tracked.insert(key, (earlier, capture));
            }
        }

        // What only later cues capture goes home — unless something else that is on
        // could still want it, the same rule a release follows.
        let still_driven = view.captured_by_the_sequences_that_are_on(Some(sequence.id));
        for at in view.captured_by(sequence) {
            if tracked.contains_key(&at) {
                continue;
            }
            let driven = still_driven.contains(&at);
            // What a later cue set goes over the out time of the cue being left.
            let out = out_time(view, leaving, &at);
            self.let_go(wall_ms, at, view, out, sequence.id, driven);
        }

        let mut latest_end = wall_ms;
        for (at, (owner, capture)) in tracked {
            if owner.id != cue.id {
                let already = self
                    .fades
                    .iter()
                    .any(|f| f.fixture_id == at.0 && f.key == at.1 && f.running.cue_id == owner.id)
                    || self
                        .effects
                        .get(&at)
                        .is_some_and(|fx| fx.source == EffectSource::Cue(owner.id))
                    || self.beside.get(&at).is_some_and(|layers| {
                        layers.iter().any(|(_, layer)| owner_cue(layer) == Some(owner.id))
                    });
                if already {
                    continue;
                }
            }
            let ends = self.start_capture(wall_ms, anchor, cue, owner, capture, view);
            latest_end = latest_end.max(ends);
        }

        if let FollowMode::FollowAfter { delay_ms } = cue.follow_mode {
            // "After the previous cue completes, plus a delay": the fade has to land first.
            self.follows.insert(sequence.id, latest_end + delay_ms as u64);
        }
    }

    /// Send one parameter home over `duration_ms`, from wherever it is now.
    fn release_key(&mut self, wall_ms: u64, at: Key, view: &ShowView<'_>, duration_ms: u32) {
        let (fixture_id, key) = at.clone();
        // Where it is *before* its own fade and effect are taken away, so a
        // parameter half way through a cue's fade goes home from where it had got
        // to rather than from where that fade was aiming.
        let from = self.value_at(view, &at, wall_ms);

        // Its own fades and effects stop: they were a sequence asserting something,
        // and it has been told to stop asserting. A cue *changing* leaves a fade to
        // finish because it has somewhere to arrive; a release does not.
        self.fades.retain(|f| !(f.fixture_id == fixture_id && f.key == key));
        self.effects.remove(&at);

        let Some(to) = view.home_value(fixture_id, &key) else {
            return;
        };
        let from = from.unwrap_or_else(|| to.clone());
        if from == to {
            return;
        }

        // A release is a move like any other, and a head letting go of a mark deserves
        // the shape a head takes it with — so this is the show's own default rather
        // than the linear every release used to be. Nothing above it can say
        // otherwise: no cue and no capture is doing this.
        let easing = view.fade_curves.for_key(&key);
        self.fades.push(Fade {
            fixture_id,
            key,
            running: RunningFade {
                from,
                to,
                t0: wall_ms,
                // A zero duration is a fade that has already landed, which is what a
                // show that has not asked for a home time wants: releasing has always
                // snapped.
                duration_ms,
                easing,
                // No cue is doing this. A node told about the movement is told about a
                // movement, and the panel that asks "is this my cue's fade" gets no.
                cue_id: Uuid::nil(),
            },
        });
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
        wall_ms: u64,
        sequence: &Sequence,
        view: &ShowView<'_>,
        leaving: Option<&Cue>,
    ) {
        let mine = view.captured_by(sequence);
        if mine.is_empty() {
            return;
        }
        let still_driven = view.captured_by_the_sequences_that_are_on(Some(sequence.id));

        for at in mine {
            let driven = still_driven.contains(&at);
            // Off ends the cue that was up, so its out time is how its values go.
            let out = out_time(view, leaving, &at);
            self.let_go(wall_ms, at, view, out, sequence.id, driven);
        }
    }

    /// One sequence letting go of one parameter: taken off, or gone back past the cue
    /// that set it.
    ///
    /// What it was playing beside the winner goes. Where another sequence that is on
    /// holds the parameter, that is left alone. Otherwise it is handed back: to the
    /// latest of what is beside it, or — `driven`, another sequence that is on captures
    /// it — to that sequence's own capture, over that sequence's cue. Only a parameter
    /// nothing else wants goes home, over `duration_ms`.
    ///
    /// Handing back is new. A parameter used to stay wherever the sequence going off
    /// left it whenever another sequence *could* drive it, so a scare's last cue at
    /// black stayed black over the look it was meant to give the room back to.
    fn let_go(&mut self, wall_ms: u64, at: Key, view: &ShowView<'_>, duration_ms: u32, sequence: Uuid, driven: bool) {
        if let Some(layers) = self.beside.get_mut(&at) {
            layers.retain(|(owner, _)| *owner != sequence);
            if layers.is_empty() {
                self.beside.remove(&at);
            }
        }
        let primary = self.primary_owner(view, &at);
        if let Some(Some(owner)) = primary {
            if owner != sequence && view.is_on(owner) {
                return;
            }
        }
        if self.promote_beside(&at) {
            return;
        }
        if driven {
            // Nothing on the parameter at all is nothing to hand back.
            if primary.is_some() {
                self.reassert(wall_ms, &at, view, sequence);
            }
            return;
        }
        self.release_key(wall_ms, at, view, duration_ms);
    }

    /// Whose the winning fade or effect on a parameter is: `None` for nothing on it,
    /// `Some(None)` for something no sequence's cue put there — a release, a parked
    /// value, a flow.
    fn primary_owner(&self, view: &ShowView<'_>, at: &Key) -> Option<Option<Uuid>> {
        if let Some(effect) = self.effects.get(at) {
            return Some(match effect.source {
                EffectSource::Cue(cue) => view.sequence_of(cue),
                EffectSource::Programmer => None,
            });
        }
        let fade = self.fades.iter().find(|f| f.fixture_id == at.0 && f.key == at.1)?;
        Some(if fade.running.cue_id.is_nil() { None } else { view.sequence_of(fade.running.cue_id) })
    }

    /// The winner on a parameter, taken off it as a layer to keep beside.
    fn take_primary(&mut self, at: &Key) -> Option<HtpLayer> {
        let effect = self.effects.remove(at);
        let fade = self.fades.iter().position(|f| f.fixture_id == at.0 && f.key == at.1).map(|i| self.fades.remove(i).running);
        self.fades.retain(|f| !(f.fixture_id == at.0 && f.key == at.1));
        (effect.is_some() || fade.is_some()).then_some(HtpLayer { effect, fade })
    }

    /// The latest layer beside a parameter becomes its winner. Answers whether there was one.
    fn promote_beside(&mut self, at: &Key) -> bool {
        let Some(layers) = self.beside.get_mut(at) else { return false };
        let Some((_, layer)) = layers.pop() else { return false };
        if layers.is_empty() {
            self.beside.remove(at);
        }
        self.fades.retain(|f| !(f.fixture_id == at.0 && f.key == at.1));
        self.effects.remove(at);
        if let Some(effect) = layer.effect {
            self.effects.insert(at.clone(), effect);
        }
        if let Some(running) = layer.fade {
            self.fades.push(Fade { fixture_id: at.0, key: at.1.clone(), running });
        }
        true
    }

    /// Give a parameter back to the sequence that is on and went last of those
    /// capturing it, from where it is now, over that sequence's own cue.
    fn reassert(&mut self, wall_ms: u64, at: &Key, view: &ShowView<'_>, except: Uuid) {
        let owner = view
            .sequences
            .iter()
            .filter(|s| s.id != except && s.active_cue_index.is_some())
            .filter_map(|s| view.tracked_capture(s, at).map(|found| (s, found)))
            .max_by_key(|(s, _)| s.went_at.unwrap_or(0));
        let Some((sequence, (owner, capture))) = owner else { return };
        let Some(cue) = view.active_cue(sequence) else { return };
        self.start_capture(wall_ms, wall_ms, cue, owner, capture, view);
    }

    /// Freeze whatever the effects this predicate picks out were showing, and drop
    /// them.
    ///
    /// An effect has nowhere to arrive, so when it stops there is no value it was on
    /// its way to. Parking what it was showing at `wall_ms` is what keeps a stopped
    /// chase looking like a held look rather than snapping to a home value nothing
    /// asked for.
    fn park_stopped_effects(
        &mut self,
        view: &ShowView<'_>,
        wall_ms: u64,
        mut which: impl FnMut(&RunningEffect) -> bool,
    ) {
        let stopping: Vec<Key> = self
            .effects
            .iter()
            .filter(|(_, fx)| which(fx))
            .map(|(at, _)| at.clone())
            .collect();
        for at in stopping {
            let showing = self.value_at(view, &at, wall_ms);
            self.effects.remove(&at);
            let Some(value) = showing else { continue };
            self.fades.retain(|f| !(f.fixture_id == at.0 && f.key == at.1));
            self.fades.push(parked(at.0, at.1, value, wall_ms));
        }
        // And beside the winner, parked as what that sequence was showing — still its,
        // so its fader still scales it.
        for layers in self.beside.values_mut() {
            for (_, layer) in layers.iter_mut() {
                let Some(effect) = layer.effect.as_ref().filter(|fx| which(fx)) else { continue };
                let EffectSource::Cue(cue_id) = effect.source else { continue };
                let value = pult_render::effect_value_at(effect, wall_ms);
                let mut fade = parked(Uuid::nil(), String::new(), value, wall_ms).running;
                fade.cue_id = cue_id;
                *layer = HtpLayer { effect: None, fade: Some(fade) };
            }
        }
    }

    /// Begin one capture, and answer when its fade lands.
    ///
    /// `cue` is the cue being taken and supplies the default times; `owner` is the
    /// cue the capture belongs to, whose id goes on the fade or the effect — the two
    /// are the same cue for its own captures and differ for one tracked in from
    /// earlier in the stack.
    ///
    /// The anchor is the sequence's `went_at`, not this station's idea of now. Two
    /// consoles process the same Go milliseconds apart, and a fade started from each
    /// one's own arrival would leave them visibly out of step for the length of the
    /// fade; an effect anchored that way would stay out of step for good.
    fn start_capture(
        &mut self,
        wall_ms: u64,
        anchor: u64,
        cue: &Cue,
        owner: &Cue,
        capture: &pult_schema::types::cue::ParameterCapture,
        view: &ShowView<'_>,
    ) -> u64 {
        {
            let key = parameter_key(&capture.parameter_kind);
            let at = (capture.fixture_id, key.clone());
            // What this capture actually asserts: the preset it names where that
            // resolves, and the literal it was stored as otherwise. One resolution,
            // in the schema, so the pass, the paperwork RPC and a Go cannot disagree
            // about a cue pointed at a palette somebody deleted.
            let to = capture.value_in(&view.presets).clone();

            // Where the parameter is *now*, before this capture takes the key off
            // whatever had it. A cue re-taken mid-fade fades on from here.
            let showing = self.value_at(view, &at, wall_ms);

            // An HTP sequence going over an intensity another sequence that is on is
            // playing keeps that one beside it; an LTP one takes the key from all of
            // them. Either way this sequence's own earlier layer goes, being replaced.
            if let Some(mine) = view.sequence_of(owner.id) {
                if let Some(layers) = self.beside.get_mut(&at) {
                    layers.retain(|(owner, _)| *owner != mine);
                }
                let htp = view.sequence(mine).is_some_and(|s| s.merge() == Merge::Htp)
                    && FadeGroup::of_key(&key) == FadeGroup::Intensity;
                if htp {
                    if let Some(Some(other)) = self.primary_owner(view, &at) {
                        if other != mine && view.is_on(other) {
                            if let Some(layer) = self.take_primary(&at) {
                                self.beside.entry(at.clone()).or_default().push((other, layer));
                            }
                        }
                    }
                } else {
                    self.beside.remove(&at);
                }
                if self.beside.get(&at).is_some_and(Vec::is_empty) {
                    self.beside.remove(&at);
                }
            }

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
                        EffectSource::Cue(owner.id),
                    ),
                );
                return wall_ms;
            }

            // Fade from wherever the parameter is now, so re-cueing mid-fade is
            // smooth — and from where it rests when nothing has ever driven it, which
            // is the fixture's answer rather than a zero of the right shape.
            //
            // Read before the key is cleared above? No: `from` is taken from `showing`,
            // which was evaluated for this key before either was removed.
            //
            // And from what is on stage *before this sequence's fader scales it*: the
            // fade below runs at that fader's level, so starting it from the showing
            // value would dim the start of every Go a sequence below full makes.
            let from = showing
                .map(|showing| unmastered(view, &key, owner.id, showing))
                // A fixture whose type has gone: nothing can say where it rests, so
                // the cue lands rather than fading from a zero nobody vouched for.
                .unwrap_or_else(|| to.clone());

            // A capture's own time wins; zero means "use the cue's". The cue's *in*
            // time, whichever way the parameter is going: a cue's in time is how it
            // starts, and its out time is how what it set goes when it ends — see
            // `out_time`. (It used to split by direction, ETC's downtime, which read
            // as a bug: back into a cue with an out time, every light caught higher
            // than the cue faded while every one caught lower snapped.)
            let duration_ms = if capture.fade_in_ms > 0 { capture.fade_in_ms } else { cue.fade_in_ms };

            let fade = Fade {
                fixture_id: capture.fixture_id,
                key: key.clone(),
                running: RunningFade {
                    from,
                    to,
                    t0: anchor + capture.delay_in_ms as u64,
                    duration_ms,
                    // The same three steps the times above take, in the same order
                    // and from the same cue: this capture's own curve, then the cue
                    // being taken, then what the show says parameters of this sort
                    // do. Resolved in the schema so that the cue editor showing an
                    // operator what a cue will do reads the same answer.
                    easing: view.fade_curves.resolve(capture.easing, cue.easing, &key),
                    cue_id: owner.id,
                },
            };
            let ends = fade.ends_at();
            self.fades.push(fade);
            ends
        }
    }

    /// Publish what is driving every parameter: the shapes, and the fades.
    ///
    /// Only the winner of playback's own two layers is listed, so a fade under an
    /// effect is left out — it is still recorded here, but it is not what anybody is
    /// seeing, and a node handed both would have to decide between them.
    ///
    /// A parameter the programmer holds a plain *value* on is listed as nothing at
    /// all, and that absence is load-bearing: it is what tells a node to stop tracing
    /// a shape it was handed and take values again. So these two fields go on meaning
    /// "what is reaching the light", and a consumer that also reads the programmer
    /// gets the same answer either way.
    ///
    /// A programmer *effect* is listed, because resolving it against a speed master is
    /// work only a station can do.
    fn emit_motion(&mut self, view: &ShowView<'_>, out: &mut Vec<PlaybackEffect>) {
        let mut per_fixture: HashMap<Uuid, (HashMap<String, RunningEffect>, HashMap<String, RunningFade>)> =
            HashMap::new();
        let held = programmer::held_keys(view);

        for (key, effect) in &self.effects {
            if held.contains(key) {
                continue;
            }
            per_fixture.entry(key.0).or_default().0.insert(key.1.clone(), effect.clone());
        }
        // The programmer writes last here, so its shape covers the cue's.
        for (key, effect) in programmer::held_effects(view) {
            per_fixture.entry(key.0).or_default().0.insert(key.1, effect);
        }

        for fade in &self.fades {
            let at = (fade.fixture_id, fade.key.clone());
            if held.contains(&at)
                || per_fixture.get(&fade.fixture_id).is_some_and(|(fx, _)| fx.contains_key(&fade.key))
            {
                continue;
            }
            per_fixture
                .entry(fade.fixture_id)
                .or_default()
                .1
                .insert(fade.key.clone(), fade.running.clone());
        }

        // Every fixture that had motion last pass has to be considered too, or one
        // that has just stopped would keep its last description for ever.
        let considered: Vec<Uuid> = view
            .fixtures
            .iter()
            .map(|f| f.id)
            .filter(|id| {
                per_fixture.contains_key(id) || self.motion.contains_key(id) || self.beside.keys().any(|at| at.0 == *id)
            })
            .collect();

        for fixture_id in considered {
            let (mut effects, mut fades) = per_fixture.remove(&fixture_id).unwrap_or_default();

            // Keep what the fixture already carries on keys playback has no opinion
            // about. The map is written whole, so publishing only what this object
            // knows would silently take a parameter's driver away because some *other*
            // parameter of the same fixture moved.
            //
            // Never the programmer's own shape, though: that lives exactly as long as
            // its entry, and `held_effects` above already lists every one still held.
            // Carried forward, an effect cleared out of the programmer kept running for
            // ever — Clear took the entry and the light went on chasing, until a plain
            // value on the same key unlisted it (found at the gig, 2026-10-26).
            if let Some(fixture) = view.fixture(fixture_id) {
                for (key, effect) in &fixture.live_effects {
                    let mine = (fixture_id, key.clone());
                    if !held.contains(&mine)
                        && !matches!(effect.source, EffectSource::Programmer)
                        && !self.effects.contains_key(&mine)
                        && !self.holds_a_fade(&mine)
                    {
                        effects.entry(key.clone()).or_insert_with(|| effect.clone());
                    }
                }
                for (key, fade) in &fixture.live_fades {
                    let mine = (fixture_id, key.clone());
                    if !held.contains(&mine)
                        && !self.effects.contains_key(&mine)
                        && !self.holds_a_fade(&mine)
                        && !effects.contains_key(key)
                    {
                        fades.entry(key.clone()).or_insert_with(|| fade.clone());
                    }
                }
            }

            // What is beside the winner, only where the winner is playback's own: the
            // programmer holding the key is all that reaches the light.
            let beside: HashMap<String, Vec<HtpLayer>> = self
                .beside
                .iter()
                .filter(|(at, layers)| at.0 == fixture_id && !layers.is_empty() && !held.contains(*at))
                .map(|(at, layers)| (at.1.clone(), layers.iter().map(|(_, layer)| layer.clone()).collect()))
                .collect();

            let previous = self.motion.get(&fixture_id);

            if previous.map(|(e, f, _)| (e, f)) != Some((&effects, &fades)) {
                out.push(PlaybackEffect::SetLiveEffects {
                    fixture_id,
                    effects: effects.clone(),
                });
                out.push(PlaybackEffect::SetLiveFades { fixture_id, fades: fades.clone() });
            }
            let empty = HashMap::new();
            if previous.map_or(&empty, |(_, _, b)| b) != &beside {
                out.push(PlaybackEffect::SetLiveHtp { fixture_id, layers: beside.clone() });
            }

            if effects.is_empty() && fades.is_empty() && beside.is_empty() {
                self.motion.remove(&fixture_id);
            } else {
                self.motion.insert(fixture_id, (effects, fades, beside));
            }
        }

        // A fixture that has left the rig takes its record with it — and so do the
        // fades and effects that were driving it, which nothing can publish any more.
        self.motion.retain(|id, _| view.by_id.contains_key(id));
        self.fades.retain(|f| view.by_id.contains_key(&f.fixture_id));
        self.effects.retain(|at, _| view.by_id.contains_key(&at.0));
        self.beside.retain(|at, _| view.by_id.contains_key(&at.0));
    }

    /// Does playback itself have a fade on this parameter?
    fn holds_a_fade(&self, at: &Key) -> bool {
        self.fades.iter().any(|f| f.fixture_id == at.0 && f.key == at.1)
    }

    fn fire_due_follows(&mut self, wall_ms: u64, effects: &mut Vec<PlaybackEffect>) {
        let due: Vec<Uuid> = self
            .follows
            .iter()
            .filter(|(_, at)| **at <= wall_ms)
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

// ── Parameter helpers ─────────────────────────────────────────────────────────

/// What a fade this cue starts has to begin from, for the stage not to jump.
///
/// The fade will run at its sequence's fader, so it starts from what is showing
/// with that fader taken back out — `from × master` is then exactly where the light
/// was. Only for the keys a master reaches; everything else starts where it is.
/// The cue an HTP layer belongs to.
fn owner_cue(layer: &HtpLayer) -> Option<Uuid> {
    match (&layer.effect, &layer.fade) {
        (Some(effect), _) => match effect.source {
            EffectSource::Cue(cue) => Some(cue),
            EffectSource::Programmer => None,
        },
        (None, Some(fade)) => (!fade.cue_id.is_nil()).then_some(fade.cue_id),
        (None, None) => None,
    }
}

fn unmastered(view: &ShowView<'_>, key: &str, cue_id: Uuid, showing: ParameterValue) -> ParameterValue {
    if FadeGroup::of_key(key) != FadeGroup::Intensity {
        return showing;
    }
    match view.masters.of_cue(cue_id) {
        Some(level) => showing.unscaled(level),
        None => showing,
    }
}

/// How long what a cue set takes to go when that cue ends — the sequence going
/// back past it, or off: the cue's own capture for the key where it says an out
/// time, then the cue's out time, then the show's *letting go* time.
fn out_time(view: &ShowView<'_>, leaving: Option<&Cue>, at: &Key) -> u32 {
    let Some(cue) = leaving else { return view.home_fade_ms };
    let own = cue
        .captures
        .iter()
        .find(|c| c.fixture_id == at.0 && parameter_key(&c.parameter_kind) == at.1)
        .map_or(0, |c| c.fade_out_ms);
    [own, cue.fade_out_ms].into_iter().find(|ms| *ms > 0).unwrap_or(view.home_fade_ms)
}

#[cfg(test)]
mod tests;
