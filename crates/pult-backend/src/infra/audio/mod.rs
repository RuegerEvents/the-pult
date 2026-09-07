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
use tracing::{info, warn};
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::assets::AssetStore};
use decode::AudioCache;
use device::{AudioDevices, Listener, Player};

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
}

pub enum AudioCommand {
    /// The `timelines` collection changed. Reconcile against it.
    Timelines(Vec<Timeline>),
    /// Timecode samples, from a device or from a test.
    ///
    /// **The test seam**, and it is the whole reason the decoder is fed by a channel
    /// rather than owned by the input stream: a test can push a generated LTC stream
    /// through the real chase, the real anchor write and the real event firing without
    /// a sound card anywhere near CI.
    Timecode { timeline_id: Uuid, samples: Vec<f32>, sample_rate: u32 },
    /// Find the beats in a timeline's audio and write them to `detected`.
    Detect { timeline_id: Uuid, reply: oneshot::Sender<Result<Detected, String>> },
    Devices { reply: oneshot::Sender<AudioDevices> },
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

    /// Feed timecode straight in. See [`AudioCommand::Timecode`].
    pub async fn feed_timecode(&self, timeline_id: Uuid, samples: Vec<f32>, sample_rate: u32) {
        let _ = self.0.send(AudioCommand::Timecode { timeline_id, samples, sample_rate }).await;
    }

    pub async fn detect(&self, timeline_id: Uuid) -> Result<Detected, String> {
        let (reply, answer) = oneshot::channel();
        self.0
            .send(AudioCommand::Detect { timeline_id, reply })
            .await
            .map_err(|_| "the audio manager has stopped".to_string())?;
        answer.await.map_err(|_| "the audio manager did not answer".to_string())?
    }

    pub async fn devices(&self) -> AudioDevices {
        let (reply, answer) = oneshot::channel();
        if self.0.send(AudioCommand::Devices { reply }).await.is_err() {
            return AudioDevices::default();
        }
        answer.await.unwrap_or_default()
    }
}

