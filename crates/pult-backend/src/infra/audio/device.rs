//! The sound card, and everything about it that blocks.
//!
//! Three things live here and nothing else does: enumerating what the machine has,
//! mixing every playing clip out of it through one stream, and reading timecode into
//! it. All three are
//! `cpal`, and all three are **on threads of their own**.
//!
//! # Why threads rather than tasks
//!
//! Two separate reasons, and either would be enough.
//!
//! **`cpal::Stream` is not `Send`.** It cannot be held across an await or moved between
//! tasks, so whatever owns one has to stay where it was built. A thread is the honest
//! shape for that; a task pretending would be a compile error at best and a stream
//! silently dropped at worst.
//!
//! **Enumeration blocks, and how long is the operating system's business.**
//! `infra/stations` records what that costs when it is got wrong: the first volume
//! enumeration in a process took six seconds against a large `target/debug/deps`, and
//! six seconds on the runtime is a station that accepts no connection and runs no
//! timer. Audio device enumeration is the same class of call — it talks to CoreAudio,
//! WASAPI or ALSA, each of which may be waiting on a driver — so it goes through
//! `spawn_blocking` every time, without exception.
//!
//! # A device is named by what it calls itself
//!
//! `preferences.toml`'s `[audio] output` is a **name**, matched case-insensitively
//! against what the device says it is called, and falling back to the platform's own
//! device id where somebody has pasted one of those in. Both, for the reason
//! `types::network::resolve` takes an interface name *or* an address: a name is what an
//! operator recognises and survives being plugged into a different port, and an id is
//! what survives somebody renaming the interface. Named nothing at all is the system
//! default, which is what a console whose operator has never opened the Settings panel
//! gets.
//!
//! A device this machine has not got is a **refusal that says so**, not a silent
//! fallback to the default — the rule [`crate::infra::net::Network::bind`] follows for a
//! cable, and for the same reason: from the desk, "playing out of the wrong output" and
//! "playing out of nothing" look identical, and only one of them is recoverable by
//! reading a status row.

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_queue::ArrayQueue;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

/// What this machine can play through and listen on.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AudioDevices {
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
}

/// One device, as the Settings panel lists it.
#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    /// What the device calls itself. What a preference names, and what a picker shows.
    pub name: String,
    /// The platform's own id. Also accepted as a preference, because a name changes
    /// when somebody renames an interface and this does not.
    pub id: String,
    /// Whether this is the host's default for its direction.
    pub is_default: bool,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Everything the machine has, in both directions.
///
/// **Blocking.** Call it through `spawn_blocking` — see the module header. A device
/// that will not describe itself is listed under its id rather than skipped: an
/// interface that is misbehaving is exactly the one somebody is looking for.
pub fn enumerate() -> AudioDevices {
    let host = cpal::default_host();
    let default_out = host.default_output_device().and_then(|d| d.id().ok()).map(|id| id.to_string());
    let default_in = host.default_input_device().and_then(|d| d.id().ok()).map(|id| id.to_string());

    let mut devices = AudioDevices::default();
    let Ok(found) = host.devices() else {
        warn!("[audio] this machine's audio devices could not be enumerated");
        return devices;
    };
    for device in found {
        let id = device.id().map(|id| id.to_string()).unwrap_or_default();
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| id.clone());
        if let Ok(config) = device.default_output_config() {
            devices.outputs.push(AudioDevice {
                name: name.clone(),
                id: id.clone(),
                is_default: default_out.as_deref() == Some(id.as_str()),
                channels: config.channels(),
                sample_rate: config.sample_rate(),
            });
        }
        if let Ok(config) = device.default_input_config() {
            devices.inputs.push(AudioDevice {
                name,
                id: id.clone(),
                is_default: default_in.as_deref() == Some(id.as_str()),
                channels: config.channels(),
                sample_rate: config.sample_rate(),
            });
        }
    }
    devices
}

