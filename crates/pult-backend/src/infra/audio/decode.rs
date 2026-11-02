//! Turning a file somebody uploaded into samples, once.
//!
//! `symphonia`, which is pure Rust and reads what a show actually arrives as: wav and
//! flac from an editing session, mp3 and m4a from everywhere else. The codecs are named
//! in the manifest rather than taken wholesale — every extra decoder is another parser
//! reading a file that came in an email, and a console is not a media player.
//!
//! **Decoded once, on a blocking thread, and cached by sha.** A forty-megabyte mp3 is
//! four hundred megabytes of f32 once it is samples, so the cache is bounded by
//! [`budget_for`] and evicts what no timeline has on it. Content addressing is what
//! makes that safe: the sha is the key and an asset's bytes cannot change under it.
//!
//! **Interleaved f32 at the file's own rate**, never resampled here. Two things want
//! these samples and they want different rates — the player converts to whatever the
//! sound card asked for, and the detector to 22 050 — so converting once in the middle
//! would be converting twice for one of them.

use std::collections::{HashMap, HashSet};
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

/// How much decoded audio one station keeps in memory, beyond what its timelines hold.
///
/// **A third of the machine's memory, and never under 512 MB** — two and a half
/// gigabytes on an 8 GB Mac mini, which is a little over an hour and a half of stereo
/// at 48 kHz. A fixed 512 MB was twenty minutes, which one long ambient file filled on
/// its own.
///
/// **A budget for what no timeline holds, never for what one does.** Every reconcile
/// asks for every clip on every timeline, so a file on a timeline that is evicted is
/// decoded again on the next pass, and evicting it again makes room for the next one.
/// At HHG one seventy-minute mp3 was 1.5 GB of samples on its own: past the budget, it
/// pushed out the other four, they pushed it out, and the audio manager decoded all
/// five every few seconds — a core, and every Play and Stop queued behind it. So what
/// [`AudioCache::pin`] names stays, whatever it costs, and only the rest is evicted;
/// a show whose sound is over the budget is a warning, in the log and on the station
/// row, rather than a station that has stopped keeping time.
pub fn budget_for(total_memory: u64) -> usize {
    const FLOOR: u64 = 512 * 1024 * 1024;
    (total_memory / 3).max(FLOOR) as usize
}

/// Decoded files, by asset sha.
#[derive(Clone)]
pub struct AudioCache {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    decoded: HashMap<String, Arc<Decoded>>,
    /// Shas newest last, so eviction takes the oldest thing nothing is playing.
    order: Vec<String>,
    bytes: usize,
    /// What a timeline has on it, which no budget evicts.
    pinned: HashSet<String>,
    budget: usize,
    /// Whether the timelines' own sound is over the budget, so crossing it is one line
    /// in the log rather than one per reconcile.
    over: bool,
}

impl Inner {
    /// Down to the budget, oldest first, passing over what is pinned.
    fn evict(&mut self) {
        while self.bytes > self.budget {
            let Some(at) = self.order.iter().position(|sha| !self.pinned.contains(sha)) else { break };
            let oldest = self.order.remove(at);
            if let Some(gone) = self.decoded.remove(&oldest) {
                self.bytes -= gone.bytes();
                debug!("[audio] let go of the decoded {}", &oldest[..8.min(oldest.len())]);
            }
        }
        let pinned = self.pinned_bytes();
        let over = pinned > self.budget;
        if over && !self.over {
            warn!(
                "[audio] the timelines' sound is {} MB decoded, over the {} MB this station sets aside for it: \
                 it is kept anyway, and the machine is the one paying",
                pinned / (1024 * 1024),
                self.budget / (1024 * 1024)
            );
        }
        self.over = over;
    }

    fn pinned_bytes(&self) -> usize {
        self.pinned.iter().filter_map(|sha| self.decoded.get(sha)).map(|d| d.bytes()).sum()
    }
}

impl AudioCache {
    pub fn with_budget(budget: usize) -> Self {
        AudioCache {
            inner: Arc::new(Mutex::new(Inner { budget, ..Inner::default() })),
        }
    }

