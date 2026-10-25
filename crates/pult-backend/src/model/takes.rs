//! A take recorded from the programmer, and how a pass joins the takes already there.
//!
//! Pure, like the rest of `model/`: what the programmer held at a position goes in, and
//! change points and flashes come out. The station's half — which station records,
//! where the bytes go, what is released afterwards — is `infra/connectors/input.rs`,
//! beside the take an input records, because both end in the same acts: encode, store,
//! and put up for review.
//!
//! **Touch-scoped.** A pass records the keys that *change* while it runs, from the
//! moment each is first touched, and nothing else. A key the programmer was already
//! holding when the pass began and that nobody moves is not in it. That is what lets
//! a second pass record the head without re-recording the jaw it is being played over.
//!
//! **A pass replaces a key from its first touch to the stop — a punch-in.** Each key
//! it touched gets a row of its own (`TimelineTrack`, one per parameter) from where
//! that key was first touched to where the pass stopped, and every row already playing
//! that key is cut out of that stretch (`types::timeline::store_take`). So the old take
//! plays right up to the moment the hand lands on the fader, which is what re-recording
//! the second half of a jaw means; before this the window began where the *pass* began
//! and touching a fader at 0:30 wiped the first thirty seconds with it (found at the
//! gig). A key let go during the pass says nothing from there to the stop, which is how
//! a stretch is removed. Keys nobody touched are left as they were.
//!
//! **A flash is not a take.** A key that *appears* in the programmer during the pass
//! and *leaves* it again having held one value the whole time is a button: the wing's
//! flash key, a surface's flash tile, anything that writes a value on press and takes
//! it away on release. That becomes a `TimelineFlash` — one value, a start and a
//! length — which is added over every take and cuts nothing, so pressing a button on a
//! second pass adds to the first instead of punching it out. Anything else is a fader:
//! a key that moved while it was held, a key that was already held when the pass began,
//! and a key still held when the pass stops (pressed and not let go is a level, not a
//! flash). The rule asks only what the programmer did, never who wrote it, so it holds
//! for every surface that flashes.
//!
//! The recording itself is one asset per pass and never changes; the rows point into
//! it. What plays is the rows composed and the flashes over them —
//! `pult_render::compose_with`.

use std::collections::{BTreeMap, HashMap};

