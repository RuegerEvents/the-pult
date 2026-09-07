//! Two halves, and they are tested very differently.
//!
//! **The grid derivation is arithmetic** and is checked against hand-written beat
//! lists: a steady song, a song that changes tempo, a waltz, a detection with no
//! downbeats in it, and a detection with a beat missing. Those are the cases an
//! operator will actually meet and none of them needs a model.
//!
//! **Inference is checked against a click track generated here.** A real song cannot
//! be checked into a repository — the recording is somebody's copyright and the file
//! is megabytes — and a click at a known tempo is the one signal whose right answer is
//! known without a human listening.
//!
//! It runs in the ordinary suite, and that was in the balance. Unoptimised it took
//! **63 seconds**, which is an `#[ignore]` and therefore a guard nobody runs; the
//! `[profile.dev.package.rten]` entries in the workspace manifest bring it to **one
//! second**, because what was slow was a SIMD matrix kernel compiled as scalar Rust and
//! not the model. Optimising one dependency was the cheaper of the two answers, and it
//! also means the *console* can detect beats in a debug build without somebody
//! deciding the feature is broken.

use super::*;

/// Beats every `interval` ms, `count` of them, with a downbeat every `bar`.
fn steady(interval: u32, count: u32, bar: u32, from: u32) -> (Vec<u32>, Vec<u32>) {
    let beats: Vec<u32> = (0..count).map(|i| from + i * interval).collect();
    let downbeats: Vec<u32> =
        beats.iter().copied().enumerate().filter(|(i, _)| *i as u32 % bar == 0).map(|(_, b)| b).collect();
    (beats, downbeats)
}

#[test]
fn a_song_that_never_changes_tempo_is_one_segment() {
    // 500 ms a beat is 120 bpm.
    let (beats, downbeats) = steady(500, 64, 4, 0);
    let grid = grid_from_beats(&beats, &downbeats);
    assert_eq!(grid.len(), 1, "a steady song is one segment, not one per bar");
    assert_eq!(grid[0].at_ms, 0);
    assert!((grid[0].bpm - 120.0).abs() < 0.5, "{}", grid[0].bpm);
    assert_eq!(grid[0].beats_per_bar, 4);
}

#[test]
fn a_tempo_change_starts_a_new_segment_on_a_downbeat() {
    // Eight bars at 120, then eight at 90 (667 ms a beat).
    let (mut beats, mut downbeats) = steady(500, 32, 4, 0);
    let after = *beats.last().unwrap() + 500;
    let (more, more_downbeats) = steady(667, 32, 4, after);
    beats.extend(more);
    downbeats.extend(more_downbeats);

    let grid = grid_from_beats(&beats, &downbeats);
    assert_eq!(grid.len(), 2, "{grid:?}");
    assert!((grid[0].bpm - 120.0).abs() < 1.0);
    assert!((grid[1].bpm - 90.0).abs() < 1.0);
    assert_eq!(grid[1].at_ms, after, "the change lands on the bar line, not between beats");
}

#[test]
fn the_bar_length_comes_off_the_downbeat_spacing() {
    let (beats, downbeats) = steady(500, 36, 3, 0);
    let grid = grid_from_beats(&beats, &downbeats);
    assert_eq!(grid[0].beats_per_bar, 3, "a waltz");
}

/// One dropped bar does not change what the song is in: the mode, not the mean.
#[test]
fn an_odd_bar_does_not_move_the_time_signature() {
    let (beats, mut downbeats) = steady(500, 40, 4, 0);
    // Insert a spurious downbeat halfway through one bar, which is what a detector
    // does on a strong backbeat.
    downbeats.push(beats[18]);
    downbeats.sort_unstable();
    let grid = grid_from_beats(&beats, &downbeats);
    assert_eq!(grid[0].beats_per_bar, 4);
}

