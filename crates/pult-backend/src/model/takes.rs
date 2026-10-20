//! A take recorded from the programmer, and how a pass joins the takes already there.
//!
//! Pure, like the rest of `model/`: what the programmer held at a position goes in, and
//! change points come out. The station's half — which station records, where the bytes
//! go, what is released afterwards — is `infra/connectors/input.rs`, beside the take an
//! input records, because both end in the same three acts: encode, store, append.
//!
//! **Touch-scoped.** A pass records the keys that *change* while it runs, from the
//! moment each is first touched, and nothing else. A key the programmer was already
//! holding when the pass began and that nobody moves is not in it. That is what lets
//! a second pass record the head without re-recording the jaw it is being played over.
//!
//! **A pass joins the track that is playing the key.** The evaluator takes one track
//! per key — the latest in the list — and a track that has started never says nothing
//! again. So a second track carrying a key the first already has would not *layer*
//! over it: it would hide it from its own first point onwards and hold its last value
//! for the rest of the song. A pass therefore splices each key it touched into the
//! track already asserting that key, between the first touch and the moment the pass
//! ended, and the old take resumes from there. A key no track has goes into a new one.
//! That is punch-in on touch with latch to the end of the pass, which is how a
//! skeleton is built: the jaw, then the head, then a better second half of the jaw.

use std::collections::{BTreeMap, HashMap};

use pult_render::{sample, Track, TrackKey, TrackPoint};
use pult_schema::types::{fixture::ParameterValue, parameter_key, programmer::ProgrammerValue};
use uuid::Uuid;

/// How far the playhead may move by itself before a pass counts it as a jump.
///
/// **Forwards, a second**: `pult_schema::clock`'s step threshold, and the largest
/// correction the audio chase makes without locating. Below it the anchor moved because
/// the loudspeakers said so, and a pass that split there would cut a take in two every
/// time the sound card corrected the show.
///
/// **Backwards, twenty milliseconds**, the threshold under which the audio station
/// corrects nothing at all. A playhead going back further than that is a loop wrapping
/// or somebody locating — and a loop can be shorter than a second, which is what a
/// symmetric threshold missed. The cost of reading a backward correction as a jump is
/// a pass stored early and the next one spliced into it, which plays the same.
pub const JUMP_FORWARD_MS: u64 = pult_schema::clock::STEP_ABOVE_MS as u64;
pub const JUMP_BACK_MS: u64 = pult_schema::clock::APPLY_BELOW_MS as u64;

/// Whether the playhead jumped between two readings of it at one moment.
pub fn jumped(was_ms: u64, is_ms: u64) -> bool {
    is_ms + JUMP_BACK_MS < was_ms || is_ms > was_ms + JUMP_FORWARD_MS
}

/// One pass of the programmer, in progress.
#[derive(Debug, Default)]
pub struct Pass {
    /// What the programmer held at the last look, per key. Primed by the first look,
    /// which records nothing: what was already held when the pass began is the
    /// baseline, not a move.
    seen: Option<HashMap<(Uuid, String), ParameterValue>>,
    /// The change points so far, from each key's first touch.
    points: BTreeMap<(Uuid, String), Vec<TrackPoint>>,
}

impl Pass {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next pass after a jump, carrying on from what this one last saw.
    ///
    /// A loop wrapping is a jump, and the operator's hand is still on the fader: the
    /// next lap's baseline is what the programmer holds now, not a fresh first look
    /// that would swallow the next move as "already held".
    pub fn continuing(&self) -> Self {
        Pass { seen: self.seen.clone(), points: BTreeMap::new() }
    }