/// Which device a preference names, in a direction.
///
/// `None` for the name is the host default. `Err` is the refusal — see the module
/// header on why this does not quietly fall back.
fn find(
    host: &cpal::Host,
    wanted: Option<&str>,
    output: bool,
) -> Result<cpal::Device, String> {
    let Some(wanted) = wanted else {
        return match output {
            true => host.default_output_device().ok_or_else(|| "this machine has no audio output".to_string()),
            false => host.default_input_device().ok_or_else(|| "this machine has no audio input".to_string()),
        };
    };
    let devices = host.devices().map_err(|e| format!("audio devices cannot be listed: {e}"))?;
    for device in devices {
        let usable = match output {
            true => device.default_output_config().is_ok(),
            false => device.default_input_config().is_ok(),
        };
        if !usable {
            continue;
        }
        let id = device.id().map(|id| id.to_string()).unwrap_or_default();
        let name = device.description().map(|d| d.name().to_string()).unwrap_or_default();
        if name.eq_ignore_ascii_case(wanted) || id == wanted {
            return Ok(device);
        }
    }
    Err(format!("this machine has no audio device called {wanted:?}"))
}

// ── Playing ───────────────────────────────────────────────────────────────────

/// How many timelines may sound at once on one station. Allocated when the device opens,
/// so a deck arriving never grows a list inside the callback.
pub const MAX_DECKS: usize = 64;

/// What the manager asks of the callback. Carried over a bounded lock-free queue: the
/// callback takes what is waiting at the top of each buffer and never waits for more.
enum Command {
    /// A deck, new or rebuilt. `keep_cursor` carries on from where the old one was —
    /// see [`pult_audio::Mixer::put`].
    Put { deck: Box<pult_audio::Deck>, keep_cursor: bool },
    Remove(u128),
    Seek(u128, f64),
    Rate(u128, f64),
}

/// Where one deck has got to, read back by the manager. One per slot, written by the
/// callback after every buffer.
#[derive(Debug, Default)]
struct Slot {
    /// In timeline milliseconds ×1000. Fixed point rather than a float because there is
    /// no atomic f64, and rounding to the millisecond in the callback would quantise the
    /// drift figure to the thing it is trying to measure.
    position_micros: AtomicU64,
    sounding: AtomicBool,
}

/// What the callback and the manager share.
///
/// Atomics and lock-free queues rather than a lock, because the other side of every one
/// of these is a real-time audio callback: a callback that blocked on a mutex the
/// manager happened to be holding would drop a buffer, and a dropped buffer is an
/// audible click.
struct Shared {
    commands: ArrayQueue<Command>,
    /// What the callback has finished with, to be dropped on the manager's side. A
    /// deck holds `Arc`s to decoded files, and freeing forty megabytes of samples is not
    /// something to do between two buffers.
    trash: ArrayQueue<Box<pult_audio::Deck>>,
    slots: Vec<Slot>,
    /// What the device says is between the callback and the loudspeaker, in
    /// microseconds. Written by the callback because only it is told.
    latency_micros: AtomicU64,
    stop: AtomicBool,
}

