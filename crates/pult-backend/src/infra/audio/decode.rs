//! Turning a file somebody uploaded into samples, once.
//!
//! `symphonia`, which is pure Rust and reads what a show actually arrives as: wav and
//! flac from an editing session, mp3 and m4a from everywhere else. The codecs are named
//! in the manifest rather than taken wholesale — every extra decoder is another parser
//! reading a file that came in an email, and a console is not a media player.
//!
//! **Decoded once, on a blocking thread, and cached by sha.** A forty-megabyte mp3 is
//! four hundred megabytes of f32 once it is samples, so the cache is bounded by
//! [`CACHE_BYTES`] and evicts what nothing is playing. Content addressing is what makes
//! that safe: the sha is the key and an asset's bytes cannot change under it.
//!
//! **Interleaved f32 at the file's own rate**, never resampled here. Two things want
//! these samples and they want different rates — the player converts to whatever the
//! sound card asked for, and the detector to 22 050 — so converting once in the middle
//! would be converting twice for one of them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use tracing::{debug, warn};

/// One decoded file.
pub struct Decoded {
    /// Interleaved, at [`Decoded::sample_rate`].
    pub samples: Arc<Vec<f32>>,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Decoded {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0;
        }
        (self.samples.len() as u64 / self.channels as u64) * 1_000 / self.sample_rate as u64
    }

    fn bytes(&self) -> usize {
        self.samples.len() * std::mem::size_of::<f32>()
    }
}

/// How much decoded audio one station keeps in memory.
///
/// 512 MB, which is about twenty minutes of stereo at 48 kHz — a show's worth of stems
/// with room to spare, and small enough that a console with a rig in it is not
/// competing with its own cache. A file being *played* is never evicted, because the
/// player holds an `Arc` to its samples: eviction only drops this map's own reference.
pub const CACHE_BYTES: usize = 512 * 1024 * 1024;

/// Decoded files, by asset sha.
#[derive(Clone, Default)]
pub struct AudioCache {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    decoded: HashMap<String, Arc<Decoded>>,
    /// Shas newest last, so eviction takes the oldest thing nothing is playing.
    order: Vec<String>,
    bytes: usize,
}

impl AudioCache {
    pub fn get(&self, sha: &str) -> Option<Arc<Decoded>> {
        let mut inner = self.inner.lock().ok()?;
        let found = inner.decoded.get(sha).cloned()?;
        // Touched, so playing a file keeps it.
        inner.order.retain(|held| held != sha);
        inner.order.push(sha.to_string());
        Some(found)
    }

    fn put(&self, sha: &str, decoded: Arc<Decoded>) {
        let Ok(mut inner) = self.inner.lock() else { return };
        inner.bytes += decoded.bytes();
        inner.decoded.insert(sha.to_string(), decoded);
        inner.order.push(sha.to_string());
        while inner.bytes > CACHE_BYTES && inner.order.len() > 1 {
            let oldest = inner.order.remove(0);
            if let Some(gone) = inner.decoded.remove(&oldest) {
                inner.bytes -= gone.bytes();
                debug!("[audio] let go of the decoded {}", &oldest[..8.min(oldest.len())]);
            }
        }
    }

    /// The samples of an asset, decoding it if this station has not already.
    ///
    /// **Awaits a blocking thread.** Decoding four minutes of mp3 is hundreds of
    /// milliseconds of pure CPU, and doing it on the runtime is every timer in the
    /// station arriving late — the fault `infra/stations` records at six seconds and
    /// this one would be at a tenth of that, which is worse because nobody would think
    /// to look for it.
    pub async fn decode(&self, sha: &str, bytes: Vec<u8>) -> Result<Arc<Decoded>, String> {
        if let Some(found) = self.get(sha) {
            return Ok(found);
        }
        let sha = sha.to_string();
        let decoded = tokio::task::spawn_blocking(move || decode_bytes(bytes))
            .await
            .map_err(|e| format!("the decoder stopped: {e}"))??;
        let decoded = Arc::new(decoded);
        self.put(&sha, decoded.clone());
        Ok(decoded)
    }
}

/// Decode a whole file to interleaved f32.
///
/// **Blocking.** See [`AudioCache::decode`].
pub fn decode_bytes(bytes: Vec<u8>) -> Result<Decoded, String> {
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    // No hint: an asset is named by its sha and has no extension to give one. Every
    // format this console accepts has a magic number, so the probe finds it — and a
    // file that lies about its extension would be believed if one were given.
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), source, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("this file is not audio this console can read: {e}"))?;

    let track = format
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| "the file has no audio track this console can read".to_string())?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "the audio track does not say how it is encoded".to_string())?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("no decoder for this file: {e}"))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut skipped = 0usize;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(e) => return Err(format!("the file stops in the middle: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = audio.spec();
                sample_rate = spec.rate();
                channels = spec.channels().count() as u16;
                audio.copy_to_vec_interleaved(&mut scratch);
                samples.extend_from_slice(&scratch);
            }
            // A corrupt packet in the middle of a song is a click, not a reason to
            // refuse the whole file — which is what an operator would get five minutes
            // before the doors if this were strict.
            Err(SymphoniaError::DecodeError(_)) => skipped += 1,
            Err(e) => return Err(format!("the file could not be decoded: {e}")),
        }
    }
    if skipped > 0 {
        warn!("[audio] {skipped} packet(s) of this file would not decode and were skipped");
    }
    if sample_rate == 0 || samples.is_empty() {
        return Err("nothing in this file decoded to any audio".to_string());
    }
    Ok(Decoded { samples: Arc::new(samples), channels: channels.max(1), sample_rate })
}
