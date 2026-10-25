//! A timeline's recordings, as the one track that plays.
//!
//! A recording is an asset and never changes. What an operator edits is the rows that
//! point into one — a `TimelineTrack` per parameter, each with a window — so cropping
//! a take, slicing it in two and deleting a part of it are all writes of rows and none
//! of them re-encodes anything. What plays is those rows composed: each one's window
//! of its parameter, nudged by its offset, laid over the rows before it.
//!
//! **A window is the row's, completely.** Inside it the row says what its recording
//! says, *including nothing*: a recording that is silent somewhere in the window is
//! silence there, not a gap the row underneath shows through. That is what makes
//! re-recording a parameter replace that stretch of it rather than add to it, and it is
//! how a part of a take is removed. Outside its window a row says nothing at all, and
//! at the window's end whatever was there before it carries on.
//!
//! **A flash is laid over all of it.** A flash key pressed during a take is not a
//! stretch of a recording but one act with a length — a value from one moment to
//! another — so it is a [`Flash`] rather than a row. Flashes go on top of every row,
//! whatever order anything was recorded in, and never cut one: over a flash the flash
//! is what plays, and at its end whatever was there carries on, an earlier flash
//! included. Pressing a button during a second pass adds to the first rather than
//! erasing it, which is what a button on a desk means.
//!
//! Here, in the crate compiled twice, because the station puts the composition on a
//! wire and the page draws it, and two compositions would disagree exactly at the edge
//! of a slice somebody just made.

use std::collections::HashMap;

use uuid::Uuid;

use crate::track::{sample, Track, TrackKey, TrackPoint};
use crate::ParameterValue;

/// One row of a timeline's recordings, borrowed: which recording, which of its
/// parameters, and over which stretch of the timeline.
#[derive(Debug, Clone, Copy)]
pub struct Part<'a> {
    pub track: &'a Track,
    /// The parameter this row plays, or every parameter in the recording — a take
    /// stored before rows were per parameter.
    pub fixture_id: Option<Uuid>,
    pub key: Option<&'a str>,
    /// The window, in timeline milliseconds after the offset is applied. `None` is
    /// open on that side.
    pub from_ms: Option<u32>,
    pub to_ms: Option<u32>,
    /// The nudge: a recording played this much later. Signed.
    pub offset_ms: i32,
}

impl Part<'_> {
    fn plays(&self, key: &TrackKey) -> bool {
        self.fixture_id.is_none_or(|id| id == key.fixture_id) && self.key.is_none_or(|k| k == key.key)
    }
}

/// One flash, borrowed: a value held on one parameter over `[from_ms, to_ms)`.
#[derive(Debug, Clone, Copy)]
pub struct Flash<'a> {
    pub fixture_id: Uuid,
    pub key: &'a str,
    pub value: &'a ParameterValue,
    pub from_ms: u32,
    pub to_ms: u32,
}

