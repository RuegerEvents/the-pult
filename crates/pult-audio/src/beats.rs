//! Where the beats are, and where the bars start.
//!
//! # Attribution
//!
//! The inference path below — the mel front end, the chunking of a long spectrogram
//! into 1500-frame windows with 6-frame borders, the "keep first" aggregation, and the
//! max-pool peak picking with its running-mean deduplication — is a port of
//! **`beat-this` 1.0.0** (MIT, © Daniel Gómez, <https://github.com/danigb/beat-this-rs>),
//! itself a Rust port of **Beat This!** (MIT, © CPJKU,
//! <https://github.com/CPJKU/beat_this>, Foscarin et al., ISMIR 2024). The model
//! weights in `models/` are that project's ONNX exports of the CPJKU checkpoints, and
//! `models/README.md` says where each one came from.
//!
//! It is **vendored rather than depended on**, and the reason is mechanical: the
//! `beat-this` crate's `clap`, `glob`, `hound`, `rubato` and `symphonia`-with-all-codecs
//! are not optional dependencies, so there is no feature that turns a CLI's argument
//! parser and a second copy of a decoder off. This crate is meant to have no OS in it
//! at all, and a command-line parser is about as much OS as a crate can have. What is
//! kept is the arithmetic, which is what the paper is about; what is left behind is a
//! program.
//!
//! # Why this detector and not another
//!
//! Every conventional one is copyleft or Python: aubio is GPL-3, BTrack GPL-3,
//! Essentia AGPL-3, MiniBPM GPL-2 or paid, the QM Vamp plugins GPL-2; madmom and
//! librosa are Python and would put an interpreter in a lighting console. Beat This!
//! is MIT for both code and weights, beats madmom in its own paper, and — the thing
//! that actually decided it — gives **downbeats as well as beats**. A speed master
//! wants the "one", and a detector that only found the pulse would leave an operator
//! tapping for the bar line anyway.
//!
//! # The models are files, and one of them is not here
//!
//! Two are embedded: the log-mel front end (~270 KB) and the small beat model
//! (~10 MB). Embedded rather than fetched because a console at a venue has no
//! internet, and a feature that only works on the office wifi is a feature nobody
//! trusts. The full-accuracy model is 83 MB, which is more than the whole rest of the
//! binary, so it is **fetched on request into the station's config directory** and
//! verified by sha256 — the station's half of that is `infra/audio`.
//!
//! # What it answers, and what the operator does with it
//!
//! Beats and downbeats, in seconds. [`grid_from_beats`] turns those into the segments
//! `Timeline::grid` holds. **The detector proposes and the operator confirms**: what
//! comes back is written to `Timeline::detected` and drawn over the waveform, and only
//! a button press puts it in `grid`. A console that silently rewrote the grid of a
//! show somebody had already programmed against would be worse than one that could not
//! detect anything.

use std::collections::HashMap;

use rten::{Model, NodeId, Value};
use rten_tensor::{AsView, Layout};

use crate::{resample, BeatsError};

/// The rate the mel front end was trained at. Not negotiable: it is baked into the
/// model's own STFT.
pub const DETECT_SAMPLE_RATE: u32 = 22_050;
/// Frames per second the beat model works in, which is what turns a frame index into a
/// position.
const FPS: f64 = 50.0;
/// Frames per chunk: thirty seconds, which is the length the model was trained on.
const CHUNK: usize = 1_500;
/// Frames trimmed from each edge of a chunk's prediction, where the model has no
/// context on one side.
const BORDER: usize = 6;
const STRIDE: usize = CHUNK - 2 * BORDER;
/// Mel bands. Part of the model's own shape.
const MELS: usize = 128;

/// The log-mel front end, ~270 KB.
const MEL_MODEL: &[u8] = include_bytes!("../models/mel_spectrogram.onnx");
/// The small beat model, ~10 MB. See the module header on why this one is embedded.
const SMALL_MODEL: &[u8] = include_bytes!("../models/beat_this_small.onnx");

