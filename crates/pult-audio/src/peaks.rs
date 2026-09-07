//! What a waveform looks like, small enough to send to a tablet.
//!
//! A three-minute song at 48 kHz is nine million samples per channel. A panel drawing
//! it wants a few hundred columns. So the station reduces it once — **min and max per
//! bin**, not an average — and every browser in the building fetches the reduction.
//!
//! Min and max rather than a mean or an RMS because a waveform is read as an
//! *envelope*: a snare hit that peaks for two milliseconds is one bin's maximum and is
//! invisible in that bin's average, and a picture somebody drags events against has to
//! show the transients they are dragging against.
//!
//! **A fixed resolution rather than one per zoom level.** [`BINS_PER_SECOND`] is a
//! hundred, which is finer than any zoom a panel offers on a screen — one bin per
//! column at ten seconds across a thousand pixels — so a zoomed-out view folds bins
//! together in the browser and a zoomed-in one runs out of pixels before it runs out
//! of bins. A pyramid of resolutions would be more data for a picture nobody looks at
//! that closely.
//!
//! **The codec stores i16 and not f32.** A waveform column is a few pixels tall; the
//! sixteenth bit of its height is not a thing anybody can see, and halving the asset
//! halves what the tablet at the back of the room downloads. Samples outside ±1 are
//! clamped rather than scaled, because a file that is already clipping should draw as
//! clipping.

use crate::PeaksError;

/// Bins per second of audio. See the module header for why it is fixed.
pub const BINS_PER_SECOND: u32 = 100;

/// The magic at the top of an encoded peaks file: `PLPK`.
const MAGIC: &[u8; 4] = b"PLPK";
/// The format version. Bumped when the layout below changes, and read before
/// anything else — a peaks asset is content-addressed and cached for ever, so a
/// station reading a file written by a newer console has to say so rather than draw
/// something wrong.
const VERSION: u16 = 1;

/// A reduced waveform: the extremes of every bin, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Peaks {
    /// The rate the samples were reduced at, which is what turns a bin index into a
    /// position. Kept rather than assumed: a file that arrives at 44.1 kHz and one at
    /// 48 kHz have different numbers of samples in a bin.
    pub sample_rate: u32,
    pub samples_per_bin: u32,
    /// `(min, max)` per bin, in `[-1, 1]`.
    pub bins: Vec<(f32, f32)>,
}

impl Peaks {
    /// Reduce interleaved f32 samples, downmixing to mono on the way.
    ///
    /// Mono because a waveform behind a timeline is a picture of the song rather than
    /// of the mix: two channels drawn over each other are the same shape twice, and
    /// drawn apart they halve the height of the thing being dragged against.
    pub fn compute(samples: &[f32], channels: usize, sample_rate: u32) -> Peaks {
        let channels = channels.max(1);
        let samples_per_bin = (sample_rate / BINS_PER_SECOND).max(1);
        let frames = samples.len() / channels;
        let mut bins = Vec::with_capacity(frames / samples_per_bin as usize + 1);

        let mut frame = 0usize;
        while frame < frames {
            let end = (frame + samples_per_bin as usize).min(frames);
            let mut low = f32::MAX;
            let mut high = f32::MIN;
            for f in frame..end {
                let mut sum = 0.0f32;
                for c in 0..channels {
                    sum += samples[f * channels + c];
                }
                let mono = sum / channels as f32;
                low = low.min(mono);
                high = high.max(mono);
            }
            bins.push((low, high));
            frame = end;
        }

        Peaks { sample_rate, samples_per_bin, bins }
    }

