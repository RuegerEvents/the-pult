//! Turning a file somebody uploaded into samples, once.
//!
//! `symphonia`, which is pure Rust and reads what a show actually arrives as: wav and
//! flac from an editing session, mp3 and m4a from everywhere else. The codecs are named
//! in the manifest rather than taken wholesale — every extra decoder is another parser
//! reading a file that came in an email, and a console is not a media player.
//!
//! **Decoded once per station, not once per process.** A file is decoded straight into
//! a file of its own in [`Disk`], beside `preferences.toml`, and read back as a memory
//! mapping. At HHG one seventy-minute mp3 was 1.5 GB of f32: held on the heap it was
//! 1.5 GB of the Mini's memory for as long as the show was open, and decoded again —
//! seconds of a core — every time the station started. Mapped, it is pages the OS
//! reads in as the sound reaches them and lets go of when something else needs the
//! memory, and a restart costs a header read. Content addressing is what makes that
//! safe: the sha is the name and an asset's bytes cannot change under it.
//!
//! **The heap is the fallback, not a second design.** A station with nowhere to write
//! (no config directory, a disk that is full) decodes into a `Vec` exactly as it always
//! did, and [`budget_for`] bounds what of that no timeline holds.
//!
//! **Interleaved f32 at the file's own rate**, never resampled here. Two things want
//! these samples and they want different rates — the player converts to whatever the
//! sound card asked for, and the detector to 22 050 — so converting once in the middle
//! would be converting twice for one of them. f32 on the disk as well, rather than
//! i16 at half the size, so a file played from the disk is the same samples as one
//! played from memory.

use std::collections::{HashMap, HashSet};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use tracing::{debug, info, warn};

/// One decoded file.
pub struct Decoded {
    /// Interleaved, at [`Decoded::sample_rate`].
    pub samples: pult_audio::Samples,
    pub channels: u16,
    pub sample_rate: u32,
    /// Where the samples are a file in [`Disk`], the mapping of it.
    mapped: Option<Arc<Mapped>>,
}

impl Decoded {
    pub fn in_memory(samples: Vec<f32>, channels: u16, sample_rate: u32) -> Self {
        Decoded { samples: Arc::new(samples), channels, sample_rate, mapped: None }
    }

    fn mapped(mapped: Mapped) -> Self {
        let (channels, sample_rate) = (mapped.channels, mapped.sample_rate);
        let mapped = Arc::new(mapped);
        Decoded { samples: mapped.clone(), channels, sample_rate, mapped: Some(mapped) }
    }

    /// The samples, interleaved.
    pub fn data(&self) -> &[f32] {
        (*self.samples).as_ref()
    }

    /// What the mixer plays.
    pub fn source(&self) -> pult_audio::Source {
        pult_audio::Source {
            samples: self.samples.clone(),
            channels: self.channels as usize,
            sample_rate: self.sample_rate,
        }
    }

    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0;
        }
        (self.data().len() as u64 / self.channels as u64) * 1_000 / self.sample_rate as u64
    }

    fn bytes(&self) -> usize {
        self.data().len() * std::mem::size_of::<f32>()
    }

    /// Read off the disk rather than held on the heap.
    pub fn on_disk(&self) -> bool {
        self.mapped.is_some()
    }

    /// Ask the OS to start reading `len_ms` of the file from `from_ms` in, so the audio
    /// callback finds it in memory when it gets there.
    ///
    /// **The callback must never wait on a disk.** A page that is not resident when the
    /// mixer reads it is a read on the real-time thread — a hundred microseconds on an
    /// SSD usually, and nothing guarantees usually. So the manager says where each deck
    /// is about to be, a few seconds ahead, and the OS reads it in the background:
    /// `madvise(WILLNEED)` returns at once. Nothing for a file in memory, and nothing at
    /// all where the platform has no `madvise`.
    pub fn prefetch(&self, from_ms: f64, len_ms: f64) {
        let Some(mapped) = &self.mapped else { return };
        let frame_bytes = self.channels.max(1) as f64 * 4.0 * self.sample_rate as f64 / 1_000.0;
        let from = (from_ms.max(0.0) * frame_bytes) as usize;
        let len = (len_ms.max(0.0) * frame_bytes) as usize;
        mapped.prefetch(from, len);
    }
}

