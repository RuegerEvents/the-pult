//! Changing a sample rate, twice, for two reasons.
//!
//! **The detector's front end wants 22 050 Hz** and a file arrives at whatever it was
//! made at, so a song is resampled once before inference. That one is offline and can
//! afford a windowed sinc.
//!
//! **A chase wants to run 1.5% fast** for a second or two, which is the same operation
//! at a ratio that keeps changing. That one is in the audio callback's path and has to
//! be cheap and stateless in the only sense that matters: given a fractional read
//! position, produce a sample.
//!
//! So there are two functions rather than one general resampler, and no dependency:
//! `rubato` would do the first beautifully and is the wrong shape for the second, and
//! carrying it for one of the two would leave the other written here anyway.

/// A windowed-sinc resample of a whole buffer. Offline: this walks the input once per
/// output sample and is not for a callback.
///
/// The window is Blackman over [`TAPS`] input samples either side, and the sinc is
/// cut off just below Nyquist of the *lower* of the two rates — which is what stops a
/// downsample folding the top octave back over the music, and is the only part of this
/// that a naive implementation gets wrong.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = if ratio < 1.0 { ratio } else { 1.0 } * 0.94;
    let out_len = (samples.len() as f64 * ratio).round() as usize;
    // A downsample needs a wider window in input samples to cover the same number of
    // cycles of the (lower) cutoff, or the filter it is applying is not the filter it
    // computed.
    let half = (TAPS as f64 / cutoff).ceil() as isize;

    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let centre = i as f64 / ratio;
        let base = centre.floor() as isize;
        let mut sum = 0.0f64;
        let mut weight = 0.0f64;
        for tap in (base - half)..=(base + half) {
            if tap < 0 || tap as usize >= samples.len() {
                continue;
            }
            let x = (tap as f64 - centre) * cutoff;
            let w = sinc(x) * blackman((tap as f64 - centre) / half as f64);
            sum += samples[tap as usize] as f64 * w;
            weight += w;
        }
        // Normalised by the weights actually used, so the first and last few samples —
        // where half the window is off the end of the buffer — do not fade in and out.
        out.push(if weight.abs() > 1e-12 { (sum / weight) as f32 } else { 0.0 });
    }
    out
}

/// How many cycles of the cutoff either side. Sixteen is well past what a beat
/// detector can tell from thirty-two, and this runs once per file.
const TAPS: usize = 16;

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        return 1.0;
    }
    let pi_x = std::f64::consts::PI * x;
    pi_x.sin() / pi_x
}

fn blackman(t: f64) -> f64 {
    if t.abs() >= 1.0 {
        return 0.0;
    }
    let a = std::f64::consts::PI * (t + 1.0);
    0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
}

/// One sample at a fractional position, linearly interpolated.
///
/// What the varispeed uses. Linear rather than sinc, and this is a decision rather
/// than laziness: the ratio is within 2% of one, so consecutive output samples come
/// from consecutive input samples and the interpolation error sits about 40 dB down at
/// the top of the band and far lower everywhere a stem has energy. A sinc in the
/// callback would cost sixty multiplies a sample to fix something nobody in the room
/// can hear, and the chase is over inside a second anyway.
pub fn sample_at(samples: &[f32], position: f64) -> f32 {
    if samples.is_empty() || position < 0.0 {
        return 0.0;
    }
    let base = position.floor() as usize;
    if base + 1 >= samples.len() {
        return samples.get(base).copied().unwrap_or(0.0);
    }
    let fraction = (position - base as f64) as f32;
    samples[base] * (1.0 - fraction) + samples[base + 1] * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        (0..(rate as f64 * seconds) as usize)
            .map(|i| {
                (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32
            })
            .collect()
    }

    /// A tone comes out as the same tone at the new rate, which is checked by counting
    /// its zero crossings rather than by comparing samples — the phase is not what a
    /// resampler promises.
    #[test]
    fn a_tone_keeps_its_frequency() {
        let input = tone(440.0, 48_000, 1.0);
        let output = resample(&input, 48_000, 22_050);
        assert!((output.len() as i32 - 22_050).abs() < 4);

        let crossings = output
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        assert!((crossings as i32 - 440).abs() <= 2, "{crossings} crossings, wanted about 440");
    }

    /// The band above the new Nyquist has to be *removed*, not folded down. A 9 kHz
    /// tone resampled to 22 050 without a filter comes back as 2 kHz, loudly.
    #[test]
    fn a_downsample_does_not_fold_the_top_octave_back_over_the_music() {
        let input = tone(15_000.0, 48_000, 0.25);
        let output = resample(&input, 48_000, 22_050);
        let energy: f32 = output.iter().map(|s| s * s).sum::<f32>() / output.len() as f32;
        assert!(energy < 0.01, "the tone came back as something audible: {energy}");
    }

    #[test]
    fn the_same_rate_is_the_same_samples() {
        let input = tone(1_000.0, 48_000, 0.01);
        assert_eq!(resample(&input, 48_000, 48_000), input);
    }

    #[test]
    fn a_fractional_read_is_between_its_neighbours() {
        let samples = [0.0f32, 1.0, 0.0];
        assert_eq!(sample_at(&samples, 0.0), 0.0);
        assert_eq!(sample_at(&samples, 0.5), 0.5);
        assert_eq!(sample_at(&samples, 1.0), 1.0);
        assert_eq!(sample_at(&samples, 1.25), 0.75);
        assert_eq!(sample_at(&samples, 9.0), 0.0, "past the end is silence, not a panic");
        assert_eq!(sample_at(&samples, -1.0), 0.0);
    }
}