/// A beat the detector missed doubles one interval. The median decides how many beats
/// the span holds; a mean over the intervals would report the song as slower than it is.
#[test]
fn a_missed_beat_does_not_move_the_tempo() {
    let (mut beats, downbeats) = steady(500, 64, 4, 0);
    beats.remove(37);
    let grid = grid_from_beats(&beats, &downbeats);
    assert!((grid[0].bpm - 120.0).abs() < 1.0, "{}", grid[0].bpm);
}

/// The defect a real song found. The model works at 50 frames a second, so every beat
/// it reports is quantised to 20 ms — and at 128 bpm a 468.75 ms beat lands on 460 or
/// 480. A tempo taken from one interval is then 130.4 or 125, either of which walks a
/// whole bar out of step inside a minute. Taken from the *span*, it is right.
#[test]
fn quantised_beats_still_give_the_tempo_they_came_from() {
    let true_interval = 60_000.0 / 128.0;
    // Thirty-two bars of 128 bpm, each beat snapped to the detector's 20 ms grid.
    let beats: Vec<u32> = (0..128)
        .map(|i| ((i as f32 * true_interval / 20.0).round() * 20.0) as u32)
        .collect();
    let downbeats: Vec<u32> = beats.iter().copied().step_by(4).collect();

    // The naive reading, for the record: the median of those intervals is a whole
    // frame away from the truth.
    let mut gaps: Vec<u32> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let from_one_gap = 60_000.0 / gaps[gaps.len() / 2] as f32;
    assert!((from_one_gap - 128.0).abs() > 1.0, "the naive reading is {from_one_gap}");

    let grid = grid_from_beats(&beats, &downbeats);
    assert!((grid[0].bpm - 128.0).abs() < 0.2, "found {} bpm", grid[0].bpm);
}

#[test]
fn beats_with_no_downbeats_still_give_a_grid_in_four() {
    let (beats, _) = steady(480, 20, 4, 1_000);
    let grid = grid_from_beats(&beats, &[]);
    assert_eq!(grid.len(), 1);
    assert_eq!(grid[0].at_ms, 1_000, "the first beat, since there is no bar line to use");
    assert_eq!(grid[0].beats_per_bar, 4, "the guess to make when nothing says otherwise");
    assert!((grid[0].bpm - 125.0).abs() < 0.5);
}

#[test]
fn nothing_detected_is_no_grid_rather_than_a_default_one() {
    assert!(grid_from_beats(&[], &[]).is_empty());
    assert!(grid_from_beats(&[1_000], &[1_000]).is_empty(), "one beat is not a tempo");
}

/// Beats and downbeats are reported in milliseconds and a downbeat is always also a
/// beat, which is the invariant the grid derivation rests on.
#[test]
fn snapping_puts_every_downbeat_on_a_beat() {
    let beats = vec![1.0f64, 2.0, 3.0, 4.0];
    let mut downbeats = vec![1.1f64, 2.9];
    snap(&beats, &mut downbeats);
    assert_eq!(downbeats, vec![1.0, 3.0]);

    // Two downbeats that snap to one beat are one downbeat.
    let mut both = vec![1.9f64, 2.05];
    snap(&beats, &mut both);
    assert_eq!(both, vec![2.0]);
}

#[test]
fn a_peak_is_a_local_maximum_over_a_half_and_adjacent_ones_merge() {
    let mut logits = vec![-5.0f32; 40];
    logits[10] = 3.0;
    logits[30] = 1.0;
    assert_eq!(peaks(&logits), vec![10.0, 30.0]);

    // Two adjacent frames of equal confidence are one beat between them.
    let mut tied = vec![-5.0f32; 20];
    tied[8] = 2.0;
    tied[9] = 2.0;
    assert_eq!(peaks(&tied), vec![8.5]);

    assert!(peaks(&[-1.0, -0.5, -2.0]).is_empty(), "nothing above a half is nothing");
}