/// The sha256 of the full-accuracy model, so a station can verify what it fetched.
///
/// Here rather than in the backend because it is a fact about the *model*, and the
/// crate that knows how to run one is the crate that should say which one it will run.
pub const FULL_MODEL_SHA256: &str =
    "5f810debe53459b559127fb55bbad40035bb47cc567b20e501670f968c770f02";

/// Where the full model is published. The `beat-this-rs` release that holds the ONNX
/// export; the checksum above is what makes fetching it safe.
pub const FULL_MODEL_URL: &str =
    "https://github.com/danigb/beat-this-rs/releases/download/model-large/beat_this.onnx";

/// What one run of the detector found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detected {
    /// Beat positions, in milliseconds.
    pub beats_ms: Vec<u32>,
    /// Downbeat positions — the "one" of each bar — in milliseconds. A subset of
    /// `beats_ms`, because the postprocessing snaps each downbeat onto the nearest
    /// beat before it is reported.
    pub downbeats_ms: Vec<u32>,
}

/// A loaded detector: two models, ready to be run.
///
/// Loading is the expensive part — the small model is ten megabytes of weights — so a
/// caller that is going to analyse several files makes one of these and keeps it.
pub struct Detector {
    mel: Loaded,
    beat: Loaded,
}

impl Detector {
    /// The embedded small model. What a console has without asking anybody for
    /// anything.
    pub fn small() -> Result<Detector, BeatsError> {
        Ok(Detector {
            mel: Loaded::from_static(MEL_MODEL)?,
            beat: Loaded::from_static(SMALL_MODEL)?,
        })
    }

    /// The full-accuracy model, from bytes the station fetched and verified.
    ///
    /// Bytes rather than a path, so this crate never opens a file — the rule the rest
    /// of it lives by.
    pub fn full(beat_model: Vec<u8>) -> Result<Detector, BeatsError> {
        Ok(Detector { mel: Loaded::from_static(MEL_MODEL)?, beat: Loaded::from_owned(beat_model)? })
    }

    /// Find the beats in interleaved f32 samples.
    ///
    /// Downmixed to mono and resampled to [`DETECT_SAMPLE_RATE`] here rather than by
    /// the caller, because the rate is the model's business and nothing outside this
    /// module should have to know it.
    pub fn detect(
        &mut self,
        samples: &[f32],
        channels: usize,
        sample_rate: u32,
    ) -> Result<Detected, BeatsError> {
        let channels = channels.max(1);
        let mono: Vec<f32> = if channels == 1 {
            samples.to_vec()
        } else {
            samples
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect()
        };
        let mono = resample::resample(&mono, sample_rate, DETECT_SAMPLE_RATE);
        if mono.is_empty() {
            return Ok(Detected::default());
        }

        let (mel, frames) = self.mel(&mono)?;
        let (beat_logits, downbeat_logits) = self.predict(&mel, frames)?;
        let beats = peaks(&beat_logits);
        let mut downbeats = peaks(&downbeat_logits);
        snap(&beats, &mut downbeats);

        Ok(Detected {
            beats_ms: beats.iter().map(|f| (f / FPS * 1000.0).round() as u32).collect(),
            downbeats_ms: downbeats.iter().map(|f| (f / FPS * 1000.0).round() as u32).collect(),
        })
    }

    /// The log-mel spectrogram: `[1, frames, 128]`, flattened, plus the frame count.
    fn mel(&mut self, mono: &[f32]) -> Result<(Vec<f32>, usize), BeatsError> {
        let mut out = self.mel.run(
            "audio_pcm",
            &[1, mono.len()],
            mono.to_vec(),
            &[&["mel_spectrogram"]],
        )?;
        let (shape, data) = out.remove(0);
        if shape.len() != 3 || shape[0] != 1 || shape[2] != MELS {
            return Err(BeatsError::Shape(format!("mel came back as {shape:?}")));
        }
        Ok((data, shape[1]))
    }