/// The one output stream a station mixes every clip into.
///
/// Dropping it stops the sound: the thread notices, drops the stream, and exits.
pub struct Output {
    shared: Arc<Shared>,
    pub device_name: String,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Drop for Output {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl Output {
    /// Open a device, with nothing playing yet.
    ///
    /// The whole of this happens on a thread of its own, including finding the device —
    /// which is an enumeration, and blocks. What comes back over the oneshot is either
    /// a handle or the reason there is not one, in words a panel can print.
    pub async fn open(device: Option<String>) -> Result<Output, String> {
        let shared = Arc::new(Shared {
            commands: ArrayQueue::new(4 * MAX_DECKS),
            trash: ArrayQueue::new(8 * MAX_DECKS),
            slots: (0..MAX_DECKS).map(|_| Slot::default()).collect(),
            latency_micros: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let (ready, started) = oneshot::channel();
        let for_thread = shared.clone();

        // A named thread, so a stalled console's stack traces say which of these it is.
        std::thread::Builder::new()
            .name("audio-out".into())
            .spawn(move || play_thread(device, for_thread, ready))
            .map_err(|e| format!("no thread for audio playback: {e}"))?;

        let (device_name, channels, sample_rate) = started
            .await
            .map_err(|_| "the audio thread stopped before it said anything".to_string())??;
        Ok(Output { shared, device_name, channels, sample_rate })
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.shared
            .commands
            .push(command)
            .map_err(|_| "the audio callback is not taking commands".to_string())
    }

    /// Start a deck, or replace the one with its id. `deck.tag` is the slot its cursor
    /// is read back from.
    pub fn put(&self, deck: pult_audio::Deck, keep_cursor: bool) -> Result<(), String> {
        self.send(Command::Put { deck: Box::new(deck), keep_cursor })
    }

    pub fn remove(&self, id: u128) {
        let _ = self.send(Command::Remove(id));
    }

    pub fn seek(&self, id: u128, timeline_ms: u64) {
        let _ = self.send(Command::Seek(id, timeline_ms as f64));
    }

    /// Run at this multiple of real time until told otherwise. `1.0` is normal.
    pub fn set_rate(&self, id: u128, ratio: f32) {
        let _ = self.send(Command::Rate(id, ratio as f64));
    }

    /// Where the sound *coming out of the loudspeaker* is on a deck's timeline.
    ///
    /// The callback's own cursor **minus the device's reported latency**, and that
    /// subtraction is the whole point: a callback that has just filled a buffer has
    /// read ahead of what anybody in the room can hear, and a console that anchored the
    /// show to the read position would run every cue a buffer early — which at a
    /// conservative 512 frames and 44.1 kHz is 12 ms, and on a machine set up for
    /// safety rather than latency is five times that.
    pub fn position_ms(&self, slot: usize) -> u64 {
        let Some(slot) = self.shared.slots.get(slot) else { return 0 };
        let cursor = slot.position_micros.load(Ordering::Relaxed) / 1_000;
        cursor.saturating_sub(self.latency_ms())
    }

    /// What is between the callback and the loudspeaker. A deck is started this far
    /// *ahead* of where the show is, so that what is heard is where the show is.
    pub fn latency_ms(&self) -> u64 {
        self.shared.latency_micros.load(Ordering::Relaxed) / 1_000
    }

    /// Whether anything on a deck was audible in the last buffer.
    pub fn sounding(&self, slot: usize) -> bool {
        self.shared.slots.get(slot).is_some_and(|s| s.sounding.load(Ordering::Relaxed))
    }

    /// Drop what the callback has finished with, here and not there.
    pub fn collect(&self) {
        while self.shared.trash.pop().is_some() {}
    }
}

fn play_thread(
    device: Option<String>,
    shared: Arc<Shared>,
    ready: oneshot::Sender<Result<(String, u16, u32), String>>,
) {
    let host = cpal::default_host();
    let found = match find(&host, device.as_deref(), true) {
        Ok(device) => device,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let name = found
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "an audio device".to_string());
    let supported = match found.default_output_config() {
        Ok(config) => config,
        Err(e) => {
            let _ = ready.send(Err(format!("{name} will not say what it supports: {e}")));
            return;
        }
    };
    let format = supported.sample_format();
    // The device's whole channel count, always: a Dante Virtual Soundcard opened at
    // sixty-four channels is sixty-four zones, and a stream opened at two would be the
    // first two of them.
    let config: cpal::StreamConfig = supported.into();
    let device_channels = config.channels.max(1);
    let device_rate = config.sample_rate.max(1);

    // Every rate conversion happens inside the mixer — a 44.1 kHz file on a 48 kHz
    // device is read at 0.919 source frames per output frame — because the chase has to
    // be able to change that ratio anyway, and there is no separate step to convert a
    // file first.
    let mut mixer = pult_audio::Mixer::new(device_channels as usize, device_rate, MAX_DECKS);

    let for_callback = shared.clone();
    let fill = move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
        let timestamp = info.timestamp();
        for_callback.latency_micros.store(
            timestamp.playback.saturating_duration_since(timestamp.callback).as_micros() as u64,
            Ordering::Relaxed,
        );
        while let Some(command) = for_callback.commands.pop() {
            match command {
                Command::Put { deck, keep_cursor } => match mixer.put(deck, keep_cursor) {
                    Ok(Some(old)) | Err(old) => {
                        // Should the trash ever be full the deck is dropped here. It is
                        // eight times the decks that can exist, and emptied every
                        // twenty-five milliseconds.
                        let _ = for_callback.trash.push(old);
                    }
                    Ok(None) => {}
                },
                Command::Remove(id) => {
                    if let Some(old) = mixer.remove(id) {
                        if let Some(slot) = for_callback.slots.get(old.tag) {
                            slot.sounding.store(false, Ordering::Relaxed);
                        }
                        let _ = for_callback.trash.push(old);
                    }
                }
                Command::Seek(id, at) => mixer.seek(id, at),
                Command::Rate(id, rate) => mixer.set_rate(id, rate),
            }
        }
        mixer.render(out);
        for deck in mixer.decks() {
            if let Some(slot) = for_callback.slots.get(deck.tag) {
                slot.position_micros.store((deck.cursor_ms.max(0.0) * 1_000.0) as u64, Ordering::Relaxed);
                slot.sounding.store(deck.sounding, Ordering::Relaxed);
            }
        }
    };

    let complain = |e: cpal::Error| warn!("[audio] output stream: {e}");
    // Every sample format the device might want, converted from f32 on the way out.
    // The console's own audio is f32 all the way from the decoder, so this is the one
    // place a device's own format is anybody's business.
    let stream = match format {
        cpal::SampleFormat::F32 => {
            found.build_output_stream(config, fill, complain, None).map_err(|e| e.to_string())
        }
        other => build_converted(&found, &config, other, fill, complain),
    };
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            let _ = ready.send(Err(format!("{name} could not be opened: {e}")));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("{name} would not start: {e}")));
        return;
    }
    let _ = ready.send(Ok((name, device_channels, device_rate)));