/// Every row, composed. Later rows win where windows overlap.
pub fn compose(parts: &[Part<'_>]) -> Track {
    compose_with(parts, &[])
}

/// Every row composed, and every flash over the result. Flashes are laid in order of
/// where they start, so where two overlap the later one wins until it ends and the
/// earlier one is back for whatever is left of it.
pub fn compose_with(parts: &[Part<'_>], flashes: &[Flash<'_>]) -> Track {
    let mut track = compose_rows(parts);
    if flashes.is_empty() {
        return track;
    }
    let mut ordered: Vec<&Flash<'_>> = flashes.iter().filter(|f| f.to_ms > f.from_ms).collect();
    ordered.sort_by_key(|f| f.from_ms);
    for flash in ordered {
        let segment = [TrackPoint { release: false, ms: flash.from_ms, value: flash.value.clone() }];
        match track.keys.iter_mut().find(|k| k.fixture_id == flash.fixture_id && k.key == flash.key) {
            Some(key) => key.points = overlay(&key.points, &segment, flash.from_ms, Some(flash.to_ms)),
            None => track.keys.push(TrackKey {
                fixture_id: flash.fixture_id,
                key: flash.key.to_string(),
                points: overlay(&[], &segment, flash.from_ms, Some(flash.to_ms)),
            }),
        }
    }
    track
}

fn compose_rows(parts: &[Part<'_>]) -> Track {
    let mut keys: Vec<TrackKey> = Vec::new();
    let mut index: HashMap<(Uuid, String), usize> = HashMap::new();
    for part in parts {
        let lo = part.from_ms.unwrap_or(0);
        if part.to_ms.is_some_and(|hi| hi <= lo) {
            continue;
        }
        for key in part.track.keys.iter().filter(|key| part.plays(key)) {
            let segment = window(&shift(&key.points, part.offset_ms), lo, part.to_ms);
            match index.get(&(key.fixture_id, key.key.clone())) {
                Some(&at) => keys[at].points = overlay(&keys[at].points, &segment, lo, part.to_ms),
                None => {
                    index.insert((key.fixture_id, key.key.clone()), keys.len());
                    keys.push(TrackKey {
                        fixture_id: key.fixture_id,
                        key: key.key.clone(),
                        points: overlay(&[], &segment, lo, part.to_ms),
                    });
                }
            }
        }
    }
    keys.retain(|key| !key.points.is_empty());
    Track { keys }
}

/// A recording's points played `offset_ms` later. Points pushed before the start of
/// the timeline land on it, the last of them winning.
fn shift(points: &[TrackPoint], offset_ms: i32) -> Vec<TrackPoint> {
    let mut out: Vec<TrackPoint> = Vec::with_capacity(points.len());
    for point in points {
        let ms = (point.ms as i64 + offset_ms as i64).clamp(0, u32::MAX as i64) as u32;
        match out.last_mut() {
            Some(last) if last.ms == ms => *last = TrackPoint { ms, ..point.clone() },
            _ => out.push(TrackPoint { ms, ..point.clone() }),
        }
    }
    out
}

/// What a recording says from `lo` up to `hi`: its value at `lo`, then every change
/// after it.
fn window(points: &[TrackPoint], lo: u32, hi: Option<u32>) -> Vec<TrackPoint> {
    let mut out = Vec::new();
    if !points.iter().any(|p| p.ms == lo) {
        if let Some(value) = sample(points, lo as u64) {
            out.push(TrackPoint { release: false, ms: lo, value: value.clone() });
        }
    }
    out.extend(points.iter().filter(|p| p.ms >= lo && hi.is_none_or(|hi| p.ms < hi)).cloned());
    out
}

/// `segment` written over `base` from `lo` up to `hi`. The window is the segment's
/// completely — where it says nothing, nothing is said — and from `hi` the base
/// carries on with whatever it says there.
fn overlay(base: &[TrackPoint], segment: &[TrackPoint], lo: u32, hi: Option<u32>) -> Vec<TrackPoint> {
    let mut points: Vec<TrackPoint> = base.iter().filter(|p| p.ms < lo).cloned().collect();
    if segment.first().is_none_or(|p| p.ms > lo) {
        points.push(TrackPoint::released(lo));
    }
    points.extend(segment.iter().cloned());
    if let Some(hi) = hi {
        points.push(match sample(base, hi as u64) {
            Some(value) => TrackPoint { release: false, ms: hi, value: value.clone() },
            None => TrackPoint::released(hi),
        });
        points.extend(base.iter().filter(|p| p.ms > hi).cloned());
    }
    // A value that did not move is not a change, which is what a track is made of.
    points.dedup_by(|later, earlier| later.same_as(earlier));
    // A release before anything was said is nothing at all.
    let said = points.iter().position(|p| !p.release).unwrap_or(points.len());
    points.drain(..said);
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParameterValue;

    fn fixture() -> Uuid {
        Uuid::from_u128(7)
    }

    fn take(key: &str, pairs: &[(u32, Option<f32>)]) -> Track {
        Track {
            keys: vec![TrackKey {
                fixture_id: fixture(),
                key: key.into(),
                points: pairs
                    .iter()
                    .map(|(ms, v)| match v {
                        Some(v) => TrackPoint { release: false, ms: *ms, value: ParameterValue::Float(*v) },
                        None => TrackPoint::released(*ms),
                    })
                    .collect(),
            }],
        }
    }

    fn part<'a>(track: &'a Track, from: Option<u32>, to: Option<u32>) -> Part<'a> {
        Part { track, fixture_id: None, key: None, from_ms: from, to_ms: to, offset_ms: 0 }
    }

    fn pairs(track: &Track, key: &str) -> Vec<(u32, Option<f32>)> {
        track
            .keys
            .iter()
            .find(|k| k.key == key)
            .map(|k| {
                k.points
                    .iter()
                    .map(|p| match (&p.value, p.release) {
                        (_, true) => (p.ms, None),
                        (ParameterValue::Float(v), false) => (p.ms, Some(*v)),
                        _ => panic!("a float"),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_row_with_no_window_plays_its_recording_as_it_is() {
        let jaw = take("Jaw", &[(100, Some(0.5)), (300, Some(0.8)), (500, None)]);
        assert_eq!(pairs(&compose(&[part(&jaw, None, None)]), "Jaw"), vec![(100, Some(0.5)), (300, Some(0.8)), (500, None)]);
    }

    #[test]
    fn a_cropped_row_starts_with_the_value_it_had_and_ends_in_a_release() {
        let jaw = take("Jaw", &[(100, Some(0.5)), (300, Some(0.8)), (500, Some(0.2))]);
        let played = compose(&[part(&jaw, Some(200), Some(400))]);
        assert_eq!(pairs(&played, "Jaw"), vec![(200, Some(0.5)), (300, Some(0.8)), (400, None)]);
    }

    #[test]
    fn two_slices_of_one_take_leave_the_part_between_them_silent() {
        let jaw = take("Jaw", &[(0, Some(0.1)), (200, Some(0.2)), (400, Some(0.3)), (600, Some(0.4))]);
        let played = compose(&[part(&jaw, None, Some(200)), part(&jaw, Some(400), None)]);
        assert_eq!(pairs(&played, "Jaw"), vec![(0, Some(0.1)), (200, None), (400, Some(0.3)), (600, Some(0.4))]);
    }

    #[test]
    fn a_later_row_replaces_its_window_completely_and_the_earlier_resumes_after() {
        let old = take("Jaw", &[(0, Some(0.1)), (500, Some(0.5)), (900, Some(0.9))]);
        // The new pass let the key go at 400: that stretch of the window is silence,
        // not the old take showing through.
        let new = take("Jaw", &[(300, Some(0.7)), (400, None)]);
        let played = compose(&[part(&old, None, None), part(&new, Some(200), Some(700))]);
        assert_eq!(
            pairs(&played, "Jaw"),
            vec![(0, Some(0.1)), (200, None), (300, Some(0.7)), (400, None), (700, Some(0.5)), (900, Some(0.9))]
        );
    }

    #[test]
    fn a_row_plays_only_its_own_parameter_and_its_offset_moves_it() {
        let mut both = take("Jaw", &[(100, Some(0.5))]);
        both.keys.extend(take("Head", &[(100, Some(0.9))]).keys);
        let jaw = Part { key: Some("Jaw"), fixture_id: Some(fixture()), offset_ms: 50, ..part(&both, None, None) };
        let played = compose(&[jaw]);
        assert_eq!(played.keys.len(), 1);
        assert_eq!(pairs(&played, "Jaw"), vec![(150, Some(0.5))]);
    }

    fn flash<'a>(value: &'a ParameterValue, from: u32, to: u32) -> Flash<'a> {
        Flash { fixture_id: fixture(), key: "Jaw", value, from_ms: from, to_ms: to }
    }

    #[test]
    fn a_flash_plays_over_a_take_and_the_take_carries_on_after_it() {
        let jaw = take("Jaw", &[(0, Some(0.2)), (500, Some(0.4))]);
        let full = ParameterValue::Float(1.0);
        let played = compose_with(&[part(&jaw, None, None)], &[flash(&full, 200, 300)]);
        assert_eq!(pairs(&played, "Jaw"), vec![(0, Some(0.2)), (200, Some(1.0)), (300, Some(0.2)), (500, Some(0.4))]);
    }

    #[test]
    fn a_flash_over_nothing_lets_go_at_its_end() {
        let full = ParameterValue::Float(1.0);
        let played = compose_with(&[], &[flash(&full, 200, 300)]);
        assert_eq!(pairs(&played, "Jaw"), vec![(200, Some(1.0)), (300, None)]);
    }

    #[test]
    fn a_later_flash_inside_an_earlier_one_gives_it_back() {
        let (a, b) = (ParameterValue::Float(0.5), ParameterValue::Float(1.0));
        let played = compose_with(&[], &[flash(&b, 200, 300), flash(&a, 100, 400)]);
        assert_eq!(pairs(&played, "Jaw"), vec![(100, Some(0.5)), (200, Some(1.0)), (300, Some(0.5)), (400, None)]);
    }
}