    /// Beat and downbeat logits, one per spectrogram frame.
    ///
    /// The model was trained on thirty-second windows, so a song is cut into
    /// overlapping chunks with the six frames at each edge — where the model has
    /// context on one side only — thrown away. The chunks are written **back to
    /// front**, which is what makes an overlapping region take the *earlier* chunk's
    /// answer: the earlier chunk has more music before the overlap and is the better
    /// informed of the two.
    fn predict(&mut self, mel: &[f32], frames: usize) -> Result<(Vec<f32>, Vec<f32>), BeatsError> {
        let mut beat = vec![f32::NEG_INFINITY; frames];
        let mut downbeat = vec![f32::NEG_INFINITY; frames];

        for start in starts(frames).into_iter().rev() {
            let (chunk, chunk_frames) = cut(mel, frames, start);
            // Two spellings of each output, because the exported models have gone by
            // both — and named rather than taken by position, since a model whose
            // outputs came back the other way round would swap beats and downbeats
            // silently.
            let mut outputs = self.beat.run(
                "spectrogram",
                &[1, chunk_frames, MELS],
                chunk,
                &[&["beat", "beat_logits"], &["downbeat", "downbeat_logits"]],
            )?;
            let downbeat_out = outputs.remove(1).1;
            let beat_out = outputs.remove(0).1;
            if beat_out.len() < chunk_frames || downbeat_out.len() < chunk_frames {
                return Err(BeatsError::Shape("the beat model came back short".into()));
            }
            let write_from = start + BORDER as i32;
            for (i, (b, d)) in beat_out[BORDER..chunk_frames - BORDER]
                .iter()
                .zip(downbeat_out[BORDER..chunk_frames - BORDER].iter())
                .enumerate()
            {
                let at = write_from + i as i32;
                if at >= 0 && (at as usize) < frames {
                    beat[at as usize] = *b;
                    downbeat[at as usize] = *d;
                }
            }
        }
        Ok((beat, downbeat))
    }
}

/// One loaded model, and the node ids its inputs and outputs are called by.
struct Loaded {
    model: Model,
    inputs: HashMap<String, NodeId>,
    outputs: HashMap<String, NodeId>,
}

impl Loaded {
    fn from_static(bytes: &'static [u8]) -> Result<Loaded, BeatsError> {
        Loaded::wrap(Model::load_static_slice(bytes).map_err(|e| BeatsError::Model(e.to_string()))?)
    }

    fn from_owned(bytes: Vec<u8>) -> Result<Loaded, BeatsError> {
        Loaded::wrap(Model::load(bytes).map_err(|e| BeatsError::Model(e.to_string()))?)
    }

    fn wrap(model: Model) -> Result<Loaded, BeatsError> {
        let named = |ids: &[NodeId]| -> HashMap<String, NodeId> {
            ids.iter()
                .filter_map(|id| Some((model.node_info(*id)?.name()?.to_string(), *id)))
                .collect()
        };
        let inputs = named(model.input_ids());
        let outputs = named(model.output_ids());
        Ok(Loaded { model, inputs, outputs })
    }

    fn output_id(&self, names: &[&str]) -> Result<NodeId, BeatsError> {
        names
            .iter()
            .find_map(|name| self.outputs.get(*name).copied())
            .ok_or_else(|| BeatsError::Shape(format!("no output called any of {names:?}")))
    }

    fn input_id(&self, name: &str) -> Result<NodeId, BeatsError> {
        self.inputs
            .get(name)
            .copied()
            .ok_or_else(|| BeatsError::Shape(format!("no input called {name}")))
    }

