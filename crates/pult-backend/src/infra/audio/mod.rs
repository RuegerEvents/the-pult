//! Sound, on a station.
//!
//! The audio half of the timeline: playing a file, drawing it, finding the beats in it,
//! and chasing somebody else's clock. `pult-audio` is the arithmetic — peaks, LTC, the
//! detector, the chase discipline — and none of it has ever met a station. This is
//! everything that has: the device, the decoder, the assets and the transport.
//!
//! An actor in the shape [`crate::infra::connectors::input::InputManager`] is in, and
//! deliberately so: told the `timelines` collection whenever it changes, reconciling
//! what it is running against that collection on a one-second timer as well, and
//! publishing a LOCAL status row every second. A cable coming or going, a failover, and
//! an operator switching a device are the same act to all three managers.
//!
//! # One station plays, and the callback's clock is the reference
//!
//! [`Timeline::node_id`] says which — or the leader, the rule
//! `OutputConfig::runs_on` follows for Art-Net. **A tablet plays nothing**: Web Audio
//! was declined outright, because the reference clock must not be a tab a browser can
//! throttle to one frame a minute when somebody switches to their email.
//!
//! # One stream, a deck per timeline
//!
//! A timeline's sound is a list of clips, each with its own place, trim and destination
//! — see `types::timeline::AudioClip`. Every clip of every running timeline this station
//! plays is mixed into **one** output stream at the device's full channel count, which
//! is how a Dante Virtual Soundcard's sixty-four channels become sixty-four zones.
//! `pult_audio::mix` is the arithmetic; [`device::Output`] is the stream it runs in.
//! Each running timeline is a *deck* in that mixer, with its own cursor advanced by the
//! one callback — so each timeline anchors itself, and two timelines on one station
//! share one clock and cannot drift apart.
//!
//! The device is open while anything this station plays has clips, running or not,
//! because opening a device takes longer than a scare is allowed to take to start; and
//! every clip's file is decoded as soon as it is on a timeline, for the same reason.
//!
//! And the reference *is* the sound. The playhead this manager reads is the position
//! the audio callback has actually delivered, minus what the device says is still in
//! front of the loudspeaker — not a wall clock, because a sound card's crystal and a
//! computer's are two different crystals and the one the audience can hear is the one
//! that has to be right. When that playhead differs from where the show's anchor says
//! the timeline is by more than [`DRIFT_MS`], this station rewrites the anchor, and
//! every other station and every browser follows the sound.
//!
//! **Under twenty milliseconds it does nothing at all**, and that rule is the whole
//! reason this is safe. The anchor is the *show's*: it is SYNCED, it crosses the link,
//! every browser evaluates fades against it. A station that rewrote it every buffer
//! would be putting a few hundred bytes and a few milliseconds of jitter on the network
//! forty times a second for a correction nobody can perceive — and worse, the browsers'
//! playheads would visibly stutter. Twenty milliseconds is `pult_schema::clock`'s own
//! figure, and it is the same one for the same reason.
//!
//! # Audio never gates the transport
//!
//! A timeline with no audio, or one on a station with no device, runs exactly as task
//! 66 left it. Everything here is a correction *applied to* a transport that works
//! without it — which is what makes "the audio device is missing" a status row rather
//! than a show that will not start.
//!
//! # Chasing
//!
//! With [`TimelineSource::Ltc`], the station named for the audio also opens the input
//! device and feeds `pult_audio::ltc::Decoder`. On lock it writes the anchor **from the
//! timecode's position**, never from its own playhead — the timecode is the authority
//! and our own sound is the thing being corrected. Where there is audio too, the
//! player gets `pult_audio::lock::chase`'s answer: a small drift is resampled away
//! within ±2%, a large one is a seek.
//!
//! **Lock loss stops nothing.** The timeline goes on running from the last anchor and
//! the status says "LTC lost" until frames come back, because a dropout on a timecode
//! cable in the middle of an act is not a reason for the show to stop.
//!
//! **And chasing does not press Play.** Whether a timeline is running is the operator's
//! to decide; this steers a running one. A console that started the act because a
//! machine down the corridor began rolling would be a surprise nobody asked for.

pub mod decode;
pub mod device;
pub mod models;
pub mod monitor;
pub mod nodes;

use std::collections::{HashMap, HashSet};
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::PathSegment,
    types::timeline::{
        AudioStatus, AudioStatuses, Detected, LtcLock, LtcRate, LtcStatus, Timeline,
        TimelineSource,
    },
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::assets::AssetStore};
use decode::AudioCache;
use device::{AudioDevices, Listener, Output, MAX_DECKS};
use pult_schema::types::timeline::{AtEnd, AudioClip, AudioOutput};
use pult_schema::types::zone::{station_channels, SpeakerZone};

/// How far the sound may be from the show's anchor before the anchor is rewritten.
///
/// `pult_schema::clock`'s own threshold, and the same one `pult_audio::lock` chases to.
/// See the module header.
pub const DRIFT_MS: i64 = 20;

/// The mime a reduced waveform is stored under.
pub const PEAKS_MIME: &str = "application/vnd.pult.peaks";

/// What the `[audio]` section of `preferences.toml` says.
///
/// A station preference and never show data, which barely needs arguing: which sound
/// card is in which machine is a fact about the machine, and the same show opened on
/// the desk and on the stage rack must not fight over it. Both optional, and told
/// nothing is the system default — see [`device`] on why a *named* device that is not
/// here refuses rather than falling back.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudioPrefs {
    /// Where a timeline's audio comes out.
    pub output: Option<String>,
    /// Where timecode comes in.
    pub input: Option<String>,
    /// How far ahead a stream to an OpenHaunt node is sent: the playout delay the node
    /// buffers. 150 ms when unsaid, which rides out ordinary wifi and still feels
    /// immediate for a triggered scare.
    pub node_stream_delay_ms: Option<u32>,
    /// What this station calls its sound cards and their channels, keyed by the
    /// device's own name — the key `output` already matches by. Names only: nothing
    /// plays differently for them, but a zone mapped to "FOH L, FOH R" is a mapping
    /// somebody can check, and one mapped to "1, 2" on a 64-channel card is not.
    /// A fact about the wiring of this machine, so never show data.
    pub names: std::collections::BTreeMap<String, DeviceNames>,
}

/// One sound card's names.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DeviceNames {
    /// What it is called here: "Garden Dante" rather than "Dante Virtual Soundcard".
    pub alias: Option<String>,
    /// Channel names, the first being channel 1. Empty is unnamed. A list rather than a
    /// map because TOML keys are strings and a channel is a number.
    pub channels: Vec<String>,
}