/// How much decoded audio one station keeps on the heap, beyond what its timelines hold.
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
///
/// Only the heap counts. A file read off [`Disk`] is pages the OS can drop and read
/// again, and is held while a timeline has it on and let go of when none does.
pub fn budget_for(total_memory: u64) -> usize {
    const FLOOR: u64 = 512 * 1024 * 1024;
    (total_memory / 3).max(FLOOR) as usize
}

/// Decoded files, by asset sha.
#[derive(Clone)]
pub struct AudioCache {
    inner: Arc<Mutex<Inner>>,
    /// Where decoded files are written. `None` decodes onto the heap only.
    disk: Option<Arc<Disk>>,
}

#[derive(Default)]
struct Inner {
    decoded: HashMap<String, Arc<Decoded>>,
    /// Shas newest last, so eviction takes the oldest thing nothing is playing.
    order: Vec<String>,
    /// Of what is held, what is on the heap.
    bytes: usize,
    /// What a timeline has on it, which no budget evicts.
    pinned: HashSet<String>,
    budget: usize,
    /// Whether the timelines' own sound is over the budget, so crossing it is one line
    /// in the log rather than one per reconcile.
    over: bool,
}

impl Inner {
    /// Down to the budget, oldest first, passing over what is pinned — and every mapped
    /// file no timeline has on it, which costs nothing to map again.
    fn evict(&mut self) {
        let pinned = &self.pinned;
        let unmapped: Vec<String> = self
            .decoded
            .iter()
            .filter(|(sha, d)| d.on_disk() && !pinned.contains(*sha))
            .map(|(sha, _)| sha.clone())
            .collect();
        for sha in unmapped {
            self.decoded.remove(&sha);
            self.order.retain(|held| *held != sha);
        }
        while self.bytes > self.budget {
            let Some(at) = self
                .order
                .iter()
                .position(|sha| !self.pinned.contains(sha) && self.decoded.get(sha).is_some_and(|d| !d.on_disk()))
            else {
                break;
            };
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
                "[audio] the timelines' sound is {} MB decoded in memory, over the {} MB this station sets aside for it: \
                 it is kept anyway, and the machine is the one paying",
                pinned / (1024 * 1024),
                self.budget / (1024 * 1024)
            );
        }
        self.over = over;
    }

    /// What the timelines hold on the heap.
    fn pinned_bytes(&self) -> usize {
        self.pinned
            .iter()
            .filter_map(|sha| self.decoded.get(sha))
            .filter(|d| !d.on_disk())
            .map(|d| d.bytes())
            .sum()
    }
}

impl AudioCache {
    /// On the heap only.
    pub fn with_budget(budget: usize) -> Self {
        AudioCache { inner: Arc::new(Mutex::new(Inner { budget, ..Inner::default() })), disk: None }
    }