    /// Run for the named outputs, answering each one's shape and data in order.
    fn run(
        &mut self,
        input: &str,
        shape: &[usize],
        data: Vec<f32>,
        want: &[&[&str]],
    ) -> Result<Vec<(Vec<usize>, Vec<f32>)>, BeatsError> {
        let in_id = self.input_id(input)?;
        let out_ids: Vec<NodeId> =
            want.iter().map(|names| self.output_id(names)).collect::<Result<_, _>>()?;
        let value =
            Value::from_shape(shape, data).map_err(|e| BeatsError::Shape(e.to_string()))?;
        let outputs = self
            .model
            .run(vec![(in_id, (&value).into())], &out_ids, None)
            .map_err(|e| BeatsError::Run(e.to_string()))?;
        outputs
            .into_iter()
            .map(|value| {
                let tensor = value
                    .into_tensor::<f32>()
                    .ok_or_else(|| BeatsError::Shape("an output is not f32".into()))?;
                Ok((tensor.shape().to_vec(), tensor.to_vec()))
            })
            .collect()
    }
}

/// Where each chunk starts, in frames. Negative means the chunk is padded on the left.
fn starts(frames: usize) -> Vec<i32> {
    let mut out = Vec::new();
    let mut at = -(BORDER as i32);
    let limit = frames as i32 - BORDER as i32;
    while at < limit {
        out.push(at);
        at += STRIDE as i32;
    }
    // The last chunk is pulled back to end with the spectrogram rather than running
    // off it, so the end of a song is predicted with a full window of context instead
    // of against a few frames of zeros.
    if frames > STRIDE {
        if let Some(last) = out.last_mut() {
            *last = frames as i32 - (CHUNK - BORDER) as i32;
        }
    }
    if out.is_empty() {
        out.push(-(BORDER as i32));
    }
    out
}

/// One chunk of the spectrogram, zero-padded at the edges.
fn cut(mel: &[f32], frames: usize, start: i32) -> (Vec<f32>, usize) {
    let from = start.max(0) as usize;
    let to = ((start + CHUNK as i32).max(0) as usize).min(frames);
    let pad_left = (-start).max(0) as usize;
    let pad_right = (start + CHUNK as i32 - frames as i32).min(BORDER as i32).max(0) as usize;
    let chunk_frames = pad_left + to.saturating_sub(from) + pad_right;

    let mut data = vec![0.0f32; chunk_frames * MELS];
    for frame in from..to {
        let at = (pad_left + frame - from) * MELS;
        data[at..at + MELS].copy_from_slice(&mel[frame * MELS..frame * MELS + MELS]);
    }
    (data, chunk_frames)
}

/// Frames where the logits peak, as fractional frame indices.
///
/// A local maximum over a seven-frame window, above zero — which after a sigmoid is a
/// probability over a half — and then adjacent peaks merged at their running mean, so
/// a beat the model is confident about across two frames is one beat at the point
/// between them rather than two a fiftieth of a second apart.
fn peaks(logits: &[f32]) -> Vec<f64> {
    let mut found: Vec<usize> = Vec::new();
    for (i, value) in logits.iter().enumerate() {
        if *value <= 0.0 {
            continue;
        }
        let from = i.saturating_sub(3);
        let to = (i + 4).min(logits.len());
        if logits[from..to].iter().any(|other| other > value) {
            continue;
        }
        found.push(i);
    }
    merge(&found)
}

fn merge(found: &[usize]) -> Vec<f64> {
    let Some(&first) = found.first() else { return Vec::new() };
    let mut out = Vec::new();
    let mut at = first as f64;
    let mut count = 1.0f64;
    for &next in &found[1..] {
        let next = next as f64;
        if next - at <= 1.0 {
            count += 1.0;
            at += (next - at) / count;
        } else {
            out.push(at);
            at = next;
            count = 1.0;
        }
    }
    out.push(at);
    out
}

/// Put every downbeat onto the beat nearest it, then drop the duplicates.
///
/// The model predicts the two independently, so a downbeat can land a frame off the
/// beat it belongs to — and a "one" that is 20 ms away from the pulse is a grid whose
/// bar lines do not sit on its beat lines, which reads as a bug rather than as a
/// detection.
fn snap(beats: &[f64], downbeats: &mut Vec<f64>) {
    if beats.is_empty() || downbeats.is_empty() {
        return;
    }
    for downbeat in downbeats.iter_mut() {
        let at = beats.partition_point(|b| *b < *downbeat);
        let nearest = match (at.checked_sub(1).map(|i| beats[i]), beats.get(at).copied()) {
            (Some(before), Some(after)) => {
                if (*downbeat - before).abs() <= (after - *downbeat).abs() {
                    before
                } else {
                    after
                }
            }
            (Some(before), None) => before,
            (None, Some(after)) => after,
            (None, None) => continue,
        };
        *downbeat = nearest;
    }
    downbeats.sort_by(|a, b| a.total_cmp(b));
    downbeats.dedup();
}