    /// How long the audio this was reduced from is.
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        self.bins.len() as u64 * self.samples_per_bin as u64 * 1000 / self.sample_rate as u64
    }

    /// The file the station stores and the browser fetches.
    ///
    /// Little-endian throughout, which is what every machine this runs on is; the
    /// magic and the version are what make a wrong file say so rather than decode as
    /// noise.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(18 + self.bins.len() * 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&self.samples_per_bin.to_le_bytes());
        out.extend_from_slice(&(self.bins.len() as u32).to_le_bytes());
        for (low, high) in &self.bins {
            out.extend_from_slice(&quantise(*low).to_le_bytes());
            out.extend_from_slice(&quantise(*high).to_le_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Peaks, PeaksError> {
        if bytes.len() < 18 || &bytes[0..4] != MAGIC {
            return Err(PeaksError::NotPeaks);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != VERSION {
            return Err(PeaksError::Version(version));
        }
        let sample_rate = u32::from_le_bytes(bytes[6..10].try_into().unwrap_or_default());
        let samples_per_bin = u32::from_le_bytes(bytes[10..14].try_into().unwrap_or_default());
        let count = u32::from_le_bytes(bytes[14..18].try_into().unwrap_or_default()) as usize;
        if bytes.len() < 18 + count * 4 {
            return Err(PeaksError::Truncated);
        }
        let mut bins = Vec::with_capacity(count);
        for i in 0..count {
            let at = 18 + i * 4;
            let low = i16::from_le_bytes([bytes[at], bytes[at + 1]]);
            let high = i16::from_le_bytes([bytes[at + 2], bytes[at + 3]]);
            bins.push((dequantise(low), dequantise(high)));
        }
        Ok(Peaks { sample_rate, samples_per_bin, bins })
    }
}

/// `[-1, 1]` to a signed 16-bit column height. Clamped rather than scaled: see the
/// module header.
fn quantise(value: f32) -> i16 {
    (value.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn dequantise(value: i16) -> f32 {
    value as f32 / i16::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ramp: every bin's extremes are its own first and last sample, so the
    /// reduction can be checked against arithmetic rather than against itself.
    #[test]
    fn a_bin_holds_the_extremes_of_what_it_covers() {
        let samples: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        let peaks = Peaks::compute(&samples, 1, 100_000);
        // 100_000 / 100 bins a second = 1000 samples a bin, so this is one bin.
        assert_eq!(peaks.bins.len(), 1);
        assert_eq!(peaks.bins[0].0, 0.0);
        assert!((peaks.bins[0].1 - 0.999).abs() < 1e-6);
    }

    #[test]
    fn stereo_is_downmixed_before_it_is_reduced() {
        // Two channels in opposition: the mono sum is silence, and a waveform of a
        // song mixed that way should draw as one.
        let samples: Vec<f32> = (0..200).flat_map(|_| [1.0f32, -1.0f32]).collect();
        let peaks = Peaks::compute(&samples, 2, 10_000);
        assert!(peaks.bins.iter().all(|(low, high)| low.abs() < 1e-6 && high.abs() < 1e-6));
    }

    #[test]
    fn a_short_file_still_gets_its_last_partial_bin() {
        let samples = vec![0.5f32; 10];
        let peaks = Peaks::compute(&samples, 1, 48_000);
        assert_eq!(peaks.bins.len(), 1, "ten samples is a tenth of a bin and is still drawn");
        assert_eq!(peaks.bins[0], (0.5, 0.5));
    }

    #[test]
    fn the_codec_round_trips_to_the_quantiser_and_no_further() {
        let peaks = Peaks {
            sample_rate: 48_000,
            samples_per_bin: 480,
            bins: vec![(-1.0, 1.0), (-0.25, 0.5), (0.0, 0.0)],
        };
        let back = Peaks::decode(&peaks.encode()).unwrap();
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.samples_per_bin, 480);
        assert_eq!(back.bins.len(), 3);
        for (before, after) in peaks.bins.iter().zip(back.bins.iter()) {
            assert!((before.0 - after.0).abs() < 1.0 / 32_000.0);
            assert!((before.1 - after.1).abs() < 1.0 / 32_000.0);
        }
    }

    /// A file that is already clipping draws as clipping rather than being rescaled
    /// into range, which would make an overloaded stem look well behaved.
    #[test]
    fn samples_past_full_scale_are_clamped() {
        let peaks = Peaks { sample_rate: 48_000, samples_per_bin: 480, bins: vec![(-4.0, 9.0)] };
        let back = Peaks::decode(&peaks.encode()).unwrap();
        assert!((back.bins[0].0 + 1.0).abs() < 1e-4);
        assert!((back.bins[0].1 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn anything_that_is_not_a_peaks_file_says_so() {
        assert!(matches!(Peaks::decode(b"not this"), Err(PeaksError::NotPeaks)));
        let mut bytes = Peaks { sample_rate: 1, samples_per_bin: 1, bins: vec![(0.0, 0.0)] }.encode();
        bytes[4] = 99;
        assert!(matches!(Peaks::decode(&bytes), Err(PeaksError::Version(_))));
        let short = &Peaks { sample_rate: 1, samples_per_bin: 1, bins: vec![(0.0, 0.0); 4] }
            .encode()[..20];
        assert!(matches!(Peaks::decode(short), Err(PeaksError::Truncated)));
    }

    #[test]
    fn the_duration_comes_off_the_bins_and_the_rate() {
        let peaks = Peaks { sample_rate: 48_000, samples_per_bin: 480, bins: vec![(0.0, 0.0); 300] };
        assert_eq!(peaks.duration_ms(), 3_000);
    }
}
