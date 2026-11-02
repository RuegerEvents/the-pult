//! A timeline's sound for a page to listen to while somebody programs: the chunks it
//! is sent in, and their codec.
//!
//! **A monitor, never an output and never a clock.** The show's sound is the station's
//! device, measured off its own callback; a page that listens is told where in show
//! time each chunk belongs and schedules it there, and nothing it does reaches the
//! show. So the format carries the moment its first frame belongs to, in show
//! milliseconds, and nothing that would let a page steer anything.
//!
//! **PCM, little-endian `i16`, 48 kHz stereo.** About 1.5 Mbit/s per listening tab,
//! which a venue LAN does not notice, and decodable in a browser with nothing but a
//! `DataView` — Opus would be a tenth of the size and need an encoder in this build and
//! WebCodecs in the page, which Safari on an older iPad has not got. One reader in each
//! language, like `peaks`, and it earns that by being a header and samples with no
//! arithmetic in it to drift: `frontend/src/lib/monitor.ts` is the other.
//!
//! ```text
//! magic "PLMN" | u16 version | u16 channels | u32 sample rate | f64 at_ms | u32 frames
//! | u16 flags | frames × channels × i16, interleaved
//! ```
//!
//! Flag 1 is *loading*: a file the chunk covers is still being fetched or decoded on
//! the rendering station, so the page can say why it hears less than the timeline plays.

use thiserror::Error;

pub const MAGIC: &[u8; 4] = b"PLMN";
pub const VERSION: u16 = 1;
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// How much show time one chunk covers. Short enough that a locate is heard in step
/// within a chunk, long enough that a page schedules twenty buffers a second rather
/// than a hundred.
pub const CHUNK_MS: u64 = 50;
/// How far ahead of the show clock a chunk is rendered and sent. The page needs each
/// one before its moment arrives; a quarter of a second covers a busy tablet's
/// socket, and a locate inside it is heard late by at most that much.
pub const LEAD_MS: u64 = 250;
const HEADER: usize = 4 + 2 + 2 + 4 + 8 + 4 + 2;
const LOADING: u16 = 1;

/// Frames in one chunk at [`SAMPLE_RATE`].
pub const fn chunk_frames() -> usize {
    (SAMPLE_RATE as u64 * CHUNK_MS / 1_000) as usize
}

/// One chunk: interleaved samples, and the show millisecond its first frame belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub at_ms: f64,
    pub channels: usize,
    pub sample_rate: u32,
    /// A file this chunk covers is still loading on the station, and is not in it.
    pub loading: bool,
    pub samples: Vec<f32>,
}

#[derive(Debug, Error, PartialEq)]
pub enum ChunkError {
    #[error("not a monitor chunk")]
    Magic,
    #[error("monitor chunk version {0}, and this build reads {VERSION}")]
    Version(u16),
    #[error("the chunk is shorter than it says")]
    Short,
}

pub fn encode(chunk: &Chunk) -> Vec<u8> {
    let channels = chunk.channels.max(1);
    let frames = chunk.samples.len() / channels;
    let mut out = Vec::with_capacity(HEADER + frames * channels * 2);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(channels as u16).to_le_bytes());
    out.extend_from_slice(&chunk.sample_rate.to_le_bytes());
    out.extend_from_slice(&chunk.at_ms.to_le_bytes());
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    out.extend_from_slice(&(if chunk.loading { LOADING } else { 0 }).to_le_bytes());
    for sample in &chunk.samples[..frames * channels] {
        let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<Chunk, ChunkError> {
    if bytes.len() < HEADER {
        return Err(ChunkError::Short);
    }
    if &bytes[..4] != MAGIC {
        return Err(ChunkError::Magic);
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let version = u16_at(4);
    if version != VERSION {
        return Err(ChunkError::Version(version));
    }
    let channels = u16_at(6) as usize;
    let sample_rate = u32_at(8);
    let at_ms = f64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let frames = u32_at(20) as usize;
    let loading = u16_at(24) & LOADING != 0;
    let body = &bytes[HEADER..];
    if body.len() < frames * channels * 2 {
        return Err(ChunkError::Short);
    }
    let samples = body[..frames * channels * 2]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
        .collect();
    Ok(Chunk { at_ms, channels, sample_rate, loading, samples })
}

/// The chunks due between where the stream has got to and `now + LEAD_MS`, as the
/// show millisecond each starts at. `next` is where the stream has got to; `None` is a
/// stream just opened, which starts a lead ahead rather than in the past. A stream that
/// fell behind — a station busy for a second — skips to the lead rather than sending
/// chunks the page would be too late to play.
pub fn due(next: Option<u64>, now_ms: u64) -> (Vec<u64>, u64) {
    let horizon = now_ms + LEAD_MS;
    let mut at = match next {
        Some(at) if at + CHUNK_MS > now_ms => at,
        _ => now_ms + LEAD_MS / 2,
    };
    let mut out = Vec::new();
    while at < horizon {
        out.push(at);
        at += CHUNK_MS;
    }
    (out, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_comes_back_as_it_went() {
        let chunk = Chunk { at_ms: 12_345.5, channels: 2, sample_rate: SAMPLE_RATE, loading: true, samples: vec![0.0, 1.0, -1.0, 0.5] };
        let back = decode(&encode(&chunk)).unwrap();
        assert_eq!(back.at_ms, 12_345.5);
        assert_eq!(back.channels, 2);
        assert!(back.loading);
        assert_eq!(back.samples.len(), 4);
        for (a, b) in chunk.samples.iter().zip(&back.samples) {
            assert!((a - b).abs() < 1.0 / 16_000.0, "{a} {b}");
        }
    }

    #[test]
    fn what_is_not_a_chunk_is_refused() {
        assert_eq!(decode(b"PLMN"), Err(ChunkError::Short));
        let mut bytes = encode(&Chunk { at_ms: 0.0, channels: 2, sample_rate: SAMPLE_RATE, loading: false, samples: vec![0.0; 4] });
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(ChunkError::Magic));
        let mut bytes = encode(&Chunk { at_ms: 0.0, channels: 2, sample_rate: SAMPLE_RATE, loading: false, samples: vec![0.0; 4] });
        bytes.truncate(bytes.len() - 1);
        assert_eq!(decode(&bytes), Err(ChunkError::Short));
    }

    #[test]
    fn a_stream_stays_a_lead_ahead_and_never_sends_the_past() {
        let (first, next) = due(None, 10_000);
        assert_eq!(first.first(), Some(&(10_000 + LEAD_MS / 2)), "starts ahead of the clock");
        assert!(first.iter().all(|at| *at < 10_000 + LEAD_MS));
        let (more, _) = due(Some(next), 10_050);
        assert_eq!(more.first(), Some(&next), "carries on where it got to");
        let (late, _) = due(Some(next), 20_000);
        assert!(late.iter().all(|at| *at >= 20_000), "a stream that fell behind skips ahead");
        assert_eq!(chunk_frames(), 2_400);
    }
}