// ── From beats to a grid ──────────────────────────────────────────────────────

/// One stretch of the beat grid, in the shape `Timeline::grid` holds.
///
/// Repeated here rather than imported for the reason [`crate::ltc::LtcRate`] is: this
/// crate depends on no pult crate, and the backend converts in one place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSegment {
    pub at_ms: u32,
    pub bpm: f32,
    pub beats_per_bar: u8,
}

/// How far a bar's measured tempo may differ from the segment it is in before a new
/// segment starts.
///
/// **Five percent, and the figure is set by the measurement rather than by taste.** One
/// bar holds three or four intervals and every beat is quantised to the model's 20 ms
/// frame, so a single bar's tempo is only good to about 1.5% — two bars of a *perfectly
/// steady* song can therefore read 3% apart. A threshold below that would split a song
/// that never changes tempo into a segment per bar, which is the failure this was
/// first written with. Above it, a real change is 25% (a half-time bridge) or is
/// gradual enough that a rallentando comes out as several runs anyway.
const NEW_SEGMENT_AT: f32 = 0.05;

/// Turn beats and downbeats into the segments an operator confirms into `grid`.
///
/// **A segment per tempo change, not per bar.** A song at 128 that never moves is one
/// segment, which is what somebody dragging events against it wants to see; one with a
/// half-time bridge is three. The bpm is the *median* inter-beat interval within the
/// segment rather than the mean, because a detector that missed one beat produces a
/// double-length interval and a mean would follow it.
///
/// **`at_ms` is on a downbeat**, so the grid's bar lines are the song's. A detection
/// with no downbeats at all still gives a grid — beats without a "one" — and calls the
/// bar four, which is the right guess to make and is why it is written down here
/// rather than in a panel.
pub fn grid_from_beats(beats_ms: &[u32], downbeats_ms: &[u32]) -> Vec<GridSegment> {
    if beats_ms.len() < 2 {
        return Vec::new();
    }
    let beats_per_bar = bar_length(beats_ms, downbeats_ms);

    // Segment boundaries are downbeats where there are any, so a tempo change lands on
    // a bar line rather than in the middle of one.
    let anchors: Vec<u32> = if downbeats_ms.is_empty() {
        vec![beats_ms[0]]
    } else {
        downbeats_ms.to_vec()
    };

    // **Two passes, and the second is what makes the tempo accurate.** One bar holds
    // three or four intervals, each quantised to the model's own 20 ms frame, which is
    // enough to say whether the tempo *changed* and nowhere near enough to say what it
    // is. So the first pass groups the bars into runs at one tempo, and the second
    // measures each run across its whole span — thirty bars of quantisation averaged
    // rather than three.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut coarse: Vec<Option<f32>> = Vec::with_capacity(anchors.len());
    for (i, anchor) in anchors.iter().enumerate() {
        let until = anchors.get(i + 1).copied().unwrap_or(u32::MAX);
        let within: Vec<u32> =
            beats_ms.iter().copied().filter(|b| *b >= *anchor && *b < until).collect();
        coarse.push(tempo_of(&within, beats_ms, *anchor));
    }
    for (i, tempo) in coarse.iter().enumerate() {
        let Some(tempo) = tempo else { continue };
        // Compared against the **median** of the run so far rather than against its
        // first bar: the first bar carries its own quantisation error, and a run judged
        // against it inherits that error for its whole length.
        match runs.last_mut() {
            Some((first, last)) if same_tempo(&coarse[*first..=*last], *tempo) => *last = i,
            _ => runs.push((i, i)),
        }
    }

    runs.into_iter()
        .filter_map(|(first, last)| {
            let from = anchors[first];
            let until = anchors.get(last + 1).copied().unwrap_or(u32::MAX);
            let within: Vec<u32> =
                beats_ms.iter().copied().filter(|b| *b >= from && *b < until).collect();
            let bpm = tempo_of(&within, beats_ms, from)?;
            Some(GridSegment { at_ms: from, bpm, beats_per_bar })
        })
        .collect()
}

