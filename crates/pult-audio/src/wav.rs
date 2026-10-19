//! A clip, as a file a node keeps in its own flash.
//!
//! An OpenHaunt node plays RIFF WAVE in one of two encodings: 16-bit PCM (format tag
//! 1) and IMA ADPCM (format tag 0x11, Microsoft's block layout, which is what `ffmpeg
//! -c:a adpcm_ima_wav` writes). Both are written here, and **written deterministically**:
//! the node names a clip by the sha256 of its bytes, so the same region of the same file
//! has to come out as the same bytes on every station and after every restart, or each
//! of them would upload its own copy.
//!
//! ADPCM is four bits a sample, so a node's ten megabytes hold about ten minutes at
//! 32 kHz mono. It has a noise floor you can hear under a quiet sting, which is why the
//! short clips go as PCM instead. That choice belongs to whoever calls this, not here.
//!
//! The decoder is the firmware's arithmetic in Rust. It exists to test the encoder:
//! a file that decodes here to within a few percent of what went in is a file the node
//! will play the same way, because the two decoders are one table and one shift.

use thiserror::Error;

/// Bytes per ADPCM block per channel. A block is a header and then groups of eight
/// samples, so 1024 bytes of mono is 2041 frames. The firmware takes blocks up to 2048
/// bytes, which this stays inside for stereo too.
pub const ADPCM_BLOCK_PER_CHANNEL: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Pcm16,
    ImaAdpcm,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WavError {
    #[error("this is not a RIFF WAVE file")]
    NotWave,
    #[error("the file has no {0} chunk")]
    Missing(&'static str),
    #[error("only PCM16 and IMA ADPCM, mono or stereo")]
    Unsupported,
}

/// A decoded file, as `i16` interleaved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav {
    pub encoding: Encoding,
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

impl Wav {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

/// Interleaved f32 to a WAV file. One or two channels; anything else is the caller's
/// mistake and is refused rather than truncated.
pub fn encode(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    encoding: Encoding,
) -> Result<Vec<u8>, WavError> {
    if !(1..=2).contains(&channels) {
        return Err(WavError::Unsupported);
    }
    let pcm: Vec<i16> = samples.iter().map(|s| to_i16(*s)).collect();
    Ok(match encoding {
        Encoding::Pcm16 => pcm16(&pcm, channels, sample_rate),
        Encoding::ImaAdpcm => adpcm(&pcm, channels, sample_rate),
    })
}

/// Rounded, not truncated, and clamped: a sum that went over 1 is a clipped sample,
/// never a wrapped one.
fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

fn pcm16(pcm: &[i16], channels: u16, sample_rate: u32) -> Vec<u8> {
    let block_align = 2 * channels;
    let mut data = Vec::with_capacity(pcm.len() * 2);
    for sample in pcm {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    let mut fmt = Vec::with_capacity(16);
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    riff(&[(b"fmt ", fmt), (b"data", data)])
}

fn adpcm(pcm: &[i16], channels: u16, sample_rate: u32) -> Vec<u8> {
    let ch = channels as usize;
    let block_align = ADPCM_BLOCK_PER_CHANNEL * ch;
    let per_block = frames_per_block(block_align, ch);
    let frames = pcm.len() / ch;

    let mut data = Vec::new();
    let mut state = [Ima::default(); 2];
    let mut start = 0;
    while start < frames {
        let take = per_block.min(frames - start);
        // The header carries the block's first sample exactly, and the step index the
        // encoder had reached, so every block decodes on its own: that is what lets the
        // node seek to a late start without decoding from the beginning.
        for (c, st) in state.iter_mut().enumerate().take(ch) {
            let first = pcm[start * ch + c];
            st.predicted = first as i32;
            data.extend_from_slice(&first.to_le_bytes());
            data.push(st.index as u8);
            data.push(0);
        }
        // Then groups of eight samples per channel, four bytes each, low nibble first.
        // A short last block is padded with the last sample, and the `fact` chunk says
        // how many frames are real.
        let mut frame = 1;
        while frame < take {
            for (c, st) in state.iter_mut().enumerate().take(ch) {
                let mut group = [0u8; 4];
                for i in 0..8 {
                    let at = (start + frame + i).min(start + take - 1);
                    let nibble = st.encode(pcm[at * ch + c]);
                    group[i >> 1] |= if i & 1 == 1 { nibble << 4 } else { nibble };
                }
                data.extend_from_slice(&group);
            }
            frame += 8;
        }
        start += take;
    }

    let mut fmt = Vec::with_capacity(20);
    fmt.extend_from_slice(&0x11u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    let bytes_per_second = (sample_rate as u64 * block_align as u64 / per_block as u64) as u32;
    fmt.extend_from_slice(&bytes_per_second.to_le_bytes());
    fmt.extend_from_slice(&(block_align as u16).to_le_bytes());
    fmt.extend_from_slice(&4u16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&(per_block as u16).to_le_bytes());
    riff(&[(b"fmt ", fmt), (b"fact", (frames as u32).to_le_bytes().to_vec()), (b"data", data)])
}

fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = b"WAVE".to_vec();
    for (id, bytes) in chunks {
        body.extend_from_slice(*id);
        body.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        body.extend_from_slice(bytes);
        if bytes.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Frames in one ADPCM block: the header's sample, and eight per group after it.
pub fn frames_per_block(block_align: usize, channels: usize) -> usize {
    1 + (block_align - 4 * channels) / (4 * channels) * 8
}

// ── IMA ADPCM ─────────────────────────────────────────────────────────────────

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60,
    66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
    408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707,
    1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132,
    7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623,
    27086, 29794, 32767,
];

const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Debug, Clone, Copy, Default)]
struct Ima {
    predicted: i32,
    index: i32,
}

impl Ima {
    /// One nibble out, and the state moved on exactly as the decoder will move it —
    /// which is what keeps the encoder's prediction and the node's in step.
    fn encode(&mut self, sample: i16) -> u8 {
        let step = STEPS[self.index as usize];
        let mut diff = sample as i32 - self.predicted;
        let mut nibble = 0u8;
        if diff < 0 {
            nibble = 8;
            diff = -diff;
        }
        let mut threshold = step;
        for bit in [4u8, 2, 1] {
            if diff >= threshold {
                nibble |= bit;
                diff -= threshold;
            }
            threshold >>= 1;
        }
        self.decode(nibble);
        nibble
    }

    fn decode(&mut self, nibble: u8) -> i16 {
        let step = STEPS[self.index as usize];
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        self.predicted += if nibble & 8 != 0 { -diff } else { diff };
        self.predicted = self.predicted.clamp(-32768, 32767);
        self.index = (self.index + INDEX[nibble as usize]).clamp(0, 88);
        self.predicted as i16
    }
}

// ── Reading ───────────────────────────────────────────────────────────────────

/// A WAV file back to samples. The two encodings this module writes, and nothing else.
pub fn decode(bytes: &[u8]) -> Result<Wav, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWave);
    }
    let mut at = 12;
    let mut fmt: Option<&[u8]> = None;
    let mut fact: Option<u32> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap_or([0; 4])) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"fact" if body.len() >= 4 => {
                fact = Some(u32::from_le_bytes(body[0..4].try_into().unwrap_or([0; 4])))
            }
            b"data" => {
                let fmt = fmt.ok_or(WavError::Missing("fmt"))?;
                return read_data(fmt, fact, body);
            }
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    Err(WavError::Missing("data"))
}