    /// Look at the programmer as it is, at a position on the timeline.
    pub fn observe(&mut self, position_ms: u64, programmer: &[ProgrammerValue]) {
        let now: HashMap<(Uuid, String), ParameterValue> = programmer
            .iter()
            // An effect held in the programmer is a shape, not a value, and a shape
            // has no change points. Recording it would be recording its parameters.
            .filter(|entry| entry.effect.is_none())
            .map(|entry| {
                ((entry.fixture_id, parameter_key(&entry.parameter_kind)), entry.value.clone())
            })
            .collect();
        let Some(seen) = self.seen.replace(now.clone()) else { return };
        let ms = position_ms.min(u32::MAX as u64) as u32;
        for (at, value) in now {
            if seen.get(&at) == Some(&value) {
                continue;
            }
            let points = self.points.entry(at).or_default();
            // A playhead the sound card nudged backwards by a few milliseconds is not
            // a jump (see [`jumped`]), but a track's points have to ascend. Holding the
            // later position costs those few milliseconds and nothing else.
            let ms = points.last().map_or(ms, |last| ms.max(last.ms));
            match points.last_mut() {
                Some(last) if last.ms == ms => last.value = value,
                _ => points.push(TrackPoint { ms, value }),
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// How many keys the pass has touched so far.
    pub fn keys(&self) -> usize {
        self.points.len()
    }

    /// How many change points it holds.
    pub fn len(&self) -> usize {
        self.points.values().map(Vec::len).sum()
    }

    /// The pass as a track, in a stable key order.
    pub fn into_track(self) -> Track {
        Track {
            keys: self
                .points
                .into_iter()
                .map(|((fixture_id, key), points)| TrackKey { fixture_id, key, points })
                .collect(),
        }
    }
}

/// What storing a pass comes to.
#[derive(Debug, Default, PartialEq)]
pub struct Merged {
    /// Existing tracks rewritten, by track id, with the pass spliced in.
    pub replaced: Vec<(Uuid, Track)>,
    /// The keys no playing track had, as a new track.
    pub fresh: Option<Track>,
}

/// Join a pass to the takes already on the timeline.
///
/// `existing` is the timeline's **enabled** tracks in list order — the ones that play,
/// in the order the evaluator ranks them, so the track a key is spliced into is the
/// one that was asserting it. `punch_out_ms` is where the pass ended.
pub fn merge(pass: Track, punch_out_ms: u64, existing: &[(Uuid, Track)]) -> Merged {
    let mut replaced: Vec<(Uuid, Track)> = Vec::new();
    let mut fresh: Vec<TrackKey> = Vec::new();
    let out = punch_out_ms.min(u32::MAX as u64) as u32;

    for key in pass.keys {
        if key.points.is_empty() {
            continue;
        }
        let owner = existing.iter().rev().find(|(_, track)| {
            track.keys.iter().any(|k| k.fixture_id == key.fixture_id && k.key == key.key)
        });
        let Some((owner_id, owner)) = owner else {
            fresh.push(key);
            continue;
        };
        // Splice into the copy already being rewritten if an earlier key of this pass
        // went into the same track.
        let at = match replaced.iter().position(|(id, _)| id == owner_id) {
            Some(at) => at,
            None => {
                replaced.push((*owner_id, owner.clone()));
                replaced.len() - 1
            }
        };
        let target = &mut replaced[at].1;
        if let Some(old) =
            target.keys.iter_mut().find(|k| k.fixture_id == key.fixture_id && k.key == key.key)
        {
            old.points = splice(&old.points, &key.points, out);
        }
    }

    Merged {
        replaced,
        fresh: (!fresh.is_empty()).then_some(Track { keys: fresh }),
    }
}

/// One key's old points with a pass written over them.
///
/// The old take up to the first touch, the pass, and then the old take again from the
/// punch-out — resuming at the value it had there, so it carries on exactly as though
/// the pass had never been. The resume is put a millisecond after the pass's last
/// point if the pass ran right up to the punch-out, because two points at one position
/// are one point and the pass's would be lost.
fn splice(old: &[TrackPoint], pass: &[TrackPoint], out: u32) -> Vec<TrackPoint> {
    let Some(first) = pass.first() else { return old.to_vec() };
    let last = pass.last().map_or(first.ms, |p| p.ms);
    let mut points: Vec<TrackPoint> = old.iter().filter(|p| p.ms < first.ms).cloned().collect();
    points.extend(pass.iter().cloned());
    let resume_at = out.max(last.saturating_add(1));
    if let Some(value) = sample(old, resume_at as u64) {
        points.push(TrackPoint { ms: resume_at, value: value.clone() });
    }
    points.extend(old.iter().filter(|p| p.ms > resume_at).cloned());
    // A value that did not move is not a change, which is what a track is made of.
    points.dedup_by(|later, earlier| later.value == earlier.value);
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::{
        effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
        fixture::ParameterKind,
    };

    fn jaw() -> Uuid {
        Uuid::from_u128(1)
    }

    fn held(value: f32) -> ProgrammerValue {
        held_kind(ParameterKind::Intensity, value)
    }

    fn held_kind(kind: ParameterKind, value: f32) -> ProgrammerValue {
        ProgrammerValue {
            id: Uuid::new_v4(),
            fixture_id: jaw(),
            parameter_kind: kind,
            value: ParameterValue::Float(value),
            effect: None,
            preset: None,
            locked: false,
        }
    }

    fn points(pairs: &[(u32, f32)]) -> Vec<TrackPoint> {
        pairs.iter().map(|(ms, v)| TrackPoint { ms: *ms, value: ParameterValue::Float(*v) }).collect()
    }

    fn pairs(points: &[TrackPoint]) -> Vec<(u32, f32)> {
        points
            .iter()
            .map(|p| match p.value {
                ParameterValue::Float(v) => (p.ms, v),
                _ => panic!("a float"),
            })
            .collect()
    }

    fn a_track(key: &str, pairs: &[(u32, f32)]) -> Track {
        Track {
            keys: vec![TrackKey { fixture_id: jaw(), key: key.into(), points: points(pairs) }],
        }
    }

    #[test]
    fn what_was_held_when_the_pass_began_is_not_recorded() {
        let mut pass = Pass::new();
        pass.observe(0, &[held(0.5)]);
        pass.observe(100, &[held(0.5)]);
        assert!(pass.is_empty(), "a held value nobody moved is the baseline");
        pass.observe(200, &[held(0.7)]);
        pass.observe(300, &[held(0.7)]);
        pass.observe(400, &[held(0.2)]);
        let track = pass.into_track();
        assert_eq!(pairs(&track.keys[0].points), vec![(200, 0.7), (400, 0.2)]);
    }

    #[test]
    fn a_key_first_held_during_the_pass_is_a_touch() {
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        pass.observe(250, &[held(1.0)]);
        assert_eq!(pass.keys(), 1);
        assert_eq!(pass.len(), 1);
    }

    #[test]
    fn a_held_effect_is_not_recorded() {
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        let mut shape = held(0.3);
        shape.effect = Some(EffectSpec {
            effect_id: Uuid::nil(),
            curve: Curve::Shape(Shape::Sine),
            rate: Rate::Hz(1.0),
            low: ParameterValue::Float(0.0),
            high: ParameterValue::Float(1.0),
            width: 0.5,
            direction: Direction::Forward,
            phase: 0.0,
            spread: Spread::Even,
            t0: None,
            template: None,
        });
        pass.observe(100, &[shape]);
        assert!(pass.is_empty());
    }

    #[test]
    fn a_playhead_nudged_backwards_still_ascends() {
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        pass.observe(500, &[held(0.1)]);
        pass.observe(490, &[held(0.2)]);
        assert_eq!(pairs(&pass.into_track().keys[0].points), vec![(500, 0.2)]);
    }

    #[test]
    fn the_next_lap_carries_on_from_what_the_last_one_saw() {
        let mut pass = Pass::new();
        pass.observe(0, &[held(0.1)]);
        pass.observe(500, &[held(0.4)]);
        let mut lap = pass.continuing();
        lap.observe(100, &[held(0.4)]);
        assert!(lap.is_empty(), "the hand did not move across the wrap");
        lap.observe(200, &[held(0.9)]);
        assert_eq!(pairs(&lap.into_track().keys[0].points), vec![(200, 0.9)]);
    }

    #[test]
    fn a_loop_shorter_than_a_second_is_a_jump_and_a_correction_is_not() {
        assert!(jumped(810, 10), "a 0.8 s loop wrapping");
        assert!(!jumped(810, 795), "the sound card pulling the show back 15 ms");
        assert!(!jumped(810, 1_500), "a forward correction under a second");
        assert!(jumped(810, 5_000), "a locate forwards");
    }

    #[test]
    fn a_key_no_track_has_is_a_new_track() {
        let merged = merge(a_track("Intensity", &[(0, 1.0)]), 1_000, &[]);
        assert!(merged.replaced.is_empty());
        assert_eq!(merged.fresh.unwrap().keys.len(), 1);
    }

    /// The jaw, then the head: the second pass touches only the head, and the jaw's
    /// take is not rewritten at all.
    #[test]
    fn a_pass_leaves_the_keys_it_did_not_touch_alone() {
        let jaw_take = (Uuid::from_u128(10), a_track("Intensity", &[(0, 0.0), (500, 1.0)]));
        let merged = merge(a_track("Pan", &[(100, 0.5)]), 2_000, &[jaw_take]);
        assert!(merged.replaced.is_empty(), "the jaw's take is untouched");
        assert_eq!(merged.fresh.unwrap().keys[0].key, "Pan");
    }

    /// A better second half of the jaw: the old take up to the first touch, the pass,
    /// and the old take again from where the pass stopped.
    #[test]
    fn a_pass_is_spliced_into_the_take_playing_the_key() {
        let take = Uuid::from_u128(10);
        let old = a_track("Intensity", &[(0, 0.0), (1_000, 0.2), (2_000, 0.4), (3_000, 0.6), (4_000, 0.8)]);
        let merged = merge(a_track("Intensity", &[(1_500, 0.9), (2_500, 0.1)]), 3_500, &[(take, old)]);
        assert!(merged.fresh.is_none());
        let (id, track) = &merged.replaced[0];
        assert_eq!(*id, take);
        assert_eq!(
            pairs(&track.keys[0].points),
            vec![(0, 0.0), (1_000, 0.2), (1_500, 0.9), (2_500, 0.1), (3_500, 0.6), (4_000, 0.8)],
            "the old take resumes at what it held at the punch-out"
        );
    }

    #[test]
    fn the_later_of_two_takes_holding_a_key_is_the_one_rewritten() {
        let first = (Uuid::from_u128(1), a_track("Intensity", &[(0, 0.1)]));
        let second = (Uuid::from_u128(2), a_track("Intensity", &[(0, 0.2)]));
        let merged = merge(a_track("Intensity", &[(100, 0.9)]), 200, &[first, second]);
        assert_eq!(merged.replaced[0].0, Uuid::from_u128(2), "the one the evaluator plays");
    }

    #[test]
    fn a_pass_to_the_punch_out_resumes_a_millisecond_later() {
        let old = (Uuid::from_u128(1), a_track("Intensity", &[(0, 0.3)]));
        let merged = merge(a_track("Intensity", &[(100, 0.9), (200, 0.5)]), 200, &[old]);
        assert_eq!(
            pairs(&merged.replaced[0].1.keys[0].points),
            vec![(0, 0.3), (100, 0.9), (200, 0.5), (201, 0.3)]
        );
    }
}