pub enum AudioCommand {
    /// The `timelines` collection changed. Reconcile against it.
    Timelines(Vec<Timeline>),
    /// The `speaker_zones` collection changed: where each zone is, on every station.
    Zones(Vec<SpeakerZone>),
    /// Timecode samples, from a device or from a test.
    ///
    /// **The test seam**, and it is the whole reason the decoder is fed by a channel
    /// rather than owned by the input stream: a test can push a generated LTC stream
    /// through the real chase, the real anchor write and the real event firing without
    /// a sound card anywhere near CI.
    Timecode { timeline_id: Uuid, samples: Vec<f32>, sample_rate: u32 },
    /// Find the beats in one of a timeline's clips — the first, unless another is
    /// named — and write them to `detected`, in timeline positions.
    Detect {
        timeline_id: Uuid,
        clip_id: Option<Uuid>,
        reply: oneshot::Sender<Result<Detected, String>>,
    },
    Devices { reply: oneshot::Sender<AudioDevices> },
    /// A page wants to listen: chunks of one timeline's sound, or every running one's,
    /// sent until the receiver is dropped. See [`monitor`].
    Monitor { timeline: Option<Uuid>, chunks: mpsc::Sender<Vec<u8>> },
    #[allow(dead_code, reason = "Stop has no caller until the server shuts down gracefully")]
    Stop,
}

#[derive(Clone)]
pub struct AudioHandle(pub mpsc::Sender<AudioCommand>);

impl AudioHandle {
    /// Hand over the timelines. Never blocks the engine, the way
    /// [`crate::infra::connectors::input::InputHandle::timelines`] does not.
    pub fn timelines(&self, timelines: Vec<Timeline>) {
        let _ = self.0.try_send(AudioCommand::Timelines(timelines));
    }

    /// Hand over the speaker zones. Never blocks the engine either.
    pub fn zones(&self, zones: Vec<SpeakerZone>) {
        let _ = self.0.try_send(AudioCommand::Zones(zones));
    }

    /// Feed timecode straight in. See [`AudioCommand::Timecode`].
    pub async fn feed_timecode(&self, timeline_id: Uuid, samples: Vec<f32>, sample_rate: u32) {
        let _ = self.0.send(AudioCommand::Timecode { timeline_id, samples, sample_rate }).await;
    }

    pub async fn detect(&self, timeline_id: Uuid, clip_id: Option<Uuid>) -> Result<Detected, String> {
        let (reply, answer) = oneshot::channel();
        self.0
            .send(AudioCommand::Detect { timeline_id, clip_id, reply })
            .await
            .map_err(|_| "the audio manager has stopped".to_string())?;
        answer.await.map_err(|_| "the audio manager did not answer".to_string())?
    }

    /// Listen: one timeline's sound, or every running timeline's, as monitor chunks
    /// until the receiver is dropped. See [`monitor`].
    pub async fn monitor(&self, timeline: Option<Uuid>) -> Option<mpsc::Receiver<Vec<u8>>> {
        let (chunks, rx) = mpsc::channel(monitor::QUEUE);
        self.0.send(AudioCommand::Monitor { timeline, chunks }).await.ok()?;
        Some(rx)
    }

    pub async fn devices(&self) -> AudioDevices {
        let (reply, answer) = oneshot::channel();
        if self.0.send(AudioCommand::Devices { reply }).await.is_err() {
            return AudioDevices::default();
        }
        answer.await.unwrap_or_default()
    }
}

/// One timeline's deck, playing on this station.
struct Playing {
    /// Which of the output's slots its cursor is read back from.
    slot: usize,
    /// Everything the deck was built from, so a pass that changes nothing about the
    /// sound sends nothing to the callback. A clip dragged while the timeline runs does
    /// change it, and the rebuilt deck carries on from the old one's cursor.
    built: String,
    /// The transport this manager last *wrote*, so its own write coming back round
    /// is not mistaken for an operator locating.
    ///
    /// The same problem `model/timelines.rs` solves with `Seen`, and the same answer:
    /// exact comparison rather than a tolerance. A tolerance here would either miss a
    /// small locate or chase this station's own echo for ever.
    wrote: Option<(u64, u64)>,
    /// The transport as it stood at the end of the last pass.
    seen: (u64, u64, f32),
}

/// One timeline chasing timecode.
struct Chasing {
    decoder: pult_audio::ltc::Decoder,
    /// `None` where the samples are being pushed in by a test rather than a device.
    listener: Option<Listener>,
    rate: LtcRate,
    offset_frames: i64,
    lock: LtcLock,
    position_ms: Option<u64>,
    drop_frame: Option<bool>,
    fault: Option<String>,
}

/// Owns the devices, the decoded files and what is playing.
pub struct AudioManager {
    node_id: NodeId,
    /// What this station calls its cards and channels, and when the preferences file
    /// it was read from last changed: re-read when that moves, so a name typed on this
    /// station's page reaches the others without a restart.
    names: (Option<std::time::SystemTime>, std::collections::BTreeMap<String, DeviceNames>),
    /// The `station_audio` row last written, so it is written only when it moves.
    station_said: Option<pult_schema::types::StationAudio>,
    /// What this machine is called, for a fault that has to say which station has not
    /// mapped a zone — the status row of the one station the browser is on.
    station_name: String,
    engine: EngineHandle,
    net: crate::infra::net::NetHandle,
    prefs: AudioPrefs,
    /// Where the audio and the peaks live. `None` with no show open, which is the one
    /// state in which there is nothing to play and nowhere to put a waveform.
    assets: Option<AssetStore>,
    rx: mpsc::Receiver<AudioCommand>,
    /// Timecode samples from the input threads, and from the test seam.
    samples_tx: mpsc::Sender<(Uuid, Vec<f32>, u32)>,
    samples: mpsc::Receiver<(Uuid, Vec<f32>, u32)>,
    cache: AudioCache,
    timelines: Vec<Timeline>,
    /// Where each zone is, on every station. See `types::zone`.
    zones: Vec<SpeakerZone>,
    /// The one stream every clip here is mixed into. `None` when nothing this station
    /// plays has a clip.
    output: Option<Output>,
    /// Why there is no stream, where one is wanted.
    output_fault: Option<String>,
    playing: HashMap<Uuid, Playing>,
    chasing: HashMap<Uuid, Chasing>,
    /// The waveform asset of every file this station has reduced, by the file's sha.
    reduced: HashMap<String, String>,
    /// The clips whose waveform this station has written, and for which file — so a
    /// copy of the timeline that has not caught up with that write is not written again.
    peaks_written: HashMap<Uuid, String>,
    /// Assets that would not decode. Kept apart from `reduced` and never cleared:
    /// a file that will never decode must not be decoded again every second for the
    /// rest of the show, and it is the *only* thing that should be remembered that way.
    refused: HashSet<String>,
    /// Why a timeline is not playing, keyed by timeline. Kept rather than only logged:
    /// see [`device`] on a refusal that says so.
    faults: HashMap<Uuid, String>,
    /// Why a clip is not playing while the rest of its timeline is, keyed by timeline
    /// and then by clip.
    clip_faults: HashMap<Uuid, std::collections::BTreeMap<String, String>>,
    /// Which of this station's channels each clip comes out of, keyed by timeline and
    /// then by clip: its zone through this station's map.
    clip_channels: HashMap<Uuid, std::collections::BTreeMap<String, Vec<u16>>>,
    /// What the OpenHaunt nodes' loudspeakers hold and are told. See [`nodes`].
    nodes: nodes::Nodes,
    /// The pages listening. Asked for, never published: with none, nothing is rendered.
    monitors: Vec<monitor::Listener>,
}