/// The chunking is what makes a six-minute song possible at all, and every frame of it
/// has to be covered by exactly the chunk that is best informed about it.
#[test]
fn every_frame_of_a_long_song_is_covered_by_a_chunk() {
    for frames in [40usize, 500, 1_488, 1_500, 3_000, 9_000, 18_000] {
        let mut covered = vec![false; frames];
        for start in starts(frames) {
            let (_, chunk_frames) = cut(&vec![0.0; frames * MELS], frames, start);
            let from = (start + BORDER as i32).max(0) as usize;
            let to = ((start + chunk_frames as i32 - BORDER as i32).max(0) as usize).min(frames);
            for frame in from..to {
                covered[frame] = true;
            }
        }
        assert!(covered.iter().all(|c| *c), "{frames} frames: {:?} uncovered", covered.iter().position(|c| !c));
    }
}

/// A short song is one chunk, padded on both sides, and never one that runs off the
/// end of the spectrogram.
#[test]
fn a_short_song_is_one_padded_chunk() {
    assert_eq!(starts(100), vec![-6]);
    let (chunk, frames) = cut(&vec![1.0; 100 * MELS], 100, -6);
    assert_eq!(frames, 112, "six of padding either side of a hundred frames");
    assert!(chunk[..6 * MELS].iter().all(|v| *v == 0.0));
    assert_eq!(chunk[6 * MELS], 1.0);
    assert!(chunk[106 * MELS..].iter().all(|v| *v == 0.0));
}

// ── Inference ─────────────────────────────────────────────────────────────────

/// A click track: a short percussive burst every beat, with a louder one on the "one".
///
/// Not a sine — a beat detector trained on music finds *onsets*, and a steady tone has
/// none. A short noise burst with a fast decay is what a metronome sounds like and is
/// the least musical thing the model will still find a pulse in.
fn click_track(bpm: f32, bars: u32, beats_per_bar: u32, sample_rate: u32) -> Vec<f32> {
    let interval = (60.0 / bpm * sample_rate as f32) as usize;
    let total = interval * (bars * beats_per_bar) as usize;
    let mut out = vec![0.0f32; total];
    let mut state = 99_991u32;
    for beat in 0..(bars * beats_per_bar) {
        let at = beat as usize * interval;
        let level = if beat % beats_per_bar == 0 { 1.0 } else { 0.55 };
        // 40 ms of decaying noise, which is a click with a body rather than a tick.
        for i in 0..(sample_rate as usize / 25) {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let noise = (state >> 16) as f32 / 32_768.0 - 1.0;
            let decay = (-(i as f32) / (sample_rate as f32 * 0.006)).exp();
            if at + i < total {
                out[at + i] += noise * decay * level;
            }
        }
    }
    out
}

/// The model, on a signal whose answer is known.
///
/// The only test in this crate that takes a second rather than a millisecond, and the
/// only one that depends on ten megabytes of weights doing what they did yesterday.
#[test]
fn a_click_track_comes_back_at_the_tempo_it_was_made_at() {
    let sample_rate = DETECT_SAMPLE_RATE;
    let samples = click_track(120.0, 16, 4, sample_rate);
    let mut detector = Detector::small().expect("the embedded models should load");
    let found = detector.detect(&samples, 1, sample_rate).expect("detection");

    assert!(found.beats_ms.len() > 40, "only {} beats", found.beats_ms.len());
    let grid = grid_from_beats(&found.beats_ms, &found.downbeats_ms);
    assert!(!grid.is_empty(), "no grid out of a click track");
    assert!((grid[0].bpm - 120.0).abs() <= 1.0, "found {} bpm", grid[0].bpm);

    // The phase: the first downbeat should be on a bar line of the track, which is
    // every two seconds from zero.
    let first = *found.downbeats_ms.first().expect("a downbeat");
    let off_by = (first as i64 % 2_000).min(2_000 - first as i64 % 2_000);
    assert!(off_by <= 50, "the first downbeat is {off_by} ms off the bar");
}
