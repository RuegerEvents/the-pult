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
//! **A pass replaces what it recorded over, completely.** Each key it touched gets a
//! row of its own (`TimelineTrack`, one per parameter) whose window is the whole pass —
//! from where it started to where it stopped — and every row already playing that key
//! is cut out of that stretch (`types::timeline::cut_rows`). Before the first touch the
//! key holds what the programmer held when the pass began, or says nothing; a key let
//! go during the pass says nothing from there. So what the operator saw while it rolled
//! is what is stored, and a stretch is removed by recording over it with the key let
//! go. Keys nobody touched are left as they were: the jaw, then the head, then the
//! jaw again from the middle.
//!
//! The recording itself is one asset per pass and never changes; the rows point into
//! it. What plays is the rows composed — `pult_render::compose`.

use std::collections::{BTreeMap, HashMap};

use pult_render::{Track, TrackKey, TrackPoint};
use pult_schema::types::{
    fixture::ParameterValue,
    parameter_key,
    programmer::ProgrammerValue,
    sequence::LEVEL_KEY,
    timeline::{cut_rows, split_row, TimelineTrack},
};
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
    /// What the programmer held when the pass began: where a touched key starts from.
    baseline: HashMap<(Uuid, String), ParameterValue>,
    /// The change points so far, from each key's first touch.
    points: BTreeMap<(Uuid, String), Vec<TrackPoint>>,
    /// Where on the timeline the pass began, which is where what it replaces begins.
    from_ms: u32,
}

impl Pass {
    pub fn new() -> Self {
        Self::default()
    }

    /// A pass beginning at a position on the timeline.
    pub fn starting_at(position_ms: u64) -> Self {
        Pass { from_ms: position_ms.min(u32::MAX as u64) as u32, ..Self::default() }
    }

    /// The next pass after a jump, from the position the playhead jumped to, carrying
    /// on from what this one last saw.
    ///
    /// A loop wrapping is a jump, and the operator's hand is still on the fader: the
    /// next lap's baseline is what the programmer holds now, not a fresh first look
    /// that would swallow the next move as "already held".
    pub fn continuing(&self, position_ms: u64) -> Self {
        Pass {
            seen: self.seen.clone(),
            baseline: self.seen.clone().unwrap_or_default(),
            points: BTreeMap::new(),
            from_ms: position_ms.min(u32::MAX as u64) as u32,
        }
    }

    /// Where the pass began.
    pub fn from_ms(&self) -> u32 {
        self.from_ms
    }

    /// Look at the programmer as it is, at a position on the timeline.
    pub fn observe(&mut self, position_ms: u64, programmer: &[ProgrammerValue]) {
        self.observe_with_levels(position_ms, programmer, &[]);
    }