    /// A cache bounded by [`budget_for`] this machine's memory.
    pub fn sized_for_this_machine() -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        Self::with_budget(budget_for(system.total_memory()))
    }

    /// Decoding into `disk` from now on, and reading back what is already there.
    pub fn on_disk(mut self, disk: Disk) -> Self {
        self.disk = Some(Arc::new(disk));
        self
    }

    /// What the cache is holding, for the station row.
    pub fn stats(&self) -> pult_schema::types::station::AudioCacheStats {
        let Ok(inner) = self.inner.lock() else { return Default::default() };
        let mapped = inner.decoded.values().filter(|d| d.on_disk());
        let (mapped_files, mapped_bytes) = mapped.fold((0u32, 0u64), |(n, b), d| (n + 1, b + d.bytes() as u64));
        pult_schema::types::station::AudioCacheStats {
            budget: inner.budget as u64,
            held: inner.bytes as u64,
            files: inner.decoded.len() as u32 - mapped_files,
            timeline_bytes: inner.pinned_bytes() as u64,
            timeline_files: inner.pinned.len() as u32,
            timeline_decoded: inner.pinned.iter().filter(|sha| inner.decoded.contains_key(*sha)).count() as u32,
            mapped: mapped_bytes,
            mapped_files,
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
        if !decoded.on_disk() {
            inner.bytes += decoded.bytes();
        }
        if let Some(replaced) = inner.decoded.insert(sha.to_string(), decoded) {
            if !replaced.on_disk() {
                inner.bytes -= replaced.bytes();
            }
        }
        inner.order.retain(|held| held != sha);
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

    /// The samples of an asset held already, or on the disk from an earlier run.
    ///
    /// **Asked before the bytes are fetched**, because fetching is reading a
    /// hundred-megabyte mp3 out of the asset store — or across the link from a peer —
    /// to decode something this station decoded yesterday.
    pub async fn from_disk(&self, sha: &str) -> Option<Arc<Decoded>> {
        if let Some(found) = self.get(sha) {
            return Some(found);
        }
        let disk = self.disk.clone()?;
        let wanted = sha.to_string();
        let found = Arc::new(tokio::task::spawn_blocking(move || disk.open(&wanted)).await.ok()??);
        self.put(sha, found.clone());
        Some(found)
    }

    /// The samples of an asset, decoding it if this station has not already.
    ///
    /// **Awaits a blocking thread.** Decoding four minutes of mp3 is hundreds of
    /// milliseconds of pure CPU, and doing it on the runtime is every timer in the
    /// station arriving late — the fault `infra/stations` records at six seconds and
    /// this one would be at a tenth of that, which is worse because nobody would think
    /// to look for it. And whoever awaits *this* waits as long: the audio manager never
    /// does, and hands it to a task of its own (`AudioManager::decoded`).
    pub async fn decode(&self, sha: &str, bytes: Vec<u8>) -> Result<Arc<Decoded>, String> {
        if let Some(found) = self.get(sha) {
            return Ok(found);
        }
        let (disk, wanted) = (self.disk.clone(), sha.to_string());
        let pinned = self.inner.lock().map(|i| i.pinned.clone()).unwrap_or_default();
        let decoded = tokio::task::spawn_blocking(move || {
            let Some(disk) = disk else { return decode_bytes(bytes) };
            let bytes = Shared(Arc::new(bytes));
            match disk.write(&wanted, bytes.clone()) {
                Ok(decoded) => {
                    disk.prune(&pinned);
                    return Ok(decoded);
                }
                // The file itself, which the heap would refuse just the same.
                Err(Refusal::File(e)) => return Err(e),
                Err(Refusal::Disk(e)) => {
                    warn!("[audio] a decoded file could not be written to {}: {e}; it is held in memory instead", disk.dir.display());
                }
            }
            let bytes = Arc::try_unwrap(bytes.0).unwrap_or_else(|shared| (*shared).clone());
            decode_bytes(bytes)
        })
        .await
        .map_err(|e| format!("the decoder stopped: {e}"))??;
        let decoded = Arc::new(decoded);
        self.put(sha, decoded.clone());
        Ok(decoded)
    }
}

/// Decode a whole file to interleaved f32, onto the heap.
///
/// **Blocking.** See [`AudioCache::decode`].
pub fn decode_bytes(bytes: Vec<u8>) -> Result<Decoded, String> {
    let mut samples: Vec<f32> = Vec::new();
    let (sample_rate, channels) = each_packet(bytes, |packet| samples.extend_from_slice(packet))?;
    if samples.is_empty() {
        return Err("nothing in this file decoded to any audio".to_string());
    }
    Ok(Decoded::in_memory(samples, channels, sample_rate))
}

// ── On the disk ───────────────────────────────────────────────────────────────

/// Where this station keeps the files it has decoded, and how much of them.
///
/// **A station's, never the show's.** Every byte of it can be made again from an asset,
/// and a `.pultz` carrying an hour of f32 per song is a cost nobody would accept for a
/// cache. LOCAL like the log file and the MVR-xchange commits, beside them.
///
/// **Bounded by size, oldest first, and never what a timeline has on it** — the rule
/// the heap's budget follows, for the reason it does. A file is touched each time it is
/// opened, so "oldest" is the one nothing has played for longest.
///
/// Two stations in one process may share one: a file is written under a name of its
/// own and renamed into place, so two decoding the same sha both end with one whole
/// file, and a mapping of the one replaced goes on reading what it read.
pub struct Disk {
    dir: PathBuf,
    cap: u64,
}

/// Twenty gigabytes: about fourteen hours of stereo at 48 kHz.
pub const DEFAULT_DISK_CAP: u64 = 20 * 1024 * 1024 * 1024;

/// `PLDA`, then a version, the channels, the rate and how many samples, then nothing
/// to the end of the header. Sixty-four bytes so the samples after it are aligned for
/// f32 in a mapping, which starts on a page.
const MAGIC: &[u8; 4] = b"PLDA";
const VERSION: u16 = 1;
const HEADER: usize = 64;

/// Why a file did not reach the disk: something about the file, or about the disk.
#[derive(Debug)]
enum Refusal {
    File(String),
    Disk(String),
}

impl Disk {
    pub fn new(dir: PathBuf, cap: u64) -> Self {
        Disk { dir, cap }
    }

    /// The name a sha is kept under, or `None` for one that is not a name — an asset
    /// sha is hex, and anything else must not become a path.
    fn path(&self, sha: &str) -> Option<PathBuf> {
        let plain = !sha.is_empty() && sha.len() <= 128 && sha.chars().all(|c| c.is_ascii_alphanumeric());
        plain.then(|| self.dir.join(format!("{sha}.pcm")))
    }

    /// A file decoded here before, mapped. `None` where there is none, or where what is
    /// there is not whole — which is then removed, so it is decoded again.
    ///
    /// **Blocking**, though cheap: a header read and a mapping.
    pub fn open(&self, sha: &str) -> Option<Decoded> {
        if cfg!(target_endian = "big") {
            return None;
        }
        let path = self.path(sha)?;
        let file = std::fs::File::options().read(true).write(true).open(&path).ok()?;
        match Mapped::of(&file) {
            Ok(mapped) => {
                // Touched, for the pruning order.
                let _ = file.set_modified(std::time::SystemTime::now());
                Some(Decoded::mapped(mapped))
            }
            Err(e) => {
                warn!("[audio] {} is not a whole decoded file ({e}); it is decoded again", path.display());
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Decode `bytes` straight into a file and map it.
    ///
    /// **Streamed**, packet by packet through a buffer, so the decode costs a few
    /// megabytes of memory however long the song is — the heap path needs the whole of
    /// it at once. Written under a name of its own, synced and renamed into place, so a
    /// station that stops halfway leaves nothing [`Disk::open`] would believe.
    fn write(&self, sha: &str, bytes: Shared) -> Result<Decoded, Refusal> {
        if cfg!(target_endian = "big") {
            return Err(Refusal::Disk("the samples are kept little-endian".into()));
        }
        let path = self.path(sha).ok_or_else(|| Refusal::Disk(format!("{sha} is not an asset name")))?;
        std::fs::create_dir_all(&self.dir).map_err(|e| Refusal::Disk(e.to_string()))?;
        let partial = self.dir.join(format!("{sha}.{}.partial", uuid::Uuid::new_v4().simple()));
        let written = self.write_to(&partial, bytes);
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        let samples = written?;
        std::fs::rename(&partial, &path).map_err(|e| {
            let _ = std::fs::remove_file(&partial);
            Refusal::Disk(e.to_string())
        })?;
        let file = std::fs::File::open(&path).map_err(|e| Refusal::Disk(e.to_string()))?;
        let mapped = Mapped::of(&file).map_err(Refusal::Disk)?;
        info!(
            "[audio] decoded {} to the disk: {} MB",
            &sha[..8.min(sha.len())],
            samples * 4 / (1024 * 1024)
        );
        Ok(Decoded::mapped(mapped))
    }

    /// How many samples went into `partial`.
    fn write_to(&self, partial: &Path, bytes: Shared) -> Result<usize, Refusal> {
        let file = std::fs::File::create(partial).map_err(|e| Refusal::Disk(e.to_string()))?;
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
        out.write_all(&[0; HEADER]).map_err(|e| Refusal::Disk(e.to_string()))?;
        let mut count = 0usize;
        // The first write that fails is the answer; the decode runs on to the end, with
        // nothing more written, because `each_packet` has no way to be stopped.
        let mut failed: Option<std::io::Error> = None;
        let mut buffer: Vec<u8> = Vec::new();
        let (sample_rate, channels) = each_packet(bytes, |packet| {
            if failed.is_some() {
                return;
            }
            buffer.clear();
            buffer.extend(packet.iter().flat_map(|s| s.to_le_bytes()));
            match out.write_all(&buffer) {
                Ok(()) => count += packet.len(),
                Err(e) => failed = Some(e),
            }
        })
        .map_err(Refusal::File)?;
        if let Some(e) = failed {
            return Err(Refusal::Disk(e.to_string()));
        }
        if count == 0 {
            return Err(Refusal::File("nothing in this file decoded to any audio".to_string()));
        }
        let disk = |e: std::io::Error| Refusal::Disk(e.to_string());
        let mut file = out.into_inner().map_err(|e| Refusal::Disk(e.to_string()))?;
        file.rewind().map_err(disk)?;
        file.write_all(&header(channels, sample_rate, count as u64)).map_err(disk)?;
        file.sync_all().map_err(disk)?;
        Ok(count)
    }

    /// Down to the cap, oldest first, passing over what `keep` names.
    ///
    /// A mapping of a file removed here goes on reading it — the name goes and the
    /// pages stay until the mapping does — so a deck playing one is not disturbed.
    /// Where the platform will not remove a mapped file, it is left for next time.
    pub fn prune(&self, keep: &HashSet<String>) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf, String)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?.to_string();
                let meta = entry.metadata().ok()?;
                let sha = name.strip_suffix(".pcm")?.to_string();
                Some((meta.modified().ok()?, meta.len(), path, sha))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort_by_key(|f| f.0);
        for (_, size, path, sha) in files {
            if total <= self.cap {
                break;
            }
            if keep.contains(&sha) {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                debug!("[audio] removed the decoded {} from the disk", &sha[..8.min(sha.len())]);
                total -= size;
            }
        }
    }
}

fn header(channels: u16, sample_rate: u32, samples: u64) -> [u8; HEADER] {
    let mut header = [0u8; HEADER];
    header[0..4].copy_from_slice(MAGIC);
    header[4..6].copy_from_slice(&VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&channels.to_le_bytes());
    header[8..12].copy_from_slice(&sample_rate.to_le_bytes());
    header[12..20].copy_from_slice(&samples.to_le_bytes());
    header
}

/// Bytes the decoder reads from, shared so a decode that could not reach the disk can
/// be done again on the heap without a second copy of the file.
#[derive(Clone)]
struct Shared(Arc<Vec<u8>>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// A decoded file, mapped.
struct Mapped {
    map: memmap2::Mmap,
    samples: usize,
    channels: u16,
    sample_rate: u32,
}

impl Mapped {
    /// Checked whole before it is believed: the magic, the version, and a length that
    /// is exactly the header and the samples it says it has.
    fn of(file: &std::fs::File) -> Result<Mapped, String> {
        // SAFETY: the file is this station's own, written once under another name and
        // renamed into place, and never written again — a removal or a replacement
        // leaves this mapping reading what it read. Something outside the console
        // truncating it while it plays is the one thing that could fault, and that is
        // somebody editing a cache directory by hand.
        let map = unsafe { memmap2::Mmap::map(file) }.map_err(|e| e.to_string())?;
        if map.len() < HEADER || &map[0..4] != MAGIC {
            return Err("no header".into());
        }
        let version = u16::from_le_bytes([map[4], map[5]]);
        if version != VERSION {
            return Err(format!("version {version}, where this build writes {VERSION}"));
        }
        let channels = u16::from_le_bytes([map[6], map[7]]);
        let sample_rate = u32::from_le_bytes(map[8..12].try_into().unwrap_or_default());
        let samples = u64::from_le_bytes(map[12..20].try_into().unwrap_or_default()) as usize;
        if channels == 0 || sample_rate == 0 {
            return Err("no channels or no rate".into());
        }
        if samples.checked_mul(4).and_then(|b| b.checked_add(HEADER)) != Some(map.len()) {
            return Err(format!("{} bytes, for a header that says {samples} samples", map.len()));
        }
        Ok(Mapped { map, samples, channels, sample_rate })
    }

    /// See [`Decoded::prefetch`]. Offsets in bytes into the samples.
    fn prefetch(&self, from: usize, len: usize) {
        #[cfg(unix)]
        {
            let data = self.samples * 4;
            if from >= data || len == 0 {
                return;
            }
            let len = len.min(data - from);
            let _ = self.map.advise_range(memmap2::Advice::WillNeed, HEADER + from, len);
        }
        #[cfg(not(unix))]
        let _ = (from, len);
    }
}

impl AsRef<[f32]> for Mapped {
    fn as_ref(&self) -> &[f32] {
        // SAFETY: `of` checked the mapping is exactly the header and `samples` f32s
        // after it; a mapping starts on a page and the header is sixty-four bytes, so
        // the samples are aligned; and they were written little-endian on a target that
        // `Disk` refuses unless it is little-endian.
        unsafe { std::slice::from_raw_parts(self.map.as_ptr().add(HEADER) as *const f32, self.samples) }
    }
}

// ── Reading a file ────────────────────────────────────────────────────────────

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
fn each_packet<B>(bytes: B, mut each: impl FnMut(&[f32])) -> Result<(u32, u16), String>
where
    B: AsRef<[u8]> + Send + Sync + 'static,
{
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
        Arc::new(Decoded::in_memory(vec![0.0; samples], 1, 48_000))
    }

    fn hold(inner: &mut Inner, sha: &str, samples: usize) {
        let d = decoded(samples);
        inner.bytes += d.bytes();
        inner.decoded.insert(sha.into(), d);
        inner.order.push(sha.into());
    }

    /// The house pattern for a directory a test owns: named for the run, left behind if
    /// a test panics, which is what makes a failure inspectable.
    fn a_dir() -> PathBuf {
        std::env::temp_dir().join(format!("pult-decoded-{}", uuid::Uuid::new_v4()))
    }

    fn a_wav(seconds: usize) -> (Vec<f32>, Vec<u8>) {
        let tone: Vec<f32> = (0..48_000 * 2 * seconds).map(|i| ((i % 200) as f32 / 100.0) - 1.0).collect();
        let wav = pult_audio::wav::encode(&tone, 2, 48_000, pult_audio::wav::Encoding::Pcm16).unwrap();
        (tone, wav)
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
        assert_eq!((stats.mapped, stats.mapped_files), (0, 0));
    }

    /// The whole point: what is played off the disk is the samples the heap would
    /// have held, and a second station — or the same one after a restart — opens it
    /// without decoding anything.
    #[tokio::test]
    async fn a_file_decoded_to_the_disk_is_the_same_samples_and_is_not_decoded_again() {
        let dir = a_dir();
        let (_, wav) = a_wav(2);
        let in_memory = decode_bytes(wav.clone()).unwrap();

        let cache = AudioCache::with_budget(0).on_disk(Disk::new(dir.clone(), DEFAULT_DISK_CAP));
        cache.pin(["abc".to_string()].into());
        let decoded = cache.decode("abc", wav).await.unwrap();
        assert!(decoded.on_disk());
        assert_eq!((decoded.channels, decoded.sample_rate), (2, 48_000));
        assert_eq!(decoded.data(), in_memory.data(), "bit for bit");
        let stats = cache.stats();
        assert_eq!((stats.held, stats.timeline_bytes), (0, 0), "nothing on the heap");
        assert_eq!((stats.mapped, stats.mapped_files, stats.timeline_decoded), (decoded.bytes() as u64, 1, 1));
        assert!(
            std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains("partial")),
            "and nothing half-written is left behind"
        );

        // A station that starts afresh is never asked for the bytes.
        let restarted = AudioCache::with_budget(0).on_disk(Disk::new(dir.clone(), DEFAULT_DISK_CAP));
        let again = restarted.from_disk("abc").await.expect("found on the disk");
        assert_eq!(again.data(), in_memory.data());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A station that stopped halfway through writing one, or a file from another
    /// version, is decoded again rather than believed.
    #[test]
    fn a_file_that_is_not_whole_is_not_believed_and_goes() {
        let dir = a_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let disk = Disk::new(dir.clone(), DEFAULT_DISK_CAP);
        let mut short = header(2, 48_000, 1_000).to_vec();
        short.extend_from_slice(&[0; 400]);
        std::fs::write(dir.join("short.pcm"), &short).unwrap();
        let mut old = header(2, 48_000, 1).to_vec();
        old[4] = 9;
        old.extend_from_slice(&[0; 4]);
        std::fs::write(dir.join("old.pcm"), &old).unwrap();
        assert!(disk.open("short").is_none());
        assert!(disk.open("old").is_none());
        assert!(!dir.join("short.pcm").exists() && !dir.join("old.pcm").exists());
        // And a name that is not a sha never becomes a path.
        assert!(disk.path("../preferences").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Over the cap, the oldest goes first — but never one a timeline has on it, which
    /// would be decoded again on the next look and pushed out again after it.
    #[tokio::test]
    async fn the_disk_is_pruned_oldest_first_and_never_of_a_timeline() {
        let dir = a_dir();
        let (_, wav) = a_wav(1);
        let cache = AudioCache::with_budget(0).on_disk(Disk::new(dir.clone(), DEFAULT_DISK_CAP));
        for sha in ["one", "two", "three"] {
            cache.decode(sha, wav.clone()).await.unwrap();
        }
        let size = std::fs::metadata(dir.join("one.pcm")).unwrap().len();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        for (sha, ago) in [("one", 3), ("two", 2), ("three", 1)] {
            let file = std::fs::File::options().write(true).open(dir.join(format!("{sha}.pcm"))).unwrap();
            file.set_modified(old - std::time::Duration::from_secs(ago)).unwrap();
        }
        // Room for one and a half: two have to go, and `one`, the oldest, is on a timeline.
        Disk::new(dir.clone(), size * 3 / 2).prune(&["one".to_string()].into());
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["one.pcm"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A file that is not audio is refused once, as it is on the heap — not taken for a
    /// disk that failed and decoded a second time.
    #[tokio::test]
    async fn a_file_that_is_not_audio_is_refused_from_the_disk_too() {
        let dir = a_dir();
        let cache = AudioCache::with_budget(0).on_disk(Disk::new(dir.clone(), DEFAULT_DISK_CAP));
        let refused = cache.decode("abc", b"not audio at all".to_vec()).await;
        assert!(refused.is_err_and(|e| e.contains("not audio")));
        assert!(!dir.join("abc.pcm").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Somewhere that cannot be written is the heap, as it always was.
    #[tokio::test]
    async fn a_disk_that_cannot_be_written_falls_back_to_memory() {
        let blocker = a_dir();
        std::fs::write(&blocker, b"a file where the directory should be").unwrap();
        let cache = AudioCache::with_budget(usize::MAX).on_disk(Disk::new(blocker.join("decoded"), DEFAULT_DISK_CAP));
        let (_, wav) = a_wav(1);
        let decoded = cache.decode("abc", wav).await.unwrap();
        assert!(!decoded.on_disk());
        assert!(cache.stats().held > 0);
        let _ = std::fs::remove_file(blocker);
    }
}