impl AudioManager {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        net: crate::infra::net::NetHandle,
        prefs: AudioPrefs,
        assets: Option<AssetStore>,
    ) -> (Self, AudioHandle) {
        let (tx, rx) = mpsc::channel(8);
        // Deep enough for a burst of input buffers, shallow enough that a manager that
        // has stalled does not accumulate seconds of stale timecode — the rule the
        // input connector's frame queue follows.
        let (samples_tx, samples) = mpsc::channel(64);
        (
            AudioManager {
                node_id,
                // The name every other panel already knows this machine by: the
                // `stations` row's hostname. Cheap, and asked once.
                station_name: sysinfo::System::host_name().unwrap_or_else(|| "this station".into()),
                engine,
                net,
                prefs: prefs.clone(),
                assets,
                rx,
                samples_tx,
                samples,
                cache: AudioCache::default(),
                timelines: Vec::new(),
                zones: Vec::new(),
                output: None,
                output_fault: None,
                playing: HashMap::new(),
                chasing: HashMap::new(),
                reduced: HashMap::new(),
                peaks_written: HashMap::new(),
                refused: HashSet::new(),
                faults: HashMap::new(),
                clip_faults: HashMap::new(),
                clip_channels: HashMap::new(),
                nodes: nodes::Nodes::new(prefs.node_stream_delay_ms),
                monitors: Vec::new(),
                names: (None, prefs.names.clone()),
                station_said: None,
            },
            AudioHandle(tx),
        )
    }

    /// Where the OpenHaunt nodes are and how to talk to them, so a clip can be routed
    /// to one. A manager without them plays only its own device.
    pub fn with_nodes(
        mut self,
        directory: tokio::sync::watch::Receiver<crate::infra::devices::DeviceDirectory>,
        devices: crate::infra::devices::DeviceHandle,
    ) -> Self {
        self.nodes.attach(directory, devices);
        self
    }

    pub async fn run(mut self) {
        info!("[audio] started");
        let period = std::time::Duration::from_secs(1);
        let mut report = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        // The drift check runs far more often than the status: twenty milliseconds is
        // the threshold, so looking once a second would let a station wander a whole
        // second between corrections. Twenty-five is one output frame's period, which
        // is a rate this console already lives at.
        let mut steer = tokio::time::interval(std::time::Duration::from_millis(25));
        steer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        AudioCommand::Stop => break,
                        AudioCommand::Timelines(timelines) => {
                            self.timelines = timelines;
                            self.nodes.saw_timelines();
                            self.reconcile().await;
                        }
                        AudioCommand::Zones(zones) => {
                            self.zones = zones;
                            self.nodes.saw_timelines();
                            self.reconcile().await;
                        }
                        AudioCommand::Timecode { timeline_id, samples, sample_rate } => {
                            self.take_timecode(timeline_id, &samples, sample_rate).await;
                        }
                        AudioCommand::Detect { timeline_id, clip_id, reply } => {
                            let _ = reply.send(self.detect(timeline_id, clip_id).await);
                        }
                        AudioCommand::Devices { reply } => {
                            // On a blocking thread, always. See [`device`].
                            let found = tokio::task::spawn_blocking(device::enumerate)
                                .await
                                .unwrap_or_default();
                            let _ = reply.send(found);
                        }
                        AudioCommand::Monitor { timeline, chunks } => {
                            self.monitors.push(monitor::Listener::new(timeline, chunks));
                        }
                    }
                }
                got = self.samples.recv() => {
                    let Some((timeline_id, samples, sample_rate)) = got else { break };
                    self.take_timecode(timeline_id, &samples, sample_rate).await;
                }
                Some(done) = self.nodes.results.recv() => self.node_request_done(done).await,
                _ = steer.tick() => {
                    self.steer().await;
                    self.feed_monitors(pult_schema::clock::now_ms()).await;
                    self.schedule_nodes(pult_schema::clock::now_ms());
                }
                _ = report.tick() => {
                    // Where a device appearing or disappearing is acted on, and where a
                    // failover changes which station is playing — neither changes the
                    // show, so nothing else in this loop would ever notice.
                    self.reconcile().await;
                    self.publish_status().await;
                }
            }
        }
        info!("[audio] stopped");
    }

    /// Does this station play this timeline?
    ///
    /// `node_id`, or the leader where it names nobody — the rule `OutputConfig::runs_on`
    /// follows, and for the same reason: *something* has to play, a lone console is the
    /// leader, and a second one joining stops the double-play rather than starting a
    /// fight about it.
    fn plays(&self, timeline: &Timeline) -> bool {
        match timeline.node_id {
            Some(node) => node == self.node_id,
            None => self.net.standing().is_leader,
        }
    }

    /// The clips of a timeline this station mixes itself, each with the channels its
    /// zone is here — or why it has none. A clip only a node plays is not in it.
    fn station_clips<'t>(&self, timeline: &'t Timeline) -> Vec<(&'t AudioClip, Result<Vec<u16>, String>)> {
        timeline
            .clips
            .iter()
            .filter_map(|clip| {
                station_channels(&clip.destination, &self.zones, self.node_id, &self.station_name)
                    .map(|channels| (clip, channels))
            })
            .collect()
    }

    // ── Reconciling ───────────────────────────────────────────────────────────

    async fn reconcile(&mut self) {
        let timelines = self.timelines.clone();
        let mine: Vec<&Timeline> = timelines.iter().filter(|t| self.plays(t)).collect();
        let ids: HashSet<Uuid> = mine.iter().map(|t| t.id).collect();

        // Anything this station has stopped being responsible for. Dropping a
        // `Listener` closes the input device; a deck is taken out of the mixer.
        let gone: Vec<Uuid> = self.playing.keys().filter(|id| !ids.contains(id)).copied().collect();
        for id in gone {
            self.stop_deck(id);
        }
        self.chasing.retain(|id, _| ids.contains(id));
        self.faults.retain(|id, _| ids.contains(id));
        self.clip_faults.retain(|id, _| ids.contains(id));
        self.clip_channels.retain(|id, _| ids.contains(id));

        self.reconcile_output(&mine).await;
        for timeline in mine {
            self.reconcile_deck(timeline).await;
            self.reconcile_timecode(timeline).await;
            self.reconcile_peaks(timeline).await;
        }
        if let Some(output) = &self.output {
            output.collect();
        }
        self.reconcile_nodes().await;
    }

    /// Open the device while anything this station plays has a clip on it, and close
    /// it when nothing does.
    ///
    /// Open while those timelines are *stopped* too: a scare is a timeline a flow
    /// starts, and opening CoreAudio takes longer than a scare may take to begin.
    async fn reconcile_output(&mut self, mine: &[&Timeline]) {
        let wanted = mine.iter().any(|t| !self.station_clips(t).is_empty());
        if !wanted {
            if self.output.take().is_some() {
                info!("[audio] nothing here has sound any more; the output is closed");
            }
            self.playing.clear();
            self.output_fault = None;
            return;
        }
        if self.output.is_some() {
            return;
        }
        match Output::open(self.prefs.output.clone()).await {
            Ok(output) => {
                info!(
                    "[audio] mixing into {} ({} channels at {} Hz)",
                    output.device_name, output.channels, output.sample_rate
                );
                self.output = Some(output);
                self.output_fault = None;
                // A new stream has no decks in it, whatever this map says.
                self.playing.clear();
            }
            Err(e) => {
                // Named on the row, not only in the log — the rule `Network::bind`
                // follows for a cable that is not there.
                if self.output_fault.as_ref() != Some(&e) {
                    warn!("[audio] no output: {e}");
                }
                self.output_fault = Some(e);
            }
        }
    }

    /// A timeline's deck: built from its clips while it runs, and gone when it stops.
    async fn reconcile_deck(&mut self, timeline: &Timeline) {
        let clips = self.station_clips(timeline);
        // Every file decoded as soon as it is on a timeline, so that pressing play — or a
        // flow pressing it — does not wait for an mp3.
        let mut voices = Vec::new();
        let mut faults = std::collections::BTreeMap::new();
        let mut routed = std::collections::BTreeMap::new();
        let device_channels = self.output.as_ref().map(|o| o.channels as usize);
        for (clip, channels) in &clips {
            // A zone this station has not mapped is said by name before anything is
            // decoded: there is nothing to play it through, and folding it onto channels
            // this card has would be somebody else's speaker.
            let channels = match channels {
                Ok(channels) => channels,
                Err(e) => {
                    faults.insert(clip.id.to_string(), e.clone());
                    continue;
                }
            };
            routed.insert(clip.id.to_string(), channels.clone());
            match self.voice(clip, channels, device_channels).await {
                Ok(Some(voice)) => voices.push(voice),
                // Decoded, with no device to route it to: the timeline's own fault says
                // why, once, rather than every clip saying it again.
                Ok(None) => {}
                Err(e) => {
                    faults.insert(clip.id.to_string(), e);
                }
            }
        }
        if routed.is_empty() {
            self.clip_channels.remove(&timeline.id);
        } else {
            self.clip_channels.insert(timeline.id, routed.clone());
        }
        if self.clip_faults.get(&timeline.id).map_or(!faults.is_empty(), |held| *held != faults) {
            for (clip, fault) in &faults {
                warn!("[audio] a clip on {} is not playing ({clip}): {fault}", timeline.name);
            }
        }
        if faults.is_empty() {
            self.clip_faults.remove(&timeline.id);
        } else {
            self.clip_faults.insert(timeline.id, faults);
        }

        if !timeline.running || clips.is_empty() {
            if self.playing.contains_key(&timeline.id) {
                info!("[audio] stopped playing for {}", timeline.name);
            }
            self.stop_deck(timeline.id);
            self.faults.remove(&timeline.id);
            return;
        }
        let Some(output) = &self.output else {
            if let Some(fault) = &self.output_fault {
                self.faults.insert(timeline.id, fault.clone());
            }
            return;
        };
        self.faults.remove(&timeline.id);

        // The channels each clip resolved to are in it, so re-mapping a zone while it
        // sounds rebuilds the deck rather than leaving it on the channels it had.
        let built = serde_json::to_string(&(&timeline.clips, &timeline.end, voices.len(), &routed))
            .unwrap_or_default();
        let held = self.playing.get(&timeline.id);
        if held.is_some_and(|p| p.built == built) {
            return;
        }
        let slot = match held {
            Some(playing) => playing.slot,
            None => {
                let taken: HashSet<usize> = self.playing.values().map(|p| p.slot).collect();
                match (0..MAX_DECKS).find(|slot| !taken.contains(slot)) {
                    Some(slot) => slot,
                    None => {
                        self.faults.insert(
                            timeline.id,
                            format!("{MAX_DECKS} timelines are already sounding on this station"),
                        );
                        return;
                    }
                }
            }
        };
        let now = pult_schema::clock::now_ms();
        let latency = output.latency_ms();
        // Started a latency ahead, so that what comes out of the loudspeaker is where
        // the show is — see [`Output::position_ms`]. A timeline played with its anchor
        // still to come (a count-in) holds its position until then, so the deck waits
        // in silence and starts a latency before the anchor, from where it will be.
        let (from, wait_ms) = match timeline.anchor_ms.checked_sub(now + latency) {
            Some(wait) if wait > 0 => (folded(timeline, timeline.position_at_anchor_ms), wait),
            _ => (folded(timeline, timeline.position_at(now)) + latency, 0),
        };
        let mut deck = pult_audio::Deck::new(timeline.id.as_u128(), from as f64, voices);
        deck.wait_ms = wait_ms as f64;
        deck.tag = slot;
        deck.rate = timeline.rate.max(0.0) as f64;
        deck.end = deck_end(timeline);
        let keep_cursor = held.is_some();
        match output.put(deck, keep_cursor) {
            Ok(()) => {
                if !keep_cursor {
                    info!(
                        "[audio] {} playing through {} from {} ms",
                        timeline.name,
                        output.device_name,
                        from
                    );
                }
                let seen = transport_of(timeline);
                let wrote = self.playing.get(&timeline.id).and_then(|p| p.wrote);
                self.playing.insert(timeline.id, Playing { slot, built, wrote, seen });
            }
            // Tried again on the next pass, with nothing remembered as built.
            Err(e) => warn!("[audio] {} could not start: {e}", timeline.name),
        }
    }

    fn stop_deck(&mut self, timeline_id: Uuid) {
        if self.playing.remove(&timeline_id).is_some() {
            if let Some(output) = &self.output {
                output.remove(timeline_id.as_u128());
            }
        }
    }

    /// One clip, decoded and routed to its zone's channels on the open device.
    /// `Ok(None)` is decoded with no device to route it to.
    async fn voice(
        &mut self,
        clip: &AudioClip,
        channels: &[u16],
        device_channels: Option<usize>,
    ) -> Result<Option<pult_audio::Voice>, String> {
        let decoded = self.decoded(&clip.asset).await?;
        let Some(device_channels) = device_channels else { return Ok(None) };
        let routes = pult_audio::mix::route(decoded.channels as usize, channels, device_channels)
            .map_err(|e| e.to_string())?;
        Ok(Some(pult_audio::Voice {
            source: pult_audio::Source {
                samples: decoded.samples.clone(),
                channels: decoded.channels as usize,
                sample_rate: decoded.sample_rate,
            },
            placement: placement(clip),
            routes,
        }))
    }

    /// A file, decoded, from the cache or from wherever its bytes are. A file that
    /// would not decode is remembered as such and not tried again.
    async fn decoded(&mut self, sha: &str) -> Result<std::sync::Arc<decode::Decoded>, String> {
        if self.refused.contains(sha) {
            return Err("the file would not decode".into());
        }
        if let Some(decoded) = self.cache.get(sha) {
            return Ok(decoded);
        }
        let bytes = self.asset_bytes(sha).await?;
        match self.cache.decode(sha, bytes).await {
            Ok(decoded) => Ok(decoded),
            Err(e) => {
                self.refused.insert(sha.to_string());
                Err(e)
            }
        }
    }

    /// An asset's bytes, from here or from a peer.
    ///
    /// The same path a plugin bundle takes, and for the same reason: a file uploaded at
    /// front of house has to reach the station backstage, and it cannot ride the oplog.
    async fn asset_bytes(&self, sha: &str) -> Result<Vec<u8>, String> {
        let store = self.assets.as_ref().ok_or_else(|| "no show is open".to_string())?;
        if let Ok(Some(asset)) = store.get(sha).await {
            return Ok(asset.bytes);
        }
        let peers = crate::infra::assets::peer_addresses(&self.engine, self.node_id.0).await;
        if peers.is_empty() {
            return Err("this station has not got the audio, and there is no peer to ask".into());
        }
        match crate::infra::assets::fetch_from_peers(store, sha, &peers).await {
            Ok(crate::infra::assets::Fetched::Got(asset)) => Ok(asset.bytes),
            Ok(crate::infra::assets::Fetched::NobodyHasIt) => {
                Err("no station in this session has the audio file".into())
            }
            Ok(crate::infra::assets::Fetched::Unreachable(n)) => {
                Err(format!("{n} station(s) could not be asked for the audio"))
            }
            Err(e) => Err(format!("the audio could not be fetched: {e}")),
        }
    }

    // ── Steering ──────────────────────────────────────────────────────────────

    /// The twenty-five millisecond pass: keep the show and the sound in step.
    async fn steer(&mut self) {
        let now = pult_schema::clock::now_ms();
        let timelines = self.timelines.clone();
        for timeline in &timelines {
            if !self.plays(timeline) {
                continue;
            }
            match &timeline.source {
                TimelineSource::Ltc { .. } => self.steer_to_timecode(timeline, now).await,
                TimelineSource::Internal => self.steer_to_the_sound(timeline, now).await,
            }
        }
    }

    /// The internal case: the sound is the reference and the anchor follows it.
    async fn steer_to_the_sound(&mut self, timeline: &Timeline, now: u64) {
        let Some(output) = &self.output else { return };
        let Some(playing) = self.playing.get_mut(&timeline.id) else { return };
        if !timeline.running {
            return;
        }
        let id = timeline.id.as_u128();
        let transport = transport_of(timeline);
        // Counting in: the deck is waiting for the anchor and there is no sound yet to
        // steer by — measured against a held playhead, the wait would read as drift and
        // be "corrected" by moving the anchor to now, which is the count-in gone. A
        // transport moved meanwhile (played again, located) builds the deck afresh.
        if now < timeline.anchor_ms + output.latency_ms() {
            if transport != playing.seen {
                self.stop_deck(timeline.id);
            }
            return;
        }
        let target = folded(timeline, timeline.position_at(now));
        let measured = output.position_ms(playing.slot);
        let drift = measured as i64 - target as i64;

        // Somebody located, or pressed play from somewhere. Told apart from this
        // station's own write coming back exactly rather than by a tolerance — see
        // [`Playing::wrote`]. And followed only where the sound is not already there:
        // the deck wraps itself at a looping timeline's end, so the leader's relocate
        // there arrives where the sound already is, and seeking to it would be a click.
        if transport != playing.seen {
            playing.seen = transport;
            let ours = playing.wrote == Some((transport.0, transport.1));
            if !ours {
                if drift.abs() > DRIFT_MS {
                    output.seek(id, target + output.latency_ms());
                }
                output.set_rate(id, timeline.rate.max(0.0));
                return;
            }
        }
        if !output.sounding(playing.slot) {
            // Nothing is audible — between two clips, or after the last one. The
            // timeline goes on, and there is nothing anybody can hear to be in step with.
            return;
        }
        if drift.abs() <= DRIFT_MS {
            return;
        }
        // The anchor is rewritten *from the sound*, which is the whole rule. One write
        // rather than two fields, through the timeline's own `locate` command, so it
        // replicates as one operation exactly the way a Go does.
        playing.wrote = Some((now, measured));
        playing.seen = (now, measured, timeline.rate);
        self.locate(timeline.id, measured, now).await;
    }

    /// The chasing case: the timecode is the reference and *we* are what is corrected.
    async fn steer_to_timecode(&mut self, timeline: &Timeline, now: u64) {
        let Some(chasing) = self.chasing.get(&timeline.id) else { return };
        if chasing.lock != LtcLock::Locked {
            // Lock loss stops nothing. See the module header.
            return;
        }
        let Some(timecode) = chasing.position_ms else { return };

        // The deck follows our own playhead against the timecode — resampled within
        // ±2% for a small gap, seeked for a large one.
        if let (Some(output), Some(playing)) = (&self.output, self.playing.get(&timeline.id)) {
            let id = timeline.id.as_u128();
            match pult_audio::chase(timecode, output.position_ms(playing.slot)) {
                pult_audio::Chase::Hold => output.set_rate(id, 1.0),
                pult_audio::Chase::Varispeed(ratio) => output.set_rate(id, ratio),
                pult_audio::Chase::Locate(at) => {
                    output.seek(id, at + output.latency_ms());
                    output.set_rate(id, 1.0);
                }
            }
        }

        if !timeline.running {
            return;
        }
        // And the show's anchor is written **from the timecode**, never from our
        // playhead: the generator is the authority, and anchoring to our own sound
        // would make every other station follow this one's drift instead of the code.
        let drift = timecode as i64 - timeline.position_at(now) as i64;
        if drift.abs() <= DRIFT_MS {
            return;
        }
        if let Some(playing) = self.playing.get_mut(&timeline.id) {
            playing.wrote = Some((now, timecode));
            playing.seen = (now, timecode, timeline.rate);
        }
        self.locate(timeline.id, timecode, now).await;
    }

    /// Move the show's playhead, as the timeline's own command.
    ///
    /// `Authorship::none()` by construction — [`EngineHandle::set`] writes as nobody —
    /// which is right: this is the console keeping itself in step, not an operator
    /// doing something, and it has no business in the History panel or in anyone's
    /// undo stack.
    async fn locate(&self, timeline_id: Uuid, position_ms: u64, at: u64) {
        let path = vec![
            PathSegment::Key("timelines".into()),
            PathSegment::Id(timeline_id),
            PathSegment::Key("locate".into()),
        ];
        let args = serde_json::json!({ "positionMs": position_ms, "at": at });
        if let Err(e) = self.engine.set(path, Lifecycle::Synced, args).await {
            warn!("[audio] the anchor could not be moved: {e}");
        }
    }

    // ── Timecode ──────────────────────────────────────────────────────────────

    async fn reconcile_timecode(&mut self, timeline: &Timeline) {
        let TimelineSource::Ltc { fps, offset_frames } = &timeline.source else {
            self.chasing.remove(&timeline.id);
            return;
        };
        if let Some(chasing) = self.chasing.get_mut(&timeline.id) {
            // A rate or an offset changed under a running chase: neither needs the
            // device reopened, and reopening it would drop lock for no reason.
            chasing.rate = *fps;
            chasing.offset_frames = *offset_frames;
            // A decoder that has heard nothing for four frames has lost it.
            chasing.lock = lock_state(chasing.decoder.lock(patience_ms(*fps)));
            return;
        }

        let samples = self.samples_tx.clone();
        let id = timeline.id;
        let (relay_tx, mut relay) = mpsc::channel::<Vec<f32>>(64);
        let listener = Listener::start(self.prefs.input.clone(), relay_tx).await;
        let (listener, sample_rate, fault) = match listener {
            Ok(listener) => {
                info!("[audio] {} chasing timecode from {}", timeline.name, listener.device_name);
                let rate = listener.sample_rate;
                (Some(listener), rate, None)
            }
            Err(e) => {
                warn!("[audio] {} cannot chase timecode: {e}", timeline.name);
                // 48 kHz stands in so the decoder exists and the status row can say
                // why there is no lock. A decoder with nothing feeding it costs
                // nothing and is what makes the fault visible rather than absent.
                (None, 48_000, Some(e))
            }
        };
        if listener.is_some() {
            // The device's own thread hands buffers to this relay; the relay tags them
            // with the timeline and puts them in the manager's one queue, which is the
            // same queue the test seam writes to. One path, so a test exercises the
            // code a cable does.
            tokio::spawn(async move {
                while let Some(buffer) = relay.recv().await {
                    if samples.send((id, buffer, sample_rate)).await.is_err() {
                        break;
                    }
                }
            });
        }
        self.chasing.insert(
            timeline.id,
            Chasing {
                decoder: pult_audio::ltc::Decoder::new(sample_rate),
                listener,
                rate: *fps,
                offset_frames: *offset_frames,
                lock: LtcLock::Waiting,
                position_ms: None,
                drop_frame: None,
                fault,
            },
        );
    }

    /// Samples in, a position out.
    async fn take_timecode(&mut self, timeline_id: Uuid, samples: &[f32], sample_rate: u32) {
        let Some(chasing) = self.chasing.get_mut(&timeline_id) else { return };
        // A test can push at a rate the decoder was not made for — and so can a device
        // that has come back at a different rate after being unplugged.
        if chasing.decoder.sample_rate() != sample_rate {
            chasing.decoder = pult_audio::ltc::Decoder::new(sample_rate);
        }
        let frames = chasing.decoder.push(samples);
        if let Some(last) = frames.last() {
            let rate = ltc_rate(chasing.rate);
            let at = last.to_ms(rate) as i64;
            // The offset is in *frames* because that is the unit a machine room talks
            // in, and it is subtracted: a reel starting at `01:00:00:00` is offset by
            // an hour's worth of frames and the show starts at zero.
            let frame_ms = 1_000.0 / frames_per_second(rate);
            let offset_ms = (chasing.offset_frames as f64 * frame_ms).round() as i64;
            chasing.position_ms = Some((at - offset_ms).max(0) as u64);
            chasing.drop_frame = Some(last.drop_frame);
        }
        chasing.lock = lock_state(chasing.decoder.lock(patience_ms(chasing.rate)));
    }

    // ── Peaks, and the detector ───────────────────────────────────────────────

    /// A clip with no waveform gets one, once.
    ///
    /// Computed by **the station that plays it**, which is what stops two stations
    /// reducing the same file and racing to write the row. A station that cannot get
    /// the asset says nothing and tries again on the next tick: a peer that is still
    /// coming up is not a peer without the file.
    ///
    /// Remembered twice over, and both were a defect first. A file is reduced once
    /// however many clips and timelines use it (`reduced`). And a clip is *written*
    /// once (`peaks_written`): `clips` is one column, so the write is the whole list,
    /// and the copies of the timeline arriving here are behind this manager's own
    /// writes — each stale copy without the waveform started the reduction and the
    /// write again, which on a timeline with two files went on for ever.
    async fn reconcile_peaks(&mut self, timeline: &Timeline) {
        let missing: Vec<&AudioClip> = timeline
            .clips
            .iter()
            .filter(|clip| clip.peaks.is_none())
            .filter(|clip| self.peaks_written.get(&clip.id) != Some(&clip.asset))
            .collect();
        if missing.is_empty() || self.assets.is_none() {
            return;
        }
        let files: std::collections::BTreeSet<String> = missing.iter().map(|c| c.asset.clone()).collect();
        for sha in files {
            if !self.reduced.contains_key(&sha) && !self.refused.contains(&sha) {
                self.reduce(&timeline.name, sha).await;
            }
        }
        let mut clips = timeline.clips.clone();
        let mut wrote = Vec::new();
        for clip in clips.iter_mut().filter(|c| c.peaks.is_none()) {
            if let Some(peaks) = self.reduced.get(&clip.asset) {
                clip.peaks = Some(peaks.clone());
                wrote.push((clip.id, clip.asset.clone()));
            }
        }
        if wrote.is_empty() {
            return;
        }
        let clips = serde_json::to_value(&clips).unwrap_or_default();
        if self.write(timeline.id, "clips", clips).await {
            self.peaks_written.extend(wrote);
        }
    }

    /// Decode a file and store its waveform, into `reduced`.
    async fn reduce(&mut self, name: &str, sha: String) {
        let Ok(bytes) = self.asset_bytes(&sha).await else { return };
        let decoded = match self.cache.decode(&sha, bytes).await {
            Ok(decoded) => decoded,
            Err(e) => {
                warn!("[audio] a clip on {name} could not be decoded: {e}");
                self.refused.insert(sha);
                return;
            }
        };
        // On a blocking thread: reducing four minutes of stereo is ten million
        // comparisons, which is not the runtime's to do.
        let samples = decoded.samples.clone();
        let channels = decoded.channels as usize;
        let sample_rate = decoded.sample_rate;
        let reduced = tokio::task::spawn_blocking(move || {
            pult_audio::Peaks::compute(&samples, channels, sample_rate).encode()
        })
        .await;
        let Ok(reduced) = reduced else { return };
        let Some(store) = &self.assets else { return };
        match store.put(PEAKS_MIME, &reduced).await {
            Ok(peaks_sha) => {
                info!("[audio] a clip on {name} reduced to a waveform of {} bytes", reduced.len());
                self.reduced.insert(sha, peaks_sha);
            }
            Err(e) => warn!("[audio] the waveform of a clip on {name} could not be stored: {e}"),
        }
    }

    /// Find the beats, and write them where the panel can draw them.
    ///
    /// The **full** model where somebody has fetched one and the small one otherwise,
    /// which is what makes `timeline.model` have a visible effect. Inference is on a
    /// blocking thread for the reason everything else here is: it is seconds of pure
    /// CPU, and seconds on the runtime is a station that runs no timer.
    ///
    /// On one clip — the first, unless another is named — and answered in *timeline*
    /// positions: a beat at two seconds into a file that starts at ten is a beat at
    /// twelve, and one in the part of the file the trim leaves out is no beat at all.
    async fn detect(&mut self, timeline_id: Uuid, clip_id: Option<Uuid>) -> Result<Detected, String> {
        let timeline = self
            .timelines
            .iter()
            .find(|t| t.id == timeline_id)
            .cloned()
            .ok_or_else(|| "no such timeline".to_string())?;
        let clip = match clip_id {
            Some(id) => timeline.clips.iter().find(|c| c.id == id),
            None => timeline.clips.first(),
        }
        .cloned()
        .ok_or_else(|| "that timeline has no audio".to_string())?;
        let sha = clip.asset.clone();

        let bytes = self.asset_bytes(&sha).await?;
        let decoded = self.cache.decode(&sha, bytes).await?;
        let full = models::full_model().await;

        let samples = decoded.samples.clone();
        let channels = decoded.channels as usize;
        let sample_rate = decoded.sample_rate;
        let found = tokio::task::spawn_blocking(move || {
            let mut detector = match full {
                Some(bytes) => pult_audio::Detector::full(bytes),
                None => pult_audio::Detector::small(),
            }
            .map_err(|e| e.to_string())?;
            detector.detect(&samples, channels, sample_rate).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("the detector stopped: {e}"))??;

        info!(
            "[audio] {} — {} beats, {} downbeats",
            timeline.name,
            found.beats_ms.len(),
            found.downbeats_ms.len()
        );
        let on_the_timeline = |file_ms: &Vec<u32>| -> Vec<u32> {
            file_ms
                .iter()
                .filter_map(|at| clip_to_timeline(&clip, *at as u64))
                .map(|at| at.min(u32::MAX as u64) as u32)
                .collect()
        };
        let detected = Detected {
            beats_ms: on_the_timeline(&found.beats_ms),
            downbeats_ms: on_the_timeline(&found.downbeats_ms),
        };
        // Written to `detected` and **not** to `grid`: the detector proposes and the
        // operator confirms. See `types::timeline::Detected`.
        self.write(timeline_id, "detected", serde_json::to_value(&detected).unwrap_or_default())
            .await;
        Ok(detected)
    }

    /// Whether it was written.
    async fn write(&self, timeline_id: Uuid, field: &str, value: serde_json::Value) -> bool {
        let path = vec![
            PathSegment::Key("timelines".into()),
            PathSegment::Id(timeline_id),
            PathSegment::Key(field.into()),
        ];
        match self.engine.set(path, Lifecycle::Persisted, value).await {
            Ok(_) => true,
            Err(e) => {
                warn!("[audio] {field} could not be written: {e}");
                false
            }
        }
    }

    // ── Saying what is happening ──────────────────────────────────────────────

    async fn publish_status(&mut self) {
        let now = pult_schema::clock::now_ms();
        let mut statuses = AudioStatuses::new();
        // The driving station's, read from its report where that is not this one: a
        // station that does not drive the nodes cannot tell whether one is playing.
        let all_node_faults = self.node_faults().await;
        for timeline in &self.timelines {
            // A timeline another station plays still gets a row here where a node clip
            // of it is not playing, so every station's pages say so.
            let node_faults = all_node_faults.get(&timeline.id);
            if !self.plays(timeline) && node_faults.is_none() {
                continue;
            }
            let playing = self.playing.get(&timeline.id);
            let chasing = self.chasing.get(&timeline.id);
            // Each clip's faults put against what its destination reaches: silent where
            // every member failed, half heard where some did.
            let station_faults = self.clip_faults.get(&timeline.id);
            let mut clip_faults = std::collections::BTreeMap::new();
            let mut clip_partial = std::collections::BTreeMap::new();
            let no_nodes = std::collections::BTreeMap::new();
            for clip in &timeline.clips {
                let id = clip.id.to_string();
                let station = station_faults.and_then(|f| f.get(&id)).map(String::as_str);
                let nodes = node_faults.and_then(|f| f.get(&id)).unwrap_or(&no_nodes);
                match pult_schema::types::zone::clip_health(&clip.destination, &self.zones, station, nodes) {
                    pult_schema::types::zone::ClipHealth::Fine => {}
                    pult_schema::types::zone::ClipHealth::Partial(failed) => {
                        clip_partial.insert(id, failed);
                    }
                    pult_schema::types::zone::ClipHealth::Silent(why) => {
                        clip_faults.insert(id, why);
                    }
                }
            }
            if playing.is_none()
                && chasing.is_none()
                && !self.faults.contains_key(&timeline.id)
                && clip_faults.is_empty()
                && clip_partial.is_empty()
            {
                // Nothing to say about a timeline with no audio and no timecode, which
                // is most of them: an empty row would be a panel full of "no".
                continue;
            }
            let drift = match (playing, &self.output) {
                (Some(p), Some(output)) if output.sounding(p.slot) => {
                    output.position_ms(p.slot) as i64 - folded(timeline, timeline.position_at(now)) as i64
                }
                _ => 0,
            };
            statuses.insert(
                timeline.id.to_string(),
                AudioStatus {
                    name: timeline.name.clone(),
                    playing: playing.is_some(),
                    device: playing.and(self.output.as_ref()).map(|o| o.device_name.clone()),
                    fault: self.faults.get(&timeline.id).cloned(),
                    drift_ms: drift.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                    ltc: chasing.map(|c| LtcStatus {
                        lock: c.lock,
                        device: c.listener.as_ref().map(|l| l.device_name.clone()),
                        fault: c.fault.clone(),
                        position_ms: c.position_ms,
                        drop_frame: c.drop_frame,
                    }),
                    clip_faults,
                    clip_partial,
                    clip_channels: self.clip_channels.get(&timeline.id).cloned().unwrap_or_default(),
                },
            );
        }
        if let Ok(json) = serde_json::to_value(&statuses) {
            let path = vec![PathSegment::Key("audio_status".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
        let output = AudioOutput {
            open: self.output.is_some(),
            device: self.output.as_ref().map(|o| o.device_name.clone()),
            channels: self.output.as_ref().map_or(0, |o| o.channels),
            sample_rate: self.output.as_ref().map_or(0, |o| o.sample_rate),
            fault: self.output_fault.clone(),
        };
        if let Ok(json) = serde_json::to_value(&output) {
            let path = vec![PathSegment::Key("audio_output".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
        self.publish_station_audio().await;
    }

    /// Say what this station plays through and what it calls it, for the other
    /// stations' zone matrices — and only when that moved.
    async fn publish_station_audio(&mut self) {
        // The names are this machine's preference and may have been edited since start.
        let modified = crate::infra::preferences::path()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        if modified.is_some() && modified != self.names.0 {
            self.names = (modified, crate::infra::preferences::load().audio.names);
        }
        let device = self.output.as_ref().map(|o| o.device_name.clone()).or_else(|| self.prefs.output.clone());
        let named = device.as_ref().and_then(|d| self.names.1.get(d)).cloned().unwrap_or_default();
        let channels = self
            .output
            .as_ref()
            .map_or(named.channels.len() as u16, |o| o.channels);
        let row = pult_schema::types::StationAudio {
            id: self.node_id.0,
            device,
            alias: named.alias,
            channels,
            channel_names: named.channels,
        };
        let Ok(value) = serde_json::to_value(&row) else { return };
        let path = vec![PathSegment::Key("station_audio".into()), PathSegment::Id(row.id)];
        // Unchanged *and still there*: joining a session replaces this station's state
        // with the leader's, which has never held this row, so "what I last said" is not
        // the same as "what the show holds" — and a row said once and lost is a spare
        // nobody can map.
        if self.station_said.as_ref() == Some(&row)
            && self.engine.get(path.clone()).await.is_ok_and(|held| held == value)
        {
            return;
        }
        if self.engine.set(path, Lifecycle::Synced, value.clone()).await.is_err() {
            let create = vec![PathSegment::Key("station_audio".into()), PathSegment::Key("__create".into())];
            if let Err(e) = self.engine.set(create, Lifecycle::Synced, value).await {
                debug!("[audio] could not publish this station's card: {e}");
                return;
            }
        }
        self.station_said = Some(row);
    }
}

/// The four numbers that say where a timeline's playhead is. `running` is in it because
/// stopping and starting again in one pass would otherwise look like nothing happened.
fn transport_of(timeline: &Timeline) -> (u64, u64, f32) {
    (timeline.anchor_ms, timeline.position_at_anchor_ms, timeline.rate)
}


/// A clip's place, as the mixer wants it.
pub(super) fn placement(clip: &AudioClip) -> pult_audio::Placement {
    pult_audio::Placement {
        start_ms: clip.start_ms as f64,
        offset_ms: clip.offset_ms as f64,
        length_ms: clip.length_ms.map(|l| l as f64),
        looped: clip.looped,
        gain: clip.gain(),
        fade_in_ms: clip.fade_in_ms as f64,
        fade_out_ms: clip.fade_out_ms as f64,
    }
}

/// Where a timeline ends, as a deck needs it. An end at zero is no end, the rule
/// `model/timelines.rs` follows, or a loop there would wrap on every frame.
pub(super) fn deck_end(timeline: &Timeline) -> Option<pult_audio::mix::End> {
    timeline.end.filter(|end| end.at_ms > 0).map(|end| pult_audio::mix::End {
        at_ms: end.at_ms as f64,
        looped: end.then == AtEnd::Loop,
    })
}

/// Where the timeline's sound is at a running position, its end applied: held at a
/// stopping end, and round again at a looping one.
///
/// What the transport says and what the deck does differ for the moment between the
/// deck reaching the end and the leader relocating the transport there. Measuring drift
/// against the unfolded number would read that moment as a whole loop's worth of drift
/// and rewrite the anchor for it.
pub(super) fn folded(timeline: &Timeline, position_ms: u64) -> u64 {
    match timeline.end.filter(|end| end.at_ms > 0) {
        Some(end) if position_ms >= end.at_ms => match end.then {
            AtEnd::Loop => position_ms % end.at_ms,
            AtEnd::Stop => end.at_ms,
        },
        _ => position_ms,
    }
}

/// A position in a clip's file, as a position on its timeline. `None` where the trim
/// leaves it out. A looped clip answers its first time round.
fn clip_to_timeline(clip: &AudioClip, file_ms: u64) -> Option<u64> {
    if file_ms < clip.offset_ms {
        return None;
    }
    let into = file_ms - clip.offset_ms;
    if clip.length_ms.is_some_and(|length| into >= length) {
        return None;
    }
    Some(clip.start_ms + into)
}

/// The schema's rate, as the codec's. Two spellings of five values, converted in one
/// place, because `pult-audio` depends on no pult crate — the wall `pult-render` was
/// split out over.
pub fn ltc_rate(rate: LtcRate) -> pult_audio::LtcRate {
    match rate {
        LtcRate::F24 => pult_audio::LtcRate::F24,
        LtcRate::F25 => pult_audio::LtcRate::F25,
        LtcRate::F30 => pult_audio::LtcRate::F30,
        LtcRate::F2997 => pult_audio::LtcRate::F2997,
        LtcRate::F2997Df => pult_audio::LtcRate::F2997Df,
    }
}

/// The decoder's lock, as the panel spells it. Two spellings again, for the reason
/// [`ltc_rate`] has two.
fn lock_state(lock: pult_audio::Lock) -> LtcLock {
    match lock {
        pult_audio::Lock::Waiting => LtcLock::Waiting,
        pult_audio::Lock::Locked => LtcLock::Locked,
        pult_audio::Lock::Lost => LtcLock::Lost,
    }
}

fn frames_per_second(rate: pult_audio::LtcRate) -> f64 {
    match rate {
        pult_audio::LtcRate::F24 => 24.0,
        pult_audio::LtcRate::F25 => 25.0,
        pult_audio::LtcRate::F30 => 30.0,
        pult_audio::LtcRate::F2997 | pult_audio::LtcRate::F2997Df => 30_000.0 / 1_001.0,
    }
}

/// How long a gap in the frames counts as lock lost.
///
/// Four frames at the declared rate, which is 133 ms at 30 fps and 167 at 24 — a
/// number rather than a constant, because four frames of a slow rate is longer than
/// four frames of a fast one and a console that used one figure would call 24 fps
/// unlocked between every frame.
fn patience_ms(rate: LtcRate) -> u32 {
    (4_000.0 / frames_per_second(ltc_rate(rate))).round() as u32
}

#[cfg(test)]
mod tests;