/// Whether a bar's tempo belongs to the run already going.
fn same_tempo(run: &[Option<f32>], bar: f32) -> bool {
    let mut measured: Vec<f32> = run.iter().flatten().copied().collect();
    if measured.is_empty() {
        return false;
    }
    measured.sort_by(f32::total_cmp);
    let median = measured[measured.len() / 2];
    median > 0.0 && (median - bar).abs() / median < NEW_SEGMENT_AT
}

/// The tempo of a stretch of beats: the median interval, corrected by the span.
///
/// **A single interval cannot answer this accurately**, and it took a real song to see
/// why: the model works at 50 frames a second, so every beat it reports is quantised to
/// the nearest 20 ms. At 128 bpm a beat is 468.75 ms, which lands on 460 or 480 — and a
/// median of those is 130.4 bpm or 125, either of which walks a whole bar out of step
/// inside a minute.
///
/// So the median is only used to say *how many beats apart* the ends of the stretch
/// are, and the tempo comes from the **span between the first and last beat divided by
/// that count** — which averages the quantisation over the whole stretch and is exact to
/// well under a bpm across thirty bars. The median still does the rejecting, because a
/// beat the detector missed doubles one interval and a span alone cannot see that.
fn tempo_of(within: &[u32], all: &[u32], anchor: u32) -> Option<f32> {
    let mut gaps: Vec<u32> = within.windows(2).map(|w| w[1] - w[0]).collect();
    if gaps.is_empty() {
        // A stretch with one beat in it: the interval to the next beat anywhere is the
        // only evidence there is, quantisation and all.
        let next = all.iter().find(|b| **b > anchor)?;
        return match next - anchor {
            0 => None,
            gap => Some(60_000.0 / gap as f32),
        };
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    if median == 0 {
        return None;
    }
    let first = *within.first()?;
    let last = *within.last()?;
    let span = last.saturating_sub(first);
    if span == 0 {
        return Some(60_000.0 / median as f32);
    }
    // How many beats the span *holds*, which is not `span / median`: the median of
    // quantised intervals is itself biased — at 128 bpm the intervals land on 460 and
    // 480 and the median is 460, so dividing the span by it invents two extra beats and
    // reads 130. The count is the number of intervals, with each one asked how many
    // beats it covers so that a beat the detector missed counts as the two it stands
    // for rather than as one long one.
    let beats: u32 = within
        .windows(2)
        .map(|pair| (((pair[1] - pair[0]) as f32 / median as f32).round() as u32).max(1))
        .sum();
    Some(60_000.0 * beats as f32 / span as f32)
}

/// How many beats are in a bar, from how far apart the downbeats are.
///
/// The mode of the beat counts between consecutive downbeats rather than the mean: a
/// song in four with one dropped bar of two is in four, and a mean would call it 3.9
/// and round to something nobody plays in.
fn bar_length(beats_ms: &[u32], downbeats_ms: &[u32]) -> u8 {
    if downbeats_ms.len() < 2 {
        return 4;
    }
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for pair in downbeats_ms.windows(2) {
        let beats = beats_ms.iter().filter(|b| **b >= pair[0] && **b < pair[1]).count();
        if (1..=16).contains(&beats) {
            *counts.entry(beats).or_default() += 1;
        }
    }
    // Ties go to the larger count, so a song that is half four and half two is read as
    // four — the reading in which every downbeat found is still on a bar line.
    counts
        .into_iter()
        .max_by_key(|(beats, seen)| (*seen, *beats))
        .map(|(beats, _)| beats as u8)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests;