/// One timeline's audio, playing on this station.
struct Playing {
    sha: String,
    player: Player,
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
    playing: HashMap<Uuid, Playing>,
    chasing: HashMap<Uuid, Chasing>,
    /// Assets whose peaks are being computed *right now*, so two passes do not start
    /// twice. Cleared when the write lands — `Timeline::peaks` is what stops it being
    /// done again, and it has to be, because the same file put back on a timeline
    /// somebody had cleared needs reducing a second time.
    reducing: HashSet<String>,
    /// Assets that would not decode. Kept apart from `reducing` and never cleared:
    /// a file that will never decode must not be decoded again every second for the
    /// rest of the show, and it is the *only* thing that should be remembered that way.
    refused: HashSet<String>,
    /// Why a timeline is not playing, keyed by timeline. Kept rather than only logged:
    /// see [`device`] on a refusal that says so.
    faults: HashMap<Uuid, String>,
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
                engine,
                net,
                prefs,
                assets,
                rx,
                samples_tx,
                samples,
                cache: AudioCache::default(),
                timelines: Vec::new(),
                playing: HashMap::new(),
                chasing: HashMap::new(),
                reducing: HashSet::new(),
                refused: HashSet::new(),
                faults: HashMap::new(),
            },
            AudioHandle(tx),
        )
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
                            self.reconcile().await;
                        }
                        AudioCommand::Timecode { timeline_id, samples, sample_rate } => {
                            self.take_timecode(timeline_id, &samples, sample_rate).await;
                        }
                        AudioCommand::Detect { timeline_id, reply } => {
                            let _ = reply.send(self.detect(timeline_id).await);
                        }
                        AudioCommand::Devices { reply } => {
                            // On a blocking thread, always. See [`device`].
                            let found = tokio::task::spawn_blocking(device::enumerate)
                                .await
                                .unwrap_or_default();
                            let _ = reply.send(found);
                        }
                    }
                }
                got = self.samples.recv() => {
                    let Some((timeline_id, samples, sample_rate)) = got else { break };
                    self.take_timecode(timeline_id, &samples, sample_rate).await;
                }
                _ = steer.tick() => self.steer().await,
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

    // ── Reconciling ───────────────────────────────────────────────────────────

    async fn reconcile(&mut self) {
        let timelines = self.timelines.clone();
        let mine: Vec<&Timeline> = timelines.iter().filter(|t| self.plays(t)).collect();
        let ids: HashSet<Uuid> = mine.iter().map(|t| t.id).collect();

        // Anything this station has stopped being responsible for. Dropping a `Player`
        // stops the sound; dropping a `Listener` closes the input device.
        self.playing.retain(|id, _| ids.contains(id));
        self.chasing.retain(|id, _| ids.contains(id));
        self.faults.retain(|id, _| ids.contains(id));

        for timeline in mine {
            self.reconcile_audio(timeline).await;
            self.reconcile_timecode(timeline).await;
            self.reconcile_peaks(timeline).await;
        }
    }

    async fn reconcile_audio(&mut self, timeline: &Timeline) {
        let wanted = timeline.audio.clone().filter(|_| timeline.running);
        let held = self.playing.get(&timeline.id).map(|p| p.sha.clone());
        match (held, wanted) {
            (Some(held), Some(wanted)) if held == wanted => {}
            (_, None) => {
                if self.playing.remove(&timeline.id).is_some() {
                    info!("[audio] stopped playing for {}", timeline.name);
                }
                self.faults.remove(&timeline.id);
            }
            (_, Some(sha)) => {
                self.playing.remove(&timeline.id);
                let from = timeline.position_at(pult_schema::clock::now_ms());
                match self.open(&sha, from).await {
                    Ok(player) => {
                        info!(
                            "[audio] {} playing through {} from {from} ms",
                            timeline.name, player.device_name
                        );
                        self.faults.remove(&timeline.id);
                        self.playing.insert(
                            timeline.id,
                            Playing {
                                sha,
                                player,
                                wrote: None,
                                seen: transport_of(timeline),
                            },
                        );
                    }
                    Err(e) => {
                        // Named on the row, not only in the log — the rule
                        // `Network::bind` follows for a cable that is not there.
                        if self.faults.get(&timeline.id) != Some(&e) {
                            warn!("[audio] {} has no sound: {e}", timeline.name);
                        }
                        self.faults.insert(timeline.id, e);
                    }
                }
            }
        }
    }

    /// Decode an asset — fetching it from a peer if this station has never seen it —
    /// and open a device on it.
    async fn open(&mut self, sha: &str, from_ms: u64) -> Result<Player, String> {
        let decoded = match self.cache.get(sha) {
            Some(decoded) => decoded,
            None => {
                let bytes = self.asset_bytes(sha).await?;
                self.cache.decode(sha, bytes).await?
            }
        };
        Player::start(
            self.prefs.output.clone(),
            decoded.samples.clone(),
            decoded.channels,
            decoded.sample_rate,
            from_ms,
        )
        .await
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
        let Some(playing) = self.playing.get_mut(&timeline.id) else { return };
        if !timeline.running {
            return;
        }
        let transport = transport_of(timeline);
        let target = timeline.position_at(now);

        // Somebody located, or pressed play from somewhere. Told apart from this
        // station's own write coming back exactly rather than by a tolerance — see
        // [`Playing::wrote`].
        if transport != playing.seen {
            playing.seen = transport;
            let ours = playing.wrote == Some((transport.0, transport.1));
            if !ours {
                playing.player.seek(target);
                playing.player.set_rate(timeline.rate.max(0.0));
                return;
            }
        }
        // The rate is the show's; the chase only ever changes it while following
        // timecode, and an internal timeline plays at whatever `rate` says.
        if (playing.player.rate() - timeline.rate).abs() > 1e-4 {
            playing.player.set_rate(timeline.rate.max(0.0));
        }
        if playing.player.finished() {
            // The file has run out. The timeline goes on — a song is shorter than the
            // act it is in — and there is nothing left to measure the anchor against.
            return;
        }

        let measured = playing.player.position_ms();
        let drift = measured as i64 - target as i64;
        if drift.abs() <= DRIFT_MS {
            return;
        }
        // The anchor is rewritten *from the sound*, which is the whole rule. One write
        // rather than two fields, through the timeline's own `locate` command, so it
        // replicates as one operation exactly the way a Go does.
        let wrote = (now, measured);
        playing.wrote = Some(wrote);
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

        // The player follows our own playhead against the timecode — resampled within
        // ±2% for a small gap, seeked for a large one.
        if let Some(playing) = self.playing.get_mut(&timeline.id) {
            match pult_audio::chase(timecode, playing.player.position_ms()) {
                pult_audio::Chase::Hold => playing.player.set_rate(1.0),
                pult_audio::Chase::Varispeed(ratio) => playing.player.set_rate(ratio),
                pult_audio::Chase::Locate(at) => {
                    playing.player.seek(at);
                    playing.player.set_rate(1.0);
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

    /// A timeline that has audio and no waveform gets one, once.
    ///
    /// Computed by **the station that plays it**, which is what stops two stations
    /// reducing the same file and racing to write the row. A station that cannot get
    /// the asset says nothing and tries again on the next tick: a peer that is still
    /// coming up is not a peer without the file.
    async fn reconcile_peaks(&mut self, timeline: &Timeline) {
        let Some(sha) = timeline.audio.clone() else { return };
        if timeline.peaks.is_some()
            || self.reducing.contains(&sha)
            || self.refused.contains(&sha)
            || self.assets.is_none()
        {
            return;
        }
        self.reducing.insert(sha.clone());

        let Ok(bytes) = self.asset_bytes(&sha).await else {
            self.reducing.remove(&sha);
            return;
        };
        let decoded = match self.cache.decode(&sha, bytes).await {
            Ok(decoded) => decoded,
            Err(e) => {
                warn!("[audio] {} could not be decoded: {e}", timeline.name);
                self.reducing.remove(&sha);
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
        let Ok(reduced) = reduced else {
            self.reducing.remove(&sha);
            return;
        };


        let Some(store) = &self.assets else { return };
        let stored = store.put(PEAKS_MIME, &reduced).await;
        // Out of `reducing` either way, and *before* the write rather than after it.
        // Leaving it in was a real defect: `reducing` is keyed by the audio's sha, so
        // the same file put back on a timeline whose `peaks` had been cleared — which
        // is exactly what replacing the audio does — was never reduced again, and the
        // panel said "the station is still reducing this file" for the rest of the
        // show. What stops the work being repeated is `Timeline::peaks` being set;
        // this set only stops it being started twice at once.
        self.reducing.remove(&sha);
        match stored {
            Ok(peaks_sha) => {
                info!(
                    "[audio] {} reduced to a waveform of {} bytes",
                    timeline.name,
                    reduced.len()
                );
                self.write(timeline.id, "peaks", serde_json::json!(peaks_sha)).await;
            }
            Err(e) => warn!("[audio] the waveform of {} could not be stored: {e}", timeline.name),
        }
    }

    /// Find the beats, and write them where the panel can draw them.
    ///
    /// The **full** model where somebody has fetched one and the small one otherwise,
    /// which is what makes `timeline.model` have a visible effect. Inference is on a
    /// blocking thread for the reason everything else here is: it is seconds of pure
    /// CPU, and seconds on the runtime is a station that runs no timer.
    async fn detect(&mut self, timeline_id: Uuid) -> Result<Detected, String> {
        let timeline = self
            .timelines
            .iter()
            .find(|t| t.id == timeline_id)
            .cloned()
            .ok_or_else(|| "no such timeline".to_string())?;
        let sha = timeline.audio.clone().ok_or_else(|| "that timeline has no audio".to_string())?;

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
        let detected = Detected {
            beats_ms: found.beats_ms.clone(),
            downbeats_ms: found.downbeats_ms.clone(),
        };
        // Written to `detected` and **not** to `grid`: the detector proposes and the
        // operator confirms. See `types::timeline::Detected`.
        self.write(timeline_id, "detected", serde_json::to_value(&detected).unwrap_or_default())
            .await;
        Ok(detected)
    }

    async fn write(&self, timeline_id: Uuid, field: &str, value: serde_json::Value) {
        let path = vec![
            PathSegment::Key("timelines".into()),
            PathSegment::Id(timeline_id),
            PathSegment::Key(field.into()),
        ];
        if let Err(e) = self.engine.set(path, Lifecycle::Persisted, value).await {
            warn!("[audio] {field} could not be written: {e}");
        }
    }

    // ── Saying what is happening ──────────────────────────────────────────────

    async fn publish_status(&mut self) {
        let now = pult_schema::clock::now_ms();
        let mut statuses = AudioStatuses::new();
        for timeline in &self.timelines {
            if !self.plays(timeline) {
                continue;
            }
            let playing = self.playing.get(&timeline.id);
            let chasing = self.chasing.get(&timeline.id);
            if playing.is_none() && chasing.is_none() && !self.faults.contains_key(&timeline.id) {
                // Nothing to say about a timeline with no audio and no timecode, which
                // is most of them: an empty row would be a panel full of "no".
                continue;
            }
            let drift = playing
                .map(|p| p.player.position_ms() as i64 - timeline.position_at(now) as i64)
                .unwrap_or(0);
            statuses.insert(
                timeline.id.to_string(),
                AudioStatus {
                    name: timeline.name.clone(),
                    playing: playing.is_some(),
                    device: playing.map(|p| p.player.device_name.clone()),
                    fault: self.faults.get(&timeline.id).cloned(),
                    drift_ms: drift.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                    ltc: chasing.map(|c| LtcStatus {
                        lock: c.lock,
                        device: c.listener.as_ref().map(|l| l.device_name.clone()),
                        fault: c.fault.clone(),
                        position_ms: c.position_ms,
                        drop_frame: c.drop_frame,
                    }),
                },
            );
        }
        if let Ok(json) = serde_json::to_value(&statuses) {
            let path = vec![PathSegment::Key("audio_status".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }
}

/// The four numbers that say where a timeline's playhead is. `running` is in it because
/// stopping and starting again in one pass would otherwise look like nothing happened.
fn transport_of(timeline: &Timeline) -> (u64, u64, f32) {
    (timeline.anchor_ms, timeline.position_at_anchor_ms, timeline.rate)
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