    /// A cache bounded by [`budget_for`] this machine's memory.
    pub fn sized_for_this_machine() -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        Self::with_budget(budget_for(system.total_memory()))
    }

    /// What the cache is holding, for the station row.
    pub fn stats(&self) -> pult_schema::types::station::AudioCacheStats {
        let Ok(inner) = self.inner.lock() else { return Default::default() };
        pult_schema::types::station::AudioCacheStats {
            budget: inner.budget as u64,
            held: inner.bytes as u64,
            files: inner.decoded.len() as u32,
            timeline_bytes: inner.pinned_bytes() as u64,
            timeline_files: inner.pinned.len() as u32,
            timeline_decoded: inner.pinned.iter().filter(|sha| inner.decoded.contains_key(*sha)).count() as u32,
        }
    }

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
        inner.evict();
    }

    /// The files the show's timelines have on them, kept however much they come to.
    /// A file taken off its last timeline goes back under the budget.
    pub fn pin(&self, shas: HashSet<String>) {
        let Ok(mut inner) = self.inner.lock() else { return };
        inner.pinned = shas;
        inner.evict();
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
    let mut samples: Vec<f32> = Vec::new();
    let (sample_rate, channels) = each_packet(bytes, |packet| samples.extend_from_slice(packet))?;
    if samples.is_empty() {
        return Err("nothing in this file decoded to any audio".to_string());
    }
    Ok(Decoded { samples: Arc::new(samples), channels, sample_rate })
}

/// How long a file is and how loud it ever gets, without keeping a sample of it.
///
/// For a question asked about a file before anybody has decided to play it — an
/// importer's "is this ambient loop silence?" — where [`decode_bytes`] would hold the
/// whole thing in memory to answer two numbers: fifteen minutes of stereo is three
/// hundred megabytes of f32. **Blocking**, like the decoder it shares.
pub fn survey(bytes: Vec<u8>) -> Result<Survey, String> {
    let mut peak = 0f32;
    let mut count = 0u64;
    let (sample_rate, channels) = each_packet(bytes, |packet| {
        count += packet.len() as u64;
        peak = packet.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
    })?;
    if count == 0 {
        return Err("nothing in this file decoded to any audio".to_string());
    }
    Ok(Survey { duration_ms: count / channels as u64 * 1_000 / sample_rate as u64, peak })
}

/// What [`survey`] found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Survey {
    pub duration_ms: u64,
    /// The loudest sample, as a fraction of full scale.
    pub peak: f32,
}

impl Survey {
    /// Nothing anybody would hear: quieter than -60 dBFS throughout. A file made to fill
    /// a slot that has to hold one, like an MP3 board's ambient track on a prop that has
    /// no ambience, peaks at about -72.
    pub fn silent(&self) -> bool {
        self.peak < 0.001
    }
}

/// Decode a file packet by packet, handing each one over interleaved, and answer its
/// rate and channel count.
fn each_packet(bytes: Vec<u8>, mut each: impl FnMut(&[f32])) -> Result<(u32, u16), String> {
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
                each(&scratch);
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
    if sample_rate == 0 {
        return Err("nothing in this file decoded to any audio".to_string());
    }
    Ok((sample_rate, channels.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(samples: usize) -> Arc<Decoded> {
        Arc::new(Decoded { samples: Arc::new(vec![0.0; samples]), channels: 1, sample_rate: 48_000 })
    }

    fn hold(inner: &mut Inner, sha: &str, samples: usize) {
        let d = decoded(samples);
        inner.bytes += d.bytes();
        inner.decoded.insert(sha.into(), d);
        inner.order.push(sha.into());
    }

    #[test]
    fn a_file_on_a_timeline_outlives_the_budget_and_the_rest_do_not() {
        let mut inner = Inner { budget: 400, ..Inner::default() };
        inner.pinned = ["long".to_string(), "loop".to_string()].into();
        hold(&mut inner, "loop", 10);
        hold(&mut inner, "old", 10);
        // One file bigger than the whole budget, the way a seventy-minute mp3 is.
        hold(&mut inner, "long", 1_000);
        inner.evict();
        let mut kept: Vec<_> = inner.decoded.keys().cloned().collect();
        kept.sort();
        assert_eq!(kept, ["long", "loop"]);
        assert_eq!(inner.bytes, 1_010 * 4);
        // Taken off its timeline, it goes back under the budget.
        inner.pinned = ["loop".to_string()].into();
        inner.evict();
        assert_eq!(inner.decoded.keys().collect::<Vec<_>>(), ["loop"]);
        assert!(!inner.over);
    }

    #[test]
    fn the_budget_is_a_third_of_the_machine_and_never_under_512_mb() {
        const GB: u64 = 1024 * 1024 * 1024;
        assert_eq!(budget_for(9 * GB), 3 * GB as usize);
        assert_eq!(budget_for(GB), 512 * 1024 * 1024);
    }

    #[test]
    fn the_figures_say_what_the_timelines_hold() {
        let cache = AudioCache::with_budget(400);
        cache.pin(["long".to_string(), "missing".to_string()].into());
        cache.put("long", decoded(1_000));
        cache.put("other", decoded(10));
        let stats = cache.stats();
        assert_eq!(stats.budget, 400);
        assert_eq!((stats.held, stats.files), (4_000, 1), "the unpinned one is over the budget and goes");
        assert_eq!(stats.timeline_bytes, 4_000);
        assert_eq!((stats.timeline_files, stats.timeline_decoded), (2, 1));
    }
}