fn read_data(fmt: &[u8], fact: Option<u32>, data: &[u8]) -> Result<Wav, WavError> {
    if fmt.len() < 16 {
        return Err(WavError::Unsupported);
    }
    let u16_at = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let tag = u16_at(0);
    let channels = u16_at(2);
    let sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let block_align = u16_at(12) as usize;
    if !(1..=2).contains(&channels) {
        return Err(WavError::Unsupported);
    }
    let ch = channels as usize;
    match tag {
        1 => Ok(Wav {
            encoding: Encoding::Pcm16,
            channels,
            sample_rate,
            samples: data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect(),
        }),
        0x11 => {
            let mut samples = Vec::new();
            for block in data.chunks(block_align) {
                if block.len() < 4 * ch {
                    break;
                }
                let mut state = [Ima::default(); 2];
                let mut frames = vec![vec![0i16; ch]];
                for (c, st) in state.iter_mut().enumerate().take(ch) {
                    let h = &block[4 * c..4 * c + 4];
                    st.predicted = i16::from_le_bytes([h[0], h[1]]) as i32;
                    st.index = (h[2] as i32).min(88);
                    frames[0][c] = st.predicted as i16;
                }
                for group in block[4 * ch..].chunks_exact(4 * ch) {
                    let mut eight = vec![vec![0i16; ch]; 8];
                    for (c, st) in state.iter_mut().enumerate().take(ch) {
                        for (i, frame) in eight.iter_mut().enumerate() {
                            let byte = group[4 * c + (i >> 1)];
                            let nibble = if i & 1 == 1 { byte >> 4 } else { byte & 0x0f };
                            frame[c] = st.decode(nibble);
                        }
                    }
                    frames.extend(eight);
                }
                samples.extend(frames.into_iter().flatten());
            }
            if let Some(frames) = fact {
                samples.truncate(frames as usize * ch);
            }
            Ok(Wav { encoding: Encoding::ImaAdpcm, channels, sample_rate, samples })
        }
        _ => Err(WavError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, channels: u16, rate: u32, hz: f32, level: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let v = (n as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * level;
                (0..channels).map(move |c| if c == 0 { v } else { -v })
            })
            .collect()
    }

    /// Signal over error, in decibels, against what went in.
    fn snr(input: &[f32], output: &[i16]) -> f64 {
        let (mut signal, mut noise) = (0.0f64, 0.0f64);
        for (a, b) in input.iter().zip(output) {
            let a = (*a as f64) * 32767.0;
            signal += a * a;
            noise += (a - *b as f64).powi(2);
        }
        10.0 * (signal / noise.max(1e-9)).log10()
    }

    #[test]
    fn pcm16_comes_back_exactly() {
        let input = sine(4_800, 2, 48_000, 440.0, 0.5);
        let wav = decode(&encode(&input, 2, 48_000, Encoding::Pcm16).unwrap()).unwrap();
        assert_eq!(wav.encoding, Encoding::Pcm16);
        assert_eq!((wav.channels, wav.sample_rate, wav.frames()), (2, 48_000, 4_800));
        let expected: Vec<i16> = input.iter().map(|s| to_i16(*s)).collect();
        assert_eq!(wav.samples, expected);
    }

    /// IMA ADPCM is lossy, and a sine at a sensible level comes back far above any
    /// noise a scare would be heard against. The figure is the codec's, not ours to
    /// tune: four bits a sample buys roughly this much.
    #[test]
    fn adpcm_comes_back_close_in_mono_and_stereo() {
        for channels in [1u16, 2] {
            let input = sine(32_000, channels, 32_000, 330.0, 0.6);
            let bytes = encode(&input, channels, 32_000, Encoding::ImaAdpcm).unwrap();
            let wav = decode(&bytes).unwrap();
            assert_eq!(wav.encoding, Encoding::ImaAdpcm);
            assert_eq!(wav.frames(), 32_000, "the fact chunk trims the padded last block");
            let ratio = snr(&input, &wav.samples);
            assert!(ratio > 25.0, "{channels} channel(s): {ratio:.1} dB");
        }
    }

    /// A quarter of PCM's size, near enough: four bits against sixteen, plus a header
    /// per block. Which is the whole reason a long bed is not sent as PCM.
    #[test]
    fn adpcm_is_a_quarter_of_the_size() {
        let input = sine(320_000, 1, 32_000, 330.0, 0.6);
        let pcm = encode(&input, 1, 32_000, Encoding::Pcm16).unwrap().len() as f64;
        let ima = encode(&input, 1, 32_000, Encoding::ImaAdpcm).unwrap().len() as f64;
        assert!((ima / pcm - 0.25).abs() < 0.01, "{}", ima / pcm);
    }

    /// The firmware refuses a block size that is not a header plus whole groups, and
    /// anything over 2048 bytes. Both layouts this writes are inside that.
    #[test]
    fn the_block_layout_is_one_the_node_accepts() {
        for channels in [1usize, 2] {
            let align = ADPCM_BLOCK_PER_CHANNEL * channels;
            assert!(align <= 2048);
            assert_eq!((align - 4 * channels) % (4 * channels), 0);
        }
        assert_eq!(frames_per_block(1024, 1), 2041);
        assert_eq!(frames_per_block(2048, 2), 2041);
    }

    /// The node names a clip by the sha of its bytes, so the same samples have to be
    /// the same bytes every time.
    #[test]
    fn encoding_is_deterministic() {
        let input = sine(10_000, 1, 32_000, 200.0, 0.3);
        assert_eq!(
            encode(&input, 1, 32_000, Encoding::ImaAdpcm).unwrap(),
            encode(&input, 1, 32_000, Encoding::ImaAdpcm).unwrap()
        );
    }

    #[test]
    fn more_than_two_channels_is_refused() {
        assert_eq!(encode(&[0.0; 6], 3, 48_000, Encoding::Pcm16), Err(WavError::Unsupported));
    }

    #[test]
    fn a_file_shorter_than_a_block_still_decodes() {
        let input = sine(100, 1, 32_000, 200.0, 0.3);
        let wav = decode(&encode(&input, 1, 32_000, Encoding::ImaAdpcm).unwrap()).unwrap();
        assert_eq!(wav.frames(), 100);
    }
}