    // The thread's whole remaining job is to keep the stream alive, because dropping a
    // `cpal::Stream` stops it. Woken every so often only to notice that the handle has
    // gone.
    while !shared.stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stream);
    debug!("[audio] output stream closed");
}

/// Build an output stream for a device that does not want f32.
///
/// A macro would be shorter and a match is clearer about what is actually happening:
/// each arm is the same callback with one conversion in front of it.
fn build_converted<F>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    mut fill: F,
    complain: impl FnMut(cpal::Error) + Send + 'static + Clone,
) -> Result<cpal::Stream, String>
where
    F: FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
{
    use cpal::{Sample, SampleFormat};
    macro_rules! converted {
        ($t:ty) => {{
            let mut scratch: Vec<f32> = Vec::new();
            device.build_output_stream(
                config.clone(),
                move |out: &mut [$t], info: &cpal::OutputCallbackInfo| {
                    scratch.resize(out.len(), 0.0);
                    fill(&mut scratch, info);
                    for (slot, value) in out.iter_mut().zip(scratch.iter()) {
                        *slot = <$t>::from_sample(*value);
                    }
                },
                complain.clone(),
                None,
            )
            .map_err(|e| e.to_string())
        }};
    }
    match format {
        SampleFormat::I16 => converted!(i16),
        SampleFormat::U16 => converted!(u16),
        SampleFormat::I32 => converted!(i32),
        SampleFormat::F64 => converted!(f64),
        SampleFormat::I8 => converted!(i8),
        SampleFormat::U8 => converted!(u8),
        // Anything else is a format this console has never seen a card ask for. The
        // stream is refused by name rather than played as noise.
        other => Err(format!("this console cannot play {other} samples")),
    }
}