    /// The programmer and the sequences' levels as they are, at a position.
    ///
    /// A level is kept under [`LEVEL_KEY`] with the sequence's id where a fixture's
    /// would be, so a fader moved during a take is recorded from the first touch the
    /// way a programmer fader is — and one nobody touched, which says the same thing
    /// every time it is looked at, writes nothing.
    pub fn observe_with_levels(
        &mut self,
        position_ms: u64,
        programmer: &[ProgrammerValue],
        levels: &[(Uuid, f32)],
    ) {
        let now: HashMap<(Uuid, String), ParameterValue> = programmer
            .iter()
            // An effect held in the programmer is a shape, not a value, and a shape
            // has no change points. Recording it would be recording its parameters.
            .filter(|entry| entry.effect.is_none())
            .map(|entry| {
                ((entry.fixture_id, parameter_key(&entry.parameter_kind)), entry.value.clone())
            })
            .chain(
                levels
                    .iter()
                    .map(|(sequence, level)| ((*sequence, LEVEL_KEY.to_string()), ParameterValue::Float(*level))),
            )
            .collect();
        let Some(seen) = self.seen.replace(now.clone()) else {
            self.baseline = now;
            return;
        };
        let ms = position_ms.min(u32::MAX as u64) as u32;
        // A key that left the programmer is let go here: a flash key released, a key
        // cleared. Recorded as a release, so the take says nothing from this point and
        // what is under it shows through — not the last value held for the rest of the
        // song, which is what a button press came to before (found at the gig).
        for at in seen.keys().filter(|at| !now.contains_key(*at)) {
            let points = self.points.entry(at.clone()).or_default();
            let ms = points.last().map_or(ms, |last| ms.max(last.ms));
            match points.last_mut() {
                Some(last) if last.ms == ms => *last = TrackPoint::released(ms),
                Some(last) if last.release => {}
                _ => points.push(TrackPoint::released(ms)),
            }
        }
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
                Some(last) if last.ms == ms => *last = TrackPoint { release: false, ms, value },
                _ => points.push(TrackPoint { release: false, ms, value }),
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

    /// The pass as a track, in a stable key order. A touched key starts from what it
    /// held when the pass began, so the stretch before the first touch is what the
    /// operator saw there rather than whatever an older take said.
    pub fn into_track(self) -> Track {
        let (from, baseline) = (self.from_ms, self.baseline);
        Track {
            keys: self
                .points
                .into_iter()
                .map(|((fixture_id, key), mut points)| {
                    if let Some(value) = baseline.get(&(fixture_id, key.clone())) {
                        if points.first().is_some_and(|p| p.ms > from) {
                            points.insert(0, TrackPoint { release: false, ms: from, value: value.clone() });
                        }
                    }
                    TrackKey { fixture_id, key, points }
                })
                // A key that was only ever let go, from nothing, said nothing.
                .filter(|key| key.points.iter().any(|p| !p.release))
                .collect(),
        }
    }
}

/// Store a recording into a timeline's rows: every row already playing one of its
/// keys is cut out of `[from_ms, out_ms)`, and each key gets a row of its own over
/// exactly that stretch. `asset` is the recording's sha; `input_id` where it came off
/// a wire.
pub fn store(
    rows: &mut Vec<TimelineTrack>,
    recording: &Track,
    asset: &str,
    input_id: Option<Uuid>,
    from_ms: u32,
    out_ms: u32,
) {
    // A pass that ended where it began still replaced the moment it was at.
    let out = out_ms.max(from_ms.saturating_add(1));
    for key in &recording.keys {
        cut_rows(rows, key.fixture_id, &key.key, from_ms, out);
        rows.push(TimelineTrack {
            id: Uuid::new_v4(),
            name: String::new(),
            asset: asset.to_string(),
            input_id,
            offset_ms: 0,
            enabled: true,
            fixture_id: Some(key.fixture_id),
            key: Some(key.key.clone()),
            from_ms: Some(from_ms),
            to_ms: Some(out),
        });
    }
}

/// Every row that plays all of its recording's parameters — a take stored before rows
/// were per parameter — as a row per parameter. `keys_of` is what each such recording
/// holds, by sha; a row whose recording is not known yet is left as it is. Answers
/// whether anything changed.
pub fn per_parameter(rows: &mut Vec<TimelineTrack>, keys_of: &HashMap<String, Vec<(Uuid, String)>>) -> bool {
    let mut changed = false;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        match (row.key.is_none(), keys_of.get(&row.asset)) {
            (true, Some(keys)) => {
                out.extend(split_row(&row, keys));
                changed = true;
            }
            _ => out.push(row),
        }
    }
    *rows = out;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_render::sample;
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
        pairs.iter().map(|(ms, v)| TrackPoint { release: false, ms: *ms, value: ParameterValue::Float(*v) }).collect()
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

    /// A sequence's level is recorded beside the programmer, under its own key, and
    /// only the one that moved.
    #[test]
    fn a_moved_sequence_level_is_recorded_and_a_still_one_is_not() {
        let (moved, still) = (Uuid::new_v4(), Uuid::new_v4());
        let mut pass = Pass::new();
        pass.observe_with_levels(0, &[], &[(moved, 1.0), (still, 0.6)]);
        pass.observe_with_levels(100, &[], &[(moved, 0.7), (still, 0.6)]);
        pass.observe_with_levels(200, &[], &[(moved, 0.4), (still, 0.6)]);
        let track = pass.into_track();
        assert_eq!(track.keys.len(), 1, "{track:?}");
        assert_eq!(track.keys[0].fixture_id, moved);
        assert_eq!(track.keys[0].key, LEVEL_KEY);
        assert_eq!(pairs(&track.keys[0].points), vec![(0, 1.0), (100, 0.7), (200, 0.4)], "from where it was");
    }