use pult_render::{Track, TrackKey, TrackPoint};
use pult_schema::types::{
    fixture::ParameterValue,
    parameter_key,
    programmer::ProgrammerValue,
    sequence::LEVEL_KEY,
    timeline::{split_row, PendingKey, TimelineFlash, TimelineTrack},
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

type At = (Uuid, String);

/// A key that appeared during the pass and has held one value since: a flash, if it
/// leaves before anything else happens to it.
#[derive(Debug, Clone)]
struct Pressed {
    ms: u32,
    value: ParameterValue,
}

/// One pass of the programmer, in progress.
#[derive(Debug, Default)]
pub struct Pass {
    /// What the programmer held at the last look, per key. Primed by the first look,
    /// which records nothing: what was already held when the pass began is the
    /// baseline, not a move.
    seen: Option<HashMap<At, ParameterValue>>,
    /// The take's change points so far, from each key's first touch.
    points: BTreeMap<At, Vec<TrackPoint>>,
    /// Keys that appeared and have not changed or left yet — flashes in the making.
    pressed: HashMap<At, Pressed>,
    /// Flashes finished: appeared, held one value, and left.
    flashes: Vec<(At, Pressed, u32)>,
    /// Where on the timeline the pass began.
    from_ms: u32,
}

/// A pass, finished: the recording, where each of its keys starts, and its flashes.
#[derive(Debug, Default)]
pub struct Recorded {
    pub track: Track,
    pub keys: Vec<PendingKey>,
    pub flashes: Vec<TimelineFlash>,
}

impl Recorded {
    pub fn is_empty(&self) -> bool {
        self.track.keys.is_empty() && self.flashes.is_empty()
    }
}

impl Pass {
    pub fn new() -> Self {
        Self::default()
    }

    /// A pass beginning at a position on the timeline.
    pub fn starting_at(position_ms: u64) -> Self {
        Pass { from_ms: clamp(position_ms), ..Self::default() }
    }

    /// The next pass after a jump, from the position the playhead jumped to, carrying
    /// on from what this one last saw.
    ///
    /// A loop wrapping is a jump, and the operator's hand is still on the fader: the
    /// next lap's baseline is what the programmer holds now, not a fresh first look
    /// that would swallow the next move as "already held".
    pub fn continuing(&self, position_ms: u64) -> Self {
        Pass { seen: self.seen.clone(), from_ms: clamp(position_ms), ..Self::default() }
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
    /// every time it is looked at, writes nothing. A level is always there, so it is
    /// never a flash.
    pub fn observe_with_levels(
        &mut self,
        position_ms: u64,
        programmer: &[ProgrammerValue],
        levels: &[(Uuid, f32)],
    ) {
        let now: HashMap<At, ParameterValue> = programmer
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
        let Some(seen) = self.seen.replace(now.clone()) else { return };
        let ms = clamp(position_ms);
        for at in seen.keys().filter(|at| !now.contains_key(*at)) {
            // Appeared, held one value, and gone: a button let go. Its own object.
            if let Some(pressed) = self.pressed.remove(at) {
                let out = ms.max(pressed.ms);
                self.flashes.push((at.clone(), pressed, out));
                continue;
            }
            // A key that left the programmer otherwise is let go here: a key cleared, a
            // fader's entry dropped. Recorded as a release, so the take says nothing
            // from this point and what is under it shows through.
            self.point(at.clone(), TrackPoint::released(ms));
        }
        for (at, value) in now {
            let Some(was) = seen.get(&at) else {
                // Appeared: a press, until it does something a press does not.
                self.pressed.insert(at, Pressed { ms, value });
                continue;
            };
            if *was == value {
                continue;
            }
            // A key that moved while held is a fader, and so is everything it did
            // since it appeared.
            if let Some(pressed) = self.pressed.remove(&at) {
                self.point(at.clone(), TrackPoint { release: false, ms: pressed.ms, value: pressed.value });
            }
            self.point(at, TrackPoint { release: false, ms, value });
        }
    }

    /// One change point on the take. A playhead the sound card nudged backwards by a
    /// few milliseconds is not a jump (see [`jumped`]), but a track's points have to
    /// ascend; holding the later position costs those few milliseconds and nothing else.
    fn point(&mut self, at: At, point: TrackPoint) {
        let points = self.points.entry(at).or_default();
        let ms = points.last().map_or(point.ms, |last| point.ms.max(last.ms));
        let point = TrackPoint { ms, ..point };
        match points.last_mut() {
            Some(last) if last.ms == ms => *last = point,
            Some(last) if last.release && point.release => {}
            _ => points.push(point),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.pressed.is_empty() && self.flashes.is_empty()
    }

    /// How many keys the pass has touched so far, flashes included.
    pub fn keys(&self) -> usize {
        let mut keys: std::collections::BTreeSet<&At> = self.points.keys().collect();
        keys.extend(self.pressed.keys());
        keys.extend(self.flashes.iter().map(|(at, ..)| at));
        keys.len()
    }

    /// How many change points and flashes it holds.
    pub fn len(&self) -> usize {
        self.points.values().map(Vec::len).sum::<usize>() + self.pressed.len() + self.flashes.len()
    }

    /// The pass, finished. A key still pressed when it ends was not let go, so it is a
    /// level from where it was pressed rather than a flash; every take key's stretch
    /// starts at its first point, which is its first touch.
    pub fn finish(mut self) -> Recorded {
        for (at, pressed) in std::mem::take(&mut self.pressed) {
            self.point(at, TrackPoint { release: false, ms: pressed.ms, value: pressed.value });
        }
        let mut keys = Vec::new();
        let track = Track {
            keys: self
                .points
                .into_iter()
                .filter(|(_, points)| !points.is_empty())
                .map(|((fixture_id, key), points)| {
                    keys.push(PendingKey { fixture_id, key: key.clone(), from_ms: points[0].ms });
                    TrackKey { fixture_id, key, points }
                })
                .collect(),
        };
        let mut flashes: Vec<TimelineFlash> = self
            .flashes
            .into_iter()
            .map(|((fixture_id, key), pressed, out)| TimelineFlash {
                id: Uuid::new_v4(),
                fixture_id,
                key,
                value: pressed.value,
                at_ms: pressed.ms,
                duration_ms: (out - pressed.ms).max(1),
            })
            .collect();
        flashes.sort_by_key(|f| f.at_ms);
        Recorded { track, keys, flashes }
    }
}

fn clamp(position_ms: u64) -> u32 {
    position_ms.min(u32::MAX as u64) as u32
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
        timeline::{store_take, PendingPass},
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

    fn pairs(points: &[TrackPoint]) -> Vec<(u32, f32)> {
        points
            .iter()
            .map(|p| match p.value {
                ParameterValue::Float(v) => (p.ms, v),
                _ => panic!("a float"),
            })
            .collect()
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
        let done = pass.finish();
        assert_eq!(done.track.keys.len(), 1, "{:?}", done.track);
        assert_eq!(done.track.keys[0].fixture_id, moved);
        assert_eq!(done.track.keys[0].key, LEVEL_KEY);
        assert_eq!(pairs(&done.track.keys[0].points), vec![(100, 0.7), (200, 0.4)], "from the first touch");
        assert_eq!(done.keys[0].from_ms, 100);
    }

    #[test]
    fn a_held_key_is_recorded_from_its_first_touch() {
        let mut pass = Pass::starting_at(0);
        pass.observe(0, &[held(0.5)]);
        pass.observe(100, &[held(0.5)]);
        assert!(pass.is_empty(), "a held value nobody moved is the baseline");
        pass.observe(200, &[held(0.7)]);
        pass.observe(400, &[held(0.2)]);
        let done = pass.finish();
        assert_eq!(pairs(&done.track.keys[0].points), vec![(200, 0.7), (400, 0.2)]);
        assert_eq!(done.keys[0].from_ms, 200, "the old take plays up to the touch");
    }

    #[test]
    fn a_key_that_appears_and_moves_is_a_fader_from_where_it_appeared() {
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        pass.observe(250, &[held(0.3)]);
        pass.observe(300, &[held(0.6)]);
        let done = pass.finish();
        assert!(done.flashes.is_empty());
        assert_eq!(pairs(&done.track.keys[0].points), vec![(250, 0.3), (300, 0.6)]);
        assert_eq!(done.keys[0].from_ms, 250);
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
        pass.observe(0, &[held(0.0)]);
        pass.observe(500, &[held(0.1)]);
        pass.observe(490, &[held(0.2)]);
        assert_eq!(pairs(&pass.finish().track.keys[0].points), vec![(500, 0.2)]);
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
        assert_eq!(pairs(&lap.finish().track.keys[0].points), vec![(200, 0.9)]);
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

    /// A better second half of the jaw: the pass began at 1 s, the hand landed at
    /// 1.5 s, and the old take keeps everything up to the touch.
    #[test]
    fn a_pass_replaces_a_key_from_its_first_touch_to_the_stop() {
        let mut rows = vec![a_row("old", "Intensity", None, None), a_row("old", "Pan", None, None)];
        let pass = PendingPass {
            asset: "new".into(),
            input_id: None,
            keys: vec![PendingKey { fixture_id: jaw(), key: "Intensity".into(), from_ms: 1_500 }],
            out_ms: 3_500,
            flashes: Vec::new(),
        };
        store_take(&mut rows, &pass, |_, _| true);
        assert_eq!(
            windows(&rows, "Intensity"),
            vec![
                ("old".into(), None, Some(1_500)),
                ("old".into(), Some(3_500), None),
                ("new".into(), Some(1_500), Some(3_500)),
            ]
        );
        assert_eq!(windows(&rows, "Pan"), vec![("old".into(), None, None)], "the pan was not touched");
    }

    #[test]
    fn a_held_key_let_go_is_recorded_so_the_rest_of_the_pass_is_removed() {
        let mut pass = Pass::starting_at(0);
        pass.observe(0, &[held(0.5)]);
        pass.observe(800, &[]);
        let done = pass.finish();
        assert!(done.flashes.is_empty(), "it was held before the pass, so it is not a press");
        let points = &done.track.keys[0].points;
        assert_eq!(done.keys[0].from_ms, 800);
        assert_eq!(sample(points, 900), None);
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

    /// The wing's flash: full on press, the programmer entry gone on release. One
    /// object with a length, and no take at all.
    #[test]
    fn a_press_and_a_release_is_a_flash_and_not_a_take() {
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        pass.observe(1_000, &[held(1.0)]);
        pass.observe(1_200, &[held(1.0)]);
        pass.observe(1_400, &[]);
        pass.observe(2_000, &[held(1.0)]);
        pass.observe(2_100, &[]);
        let done = pass.finish();
        assert!(done.track.keys.is_empty(), "{:?}", done.track);
        let flashes: Vec<(u32, u32)> = done.flashes.iter().map(|f| (f.at_ms, f.duration_ms)).collect();
        assert_eq!(flashes, vec![(1_000, 400), (2_000, 100)]);
        assert_eq!(done.flashes[0].value, ParameterValue::Float(1.0));
    }

    #[test]
    fn a_key_pressed_and_never_let_go_is_a_level() {
        let mut pass = Pass::new();
        pass.observe(0, &[]);
        pass.observe(1_000, &[held(1.0)]);
        let done = pass.finish();
        assert!(done.flashes.is_empty());
        assert_eq!(pairs(&done.track.keys[0].points), vec![(1_000, 1.0)]);
        assert_eq!(done.keys[0].from_ms, 1_000);
    }
}