// ── Listening, for timecode ───────────────────────────────────────────────────

/// An input stream feeding f32 samples somewhere.
///
/// Dropping it closes the device. What it sends is **one channel** — see
/// `pult_audio::ltc`: timecode on both sides of a pair is the same signal twice, and
/// summing them cancels it outright when one of them is wired backwards.
pub struct Listener {
    stop: Arc<AtomicBool>,
    pub device_name: String,
    pub sample_rate: u32,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Listener {
    pub async fn start(
        device: Option<String>,
        samples: mpsc::Sender<Vec<f32>>,
    ) -> Result<Listener, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = oneshot::channel();
        let for_thread = stop.clone();
        std::thread::Builder::new()
            .name("audio-in".into())
            .spawn(move || listen_thread(device, samples, for_thread, ready))
            .map_err(|e| format!("no thread for the timecode input: {e}"))?;

        let (device_name, sample_rate) = started
            .await
            .map_err(|_| "the timecode thread stopped before it said anything".to_string())??;
        Ok(Listener { stop, device_name, sample_rate })
    }
}

fn listen_thread(
    device: Option<String>,
    samples: mpsc::Sender<Vec<f32>>,
    stop: Arc<AtomicBool>,
    ready: oneshot::Sender<Result<(String, u32), String>>,
) {
    let host = cpal::default_host();
    let found = match find(&host, device.as_deref(), false) {
        Ok(device) => device,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let name = found
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "an audio input".to_string());
    let supported = match found.default_input_config() {
        Ok(config) => config,
        Err(e) => {
            let _ = ready.send(Err(format!("{name} will not say what it supports: {e}")));
            return;
        }
    };
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels.max(1) as usize;
    let sample_rate = config.sample_rate.max(1);

    let take = move |data: &[f32]| {
        // The first channel, and nothing else. See [`Listener`].
        let mono: Vec<f32> = data.iter().step_by(channels).copied().collect();
        // Dropped rather than awaited on a full queue, the rule every connector's
        // reader follows: a manager that is behind is behind on the whole show, and a
        // backlog of stale audio is a chase following where the timecode was.
        let _ = samples.try_send(mono);
    };

    let complain = |e: cpal::Error| warn!("[audio] timecode input: {e}");
    let stream = build_input(&found, &config, format, take, complain);
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            let _ = ready.send(Err(format!("{name} could not be opened: {e}")));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("{name} would not start: {e}")));
        return;
    }
    let _ = ready.send(Ok((name, sample_rate)));

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stream);
    debug!("[audio] timecode input closed");
}

fn build_input<F>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    mut take: F,
    complain: impl FnMut(cpal::Error) + Send + 'static + Clone,
) -> Result<cpal::Stream, String>
where
    F: FnMut(&[f32]) + Send + 'static,
{
    use cpal::{Sample, SampleFormat};
    macro_rules! converted {
        ($t:ty) => {{
            let mut scratch: Vec<f32> = Vec::new();
            device.build_input_stream(
                config.clone(),
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    scratch.clear();
                    scratch.extend(data.iter().map(|s| s.to_sample::<f32>()));
                    take(&scratch);
                },
                complain.clone(),
                None,
            )
            .map_err(|e| e.to_string())
        }};
    }
    match format {
        SampleFormat::F32 => device
            .build_input_stream(
                config.clone(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| take(data),
                complain,
                None,
            )
            .map_err(|e| e.to_string()),
        SampleFormat::I16 => converted!(i16),
        SampleFormat::U16 => converted!(u16),
        SampleFormat::I32 => converted!(i32),
        SampleFormat::F64 => converted!(f64),
        SampleFormat::I8 => converted!(i8),
        SampleFormat::U8 => converted!(u8),
        other => Err(format!("this console cannot read {other} samples")),
    }
}