    #[test]
    fn what_was_held_when_the_pass_began_is_only_where_it_starts() {
        let mut pass = Pass::new();
        pass.observe(0, &[held(0.5)]);
        pass.observe(100, &[held(0.5)]);
        assert!(pass.is_empty(), "a held value nobody moved is the baseline");
        pass.observe(200, &[held(0.7)]);
        pass.observe(300, &[held(0.7)]);
        pass.observe(400, &[held(0.2)]);
        let track = pass.into_track();
        assert_eq!(pairs(&track.keys[0].points), vec![(0, 0.5), (200, 0.7), (400, 0.2)], "from what it held");
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
        let mut lap = pass.continuing(0);
        lap.observe(100, &[held(0.4)]);
        assert!(lap.is_empty(), "the hand did not move across the wrap");
        lap.observe(200, &[held(0.9)]);
        assert_eq!(pairs(&lap.into_track().keys[0].points), vec![(0, 0.4), (200, 0.9)]);
    }

    #[test]
    fn a_loop_shorter_than_a_second_is_a_jump_and_a_correction_is_not() {
        assert!(jumped(810, 10), "a 0.8 s loop wrapping");
        assert!(!jumped(810, 795), "the sound card pulling the show back 15 ms");
        assert!(!jumped(810, 1_500), "a forward correction under a second");
        assert!(jumped(810, 5_000), "a locate forwards");
    }

    fn a_row(asset: &str, key: &str, from: Option<u32>, to: Option<u32>) -> TimelineTrack {
        TimelineTrack {
            id: Uuid::new_v4(),
            name: String::new(),
            asset: asset.into(),
            input_id: None,
            offset_ms: 0,
            enabled: true,
            fixture_id: Some(jaw()),
            key: Some(key.into()),
            from_ms: from,
            to_ms: to,
        }
    }

    fn windows(rows: &[TimelineTrack], key: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
        rows.iter()
            .filter(|r| r.key.as_deref() == Some(key))
            .map(|r| (r.asset.clone(), r.from_ms, r.to_ms))
            .collect()
    }

    /// A better second half of the jaw: recorded from the middle, the old take keeps
    /// the first half and the new pass has the rest, completely.
    #[test]
    fn a_pass_replaces_its_whole_stretch_of_the_keys_it_touched() {
        let mut rows = vec![a_row("old", "Intensity", None, None), a_row("old", "Pan", None, None)];
        let pass = a_track("Intensity", &[(1_500, 0.9), (2_500, 0.1)]);
        store(&mut rows, &pass, "new", None, 1_000, 3_500);
        assert_eq!(
            windows(&rows, "Intensity"),
            vec![
                ("old".into(), None, Some(1_000)),
                ("old".into(), Some(3_500), None),
                ("new".into(), Some(1_000), Some(3_500)),
            ]
        );
        assert_eq!(windows(&rows, "Pan"), vec![("old".into(), None, None)], "the pan was not touched");
    }

    #[test]
    fn a_touched_key_starts_from_what_was_held_when_the_pass_began() {
        let mut pass = Pass::starting_at(1_000);
        pass.observe(1_000, &[held(0.5)]);
        pass.observe(1_400, &[held(0.8)]);
        assert_eq!(pairs(&pass.into_track().keys[0].points), vec![(1_000, 0.5), (1_400, 0.8)]);
    }

    #[test]
    fn a_held_key_let_go_is_recorded_so_the_stretch_is_removed() {
        let mut pass = Pass::starting_at(0);
        pass.observe(0, &[held(0.5)]);
        pass.observe(800, &[]);
        let track = pass.into_track();
        let points = &track.keys[0].points;
        assert_eq!(sample(points, 100), Some(&ParameterValue::Float(0.5)));
        assert_eq!(sample(points, 800), None);
    }

    #[test]
    fn an_old_take_becomes_a_row_per_parameter_on_the_same_recording() {
        let mut row = a_row("old", "x", None, Some(900));
        row.fixture_id = None;
        row.key = None;
        let mut rows = vec![row];
        let keys = HashMap::from([("old".to_string(), vec![(jaw(), "Intensity".into()), (jaw(), "Pan".into())])]);
        assert!(per_parameter(&mut rows, &keys));
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.asset == "old" && r.to_ms == Some(900)));
        assert!(!per_parameter(&mut rows, &keys), "already split");
    }

    #[test]
    fn a_flash_key_let_go_is_recorded_as_let_go() {
        // The wing's flash: full on press, the programmer entry gone on release.
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        pass.observe(1_000, &[held(1.0)]);
        pass.observe(1_400, &[]);
        let track = pass.into_track();
        let points = &track.keys[0].points;
        assert_eq!(points.len(), 2);
        assert_eq!(sample(points, 1_200), Some(&ParameterValue::Float(1.0)));
        assert_eq!(sample(points, 1_400), None, "back to whatever is underneath");
    }
}
