//! Input: reading DMX off a wire, so that a recording can exist.
//!
//! The mirror of [`super::OutputManager`], and deliberately the same shape: an actor
//! that reconciles what it is running against the `inputs` collection on a one-second
//! timer and whenever the collection changes, so a cable coming or going and an
//! operator switching a row off are the same act.
//!
//! # What arrives never becomes show state
//!
//! A universe is 512 bytes forty times a second *per source*. Putting that in
//! `ShowState` would replicate somebody else's opinion of the rig across the link
//! carrying this one's, so the merged images stay here, LOCAL, and are drawn on demand
//! through the very same [`super::Viewers`] table an output's wire view uses — the
//! input's row id standing in for an output's, which is what lets a peer watch this
//! station's input with no new message on the sync protocol.
//!
//! Two things cross out of here, and both are deliberate acts:
//!
//! **A grab** decodes what is on the wire *now* for a selection and writes it into the
//! programmer. **A recording** does the same continuously against a timeline's
//! position and writes one asset at the end. Neither sends a byte: both send
//! `(fixture, parameter, value)`, because the patch is what says what a byte meant and
//! a recording of bytes could never be played back onto a rig that had been repatched.
//!
//! # Merging
//!
//! Per patch universe, a table of sources: an sACN CID or — for Art-Net, which carries
//! no source identity at all — the address the datagram came from. **Highest priority
//! present wins, HTP among equals, and a source that has said nothing for
//! [`SOURCE_TIMEOUT`] is gone.** Art-Net has no priority byte, so every one of its
//! sources sits at the same number and the rule degenerates to plain HTP, which is
//! what Art-Net's own specification says to do.
//!
//! The timeout is what makes a console being unplugged different from a console
//! sending zeros: the first drops out of the merge and lets whatever else is there
//! through, and the second is a frame of zeros that wins on HTP against nothing.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use chrono::Utc;
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        fixture::{Fixture, FixtureType, ParameterValue},
        input::{InputConfig, InputKind, InputStatus, InputStatuses},
        network::NetService,
        output::{OutputSection, OutputView, SectionBody, UniverseFrame, UniverseSummary, UniverseTraffic},
        timeline::{Timeline, TimelineTrack},
    },
};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{info, warn};
use uuid::Uuid;

use super::{dmx::Patch, Viewers, VIEW_MS};
use crate::{engine::EngineHandle, infra::assets::AssetStore};

/// How long a source may say nothing before it is dropped from the merge.
///
/// E1.31's own network data loss timeout. A sender at 40 Hz that has missed a hundred
/// frames is gone; one that is merely idle is still sending its keep-alive, which is
/// why the DMX family has one at all.
pub const SOURCE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2_500);

/// What a decoded value is, wherever it goes next.
pub type Decoded = Vec<(Uuid, String, ParameterValue)>;

pub enum InputCommand {
    /// The `inputs` collection changed. Reconcile against it.
    Configure(Vec<InputConfig>),
    /// The `timelines` collection changed: what is armed and where its playhead is.
    Timelines(Vec<Timeline>),
    /// Decode what is on the wire now for these fixtures.
    Grab {
        input_id: Option<Uuid>,
        fixture_ids: Vec<Uuid>,
        reply: oneshot::Sender<Result<Decoded, String>>,
    },
    #[allow(dead_code, reason = "Stop has no caller until the server shuts down gracefully")]
    Stop,
}

#[derive(Clone)]
pub struct InputHandle(pub mpsc::Sender<InputCommand>);

impl InputHandle {
    /// Hand over the configured inputs. Never blocks the engine, the way
    /// [`super::OutputHandle::configure`] does not.
    pub fn configure(&self, inputs: Vec<InputConfig>) {
        let _ = self.0.try_send(InputCommand::Configure(inputs));
    }

    /// Hand over the timelines: which is armed, and where each playhead is.
    pub fn timelines(&self, timelines: Vec<Timeline>) {
        let _ = self.0.try_send(InputCommand::Timelines(timelines));
    }

    /// What these fixtures are being told to do, right now, by whoever is on the wire.
    ///
    /// Awaited rather than dropped on a full queue, unlike the two above: a grab is an
    /// operator pressing a button and a silently skipped one is a button that did
    /// nothing.
    pub async fn grab(
        &self,
        input_id: Option<Uuid>,
        fixture_ids: Vec<Uuid>,
    ) -> Result<Decoded, String> {
        let (reply, answer) = oneshot::channel();
        self.0
            .send(InputCommand::Grab { input_id, fixture_ids, reply })
            .await
            .map_err(|_| "the input manager has stopped".to_string())?;
        answer.await.map_err(|_| "the input manager did not answer".to_string())?
    }
}

/// Which sender a frame came from.
///
/// Two spellings because the two protocols differ in exactly this: E1.31 carries a
/// CID that is stable for the life of a sender, and Art-Net carries nothing at all, so
/// the datagram's source address is the only identity there is. Which means a guest
/// console that changes its IP looks like a new Art-Net source and an old one that
/// times out — the right behaviour, and one nothing here can improve on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SourceId {
    Cid([u8; 16]),
    Addr(IpAddr),
}

/// One frame, parsed, on its way from a reader task to the manager.
struct Received {
    input_id: Uuid,
    source: SourceId,
    priority: u8,
    wire_universe: u16,
    channels: [u8; super::dmx::UNIVERSE_SIZE],
}

/// What one sender is currently asserting about one universe.
struct Source {
    priority: u8,
    last_seen: std::time::Instant,
    channels: [u8; super::dmx::UNIVERSE_SIZE],
}

/// One patch universe as several senders make it.
struct Merged {
    sources: HashMap<SourceId, Source>,
    image: [u8; super::dmx::UNIVERSE_SIZE],
    /// When the merged image last became different bytes — not when a packet last
    /// arrived. The wire viewer prints both, for the reason the output side does: "is
    /// anything moving" and "is this universe still being fed" are different
    /// questions, and a keep-alive answers only the second.
    changed_at: std::time::Instant,
    received_at: std::time::Instant,
}

impl Merged {
    fn new() -> Self {
        let now = std::time::Instant::now();
        Merged {
            sources: HashMap::new(),
            image: [0; super::dmx::UNIVERSE_SIZE],
            changed_at: now,
            received_at: now,
        }
    }

    /// Fold the live sources into one image. True when the bytes actually changed.
    fn remerge(&mut self, now: std::time::Instant) -> bool {
        self.sources.retain(|_, source| now.duration_since(source.last_seen) < SOURCE_TIMEOUT);
        let mut merged = [0u8; super::dmx::UNIVERSE_SIZE];
        if let Some(top) = self.sources.values().map(|source| source.priority).max() {
            for source in self.sources.values().filter(|source| source.priority == top) {
                for (slot, byte) in merged.iter_mut().zip(source.channels.iter()) {
                    *slot = (*slot).max(*byte);
                }
            }
        }
        if merged == self.image {
            return false;
        }
        self.image = merged;
        self.changed_at = now;
        true
    }
}

/// One listening input: the socket's reader, and what it has heard.
struct Listening {
    config: InputConfig,
    /// Which cable it was opened on, so a reconcile can tell a rebuild from a rename.
    wire: Option<Ipv4Addr>,
    status: InputStatus,
    reader: tokio::task::JoinHandle<()>,
    /// Merged images, keyed by **patch** universe: the map has already been applied,
    /// so nothing downstream ever sees a wire number.
    universes: BTreeMap<u16, Merged>,
    packets_since_report: u32,
    reported_at: std::time::Instant,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// A take in progress.
struct Recording {
    timeline_id: Uuid,
    input_id: Uuid,
    /// The rig as it was when the take started, restricted to the fixtures this input
    /// can actually reach.
    ///
    /// Read once: a recording that re-read the patch per frame would be doing the
    /// engine's work forty times a second, and a repatch mid-take would silently
    /// change what the earlier half of the recording meant. Restricted, because
    /// `Patch::decode` walks every placed parameter and there is no reason for a take
    /// off one universe to walk a rig of five thousand.
    patch: Patch,
    /// The change points so far, per parameter, in a stable order so two encodings of
    /// one take are the same bytes.
    points: BTreeMap<(Uuid, String), Vec<pult_render::TrackPoint>>,
    /// The last value written per parameter, so a value that did not move writes
    /// nothing. This is the whole of why a track is small.
    last: HashMap<(Uuid, String), ParameterValue>,
}

/// What the manager needs in order to be looked at, the same pair the output side has.
struct Watching {
    viewers: Viewers,
    updates: crate::engine::UpdateBroadcast,
    wake: watch::Receiver<u64>,
    last: HashMap<(Uuid, Option<String>), serde_json::Value>,
    drawn_at: std::time::Instant,
}

/// Owns the listening sockets and what they have heard.
pub struct InputManager {
    node_id: NodeId,
    engine: EngineHandle,
    net: crate::infra::net::NetHandle,
    /// Where a finished take goes. `None` with no show open, which is also the one
    /// state in which there is nowhere to put one — so arming a recording there fails
    /// with a message rather than losing a take at the end of it.
    assets: Option<AssetStore>,
    rx: mpsc::Receiver<InputCommand>,
    /// Frames from every reader task, parsed. One channel rather than a select over N
    /// sockets: a socket is read by its own task and the manager reads one queue,
    /// which is also what makes stopping an input an `abort` rather than a dance.
    frames_tx: mpsc::Sender<Received>,
    frames: mpsc::Receiver<Received>,
    listening: HashMap<Uuid, Listening>,
    configured: Vec<InputConfig>,
    timelines: Vec<Timeline>,
    recording: Option<Recording>,
    watching: Option<Watching>,
}

impl InputManager {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        net: crate::infra::net::NetHandle,
        assets: Option<AssetStore>,
    ) -> (Self, InputHandle) {
        let (tx, rx) = mpsc::channel(8);
        // Deep enough that a burst across several universes does not drop a frame of
        // a take, shallow enough that a manager that has stalled does not accumulate
        // seconds of stale wire. A dropped frame here is a change point that was
        // never recorded, which the stepwise replay simply holds through.
        let (frames_tx, frames) = mpsc::channel(256);
        (
            Self {
                node_id,
                engine,
                net,
                assets,
                rx,
                frames_tx,
                frames,
                listening: HashMap::new(),
                configured: Vec::new(),
                timelines: Vec::new(),
                recording: None,
                watching: None,
            },
            InputHandle(tx),
        )
    }

    /// Let somebody look at what is arriving, through the same table an output's
    /// viewers land in. Set after construction, the way the output manager's is.
    pub fn watchable(
        mut self,
        viewers: Viewers,
        updates: crate::engine::UpdateBroadcast,
    ) -> Self {
        self.watching = Some(Watching {
            wake: viewers.subscribe(),
            viewers,
            updates,
            last: HashMap::new(),
            drawn_at: std::time::Instant::now(),
        });
        self
    }

    pub async fn run(mut self) {
        info!("[input] started");
        let period = std::time::Duration::from_secs(1);
        let mut report = tokio::time::interval_at(tokio::time::Instant::now() + period, period);

        loop {
            let next_view = self.next_view_at().unwrap_or_else(|| {
                std::time::Instant::now() + std::time::Duration::from_secs(3600)
            });

            tokio::select! {
                biased;
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        InputCommand::Stop => break,
                        InputCommand::Configure(inputs) => self.reconcile(inputs).await,
                        InputCommand::Timelines(timelines) => {
                            self.timelines = timelines;
                            self.reconcile_recording().await;
                        }
                        InputCommand::Grab { input_id, fixture_ids, reply } => {
                            let _ = reply.send(self.grab(input_id, &fixture_ids).await);
                        }
                    }
                }
                frame = self.frames.recv() => {
                    let Some(frame) = frame else { break };
                    self.take_frame(frame);
                }
                _ = report.tick() => {
                    self.measure_rates();
                    self.publish_status().await;
                    // Where a cable coming or going is acted on, exactly as on the
                    // output side: neither changes the show, so nothing else in this
                    // loop would ever notice.
                    self.reconcile(self.configured.clone()).await;
                }
                changed = wait_for_viewers(self.watching.as_mut()) => {
                    let _ = changed;
                }
                _ = tokio::time::sleep_until(next_view.into()) => {
                    self.draw_views().await;
                }
            }
        }
        info!("[input] stopped");
    }

    fn next_view_at(&self) -> Option<std::time::Instant> {
        let watching = self.watching.as_ref()?;
        watching
            .viewers
            .any_on(self.node_id)
            .then(|| watching.drawn_at + std::time::Duration::from_millis(VIEW_MS))
    }

    // ── The wire ──────────────────────────────────────────────────────────────

    /// One parsed frame, merged into the universe it maps to.
    fn take_frame(&mut self, frame: Received) {
        let Some(input) = self.listening.get_mut(&frame.input_id) else { return };
        let Some(patch_universe) = input.config.patch_universe(frame.wire_universe) else {
            // A universe this row does not map. Arriving is not the same as being
            // wanted: an sACN group is joined per mapped universe, but a broadcast
            // Art-Net node puts every universe it has on the same port.
            return;
        };
        let now = std::time::Instant::now();
        input.packets_since_report += 1;
        input.status.last_packet = Some(Utc::now());

        let merged = input.universes.entry(patch_universe).or_insert_with(Merged::new);
        merged.received_at = now;
        merged.sources.insert(
            frame.source,
            Source { priority: frame.priority, last_seen: now, channels: frame.channels },
        );
        let moved = merged.remerge(now);
        input.status.sources = input.universes.values().map(|m| m.sources.len()).sum::<usize>()
            as u16;

        if moved {
            self.record_change(frame.input_id);
        }
    }

    /// The merged images of one input, or of every input this station holds.
    fn images_of(&self, input_id: Option<Uuid>) -> HashMap<u16, [u8; super::dmx::UNIVERSE_SIZE]> {
        let mut images = HashMap::new();
        for (id, input) in &self.listening {
            if input_id.is_some_and(|wanted| wanted != *id) {
                continue;
            }
            for (universe, merged) in &input.universes {
                images.insert(*universe, merged.image);
            }
        }
        images
    }

    // ── Grab ──────────────────────────────────────────────────────────────────

    /// What the wire is saying about these fixtures, as values.
    async fn grab(&self, input_id: Option<Uuid>, fixture_ids: &[Uuid]) -> Result<Decoded, String> {
        if let Some(wanted) = input_id {
            if !self.listening.contains_key(&wanted) {
                return Err(format!("this station is not listening on input {wanted}"));
            }
        }
        let images = self.images_of(input_id);
        if images.is_empty() {
            return Err("nothing has arrived on that input yet".to_string());
        }
        let (fixtures, types) = self.read_rig().await?;
        let wanted: Vec<Fixture> =
            fixtures.into_iter().filter(|f| fixture_ids.contains(&f.id)).collect();
        if wanted.is_empty() {
            return Err("none of those fixtures are patched".to_string());
        }
        Ok(Patch::new(wanted, types, Vec::new()).decode(&images))
    }

    async fn read_rig(&self) -> Result<(Vec<Fixture>, Vec<FixtureType>), String> {
        let rig = self
            .engine
            .get(vec![PathSegment::Key("fixtures".into())])
            .await
            .map_err(|e| format!("cannot read the rig: {e}"))?;
        let types = self
            .engine
            .get(vec![PathSegment::Key("fixture_types".into())])
            .await
            .map_err(|e| format!("cannot read the fixture types: {e}"))?;
        Ok((
            serde_json::from_value(rig).unwrap_or_default(),
            serde_json::from_value(types).unwrap_or_default(),
        ))
    }

    // ── Recording ─────────────────────────────────────────────────────────────

    /// Start, or finish, a take because the timelines said so.
    ///
    /// Armed is a *state* on the timeline rather than an act, so this is a
    /// reconciliation like the socket one next to it: a timeline that is running and
    /// armed at an input this station holds should be recording, and anything else
    /// should not.
    async fn reconcile_recording(&mut self) {
        // Resolved to a pair of ids before anything is decided, so the match below
        // does not hold a borrow of `self.timelines` across the writes it makes.
        let wanted: Option<(Uuid, Uuid)> = self.timelines.iter().find_map(|timeline| {
            let input = timeline.recording?;
            (timeline.running && self.listening.contains_key(&input))
                .then_some((timeline.id, input))
        });
        let current = self.recording.as_ref().map(|r| (r.timeline_id, r.input_id));

        match (current, wanted) {
            (Some(current), Some(wanted)) if current == wanted => {}
            (_, Some((timeline_id, input_id))) => {
                self.finish_recording().await;
                self.start_recording(timeline_id, input_id).await;
            }
            (Some(_), None) => self.finish_recording().await,
            (None, None) => {}
        }
    }

    async fn start_recording(&mut self, timeline_id: Uuid, input_id: Uuid) {
        if self.assets.is_none() {
            warn!("[input] cannot record: this console has no show open to put a take in");
            return;
        }
        let carried: Vec<u16> = match self.listening.get(&input_id) {
            Some(input) => input.config.universes.values().copied().collect(),
            None => return,
        };
        let (fixtures, types) = match self.read_rig().await {
            Ok(rig) => rig,
            Err(e) => {
                warn!("[input] cannot record: {e}");
                return;
            }
        };
        // Only the fixtures this input can reach — see `Recording::patch`.
        let reachable: Vec<Fixture> = fixtures
            .into_iter()
            .filter(|fixture| {
                fixture.address.breaks().iter().any(|b| carried.contains(&b.universe))
            })
            .collect();
        if reachable.is_empty() {
            warn!("[input] recording armed, but no fixture is patched in the universes it carries");
        }
        info!(
            "[input] recording {} fixture(s) into timeline {timeline_id}",
            reachable.len()
        );
        self.recording = Some(Recording {
            timeline_id,
            input_id,
            patch: Patch::new(reachable, types, Vec::new()),
            points: BTreeMap::new(),
            last: HashMap::new(),
        });
        // Whatever is already on the wire is the value at the position the take
        // starts from, so a parameter that never moves again is still in the file.
        self.record_change(input_id);
    }

    /// A merged image moved: append what changed, at the position it moved at.
    fn record_change(&mut self, input_id: Uuid) {
        let Some(recording) = &self.recording else { return };
        if recording.input_id != input_id {
            return;
        }
        let Some(timeline) =
            self.timelines.iter().find(|t| t.id == recording.timeline_id && t.running)
        else {
            return;
        };
        // The timeline's own position, not a wall clock: a take made at half rate
        // plays back at the positions it was recorded against.
        let at_ms = timeline.position_at(pult_schema::types::sequence::now_ms()).min(u32::MAX as u64)
            as u32;
        let images = self.images_of(Some(input_id));
        let decoded = recording.patch.decode(&images);

        let Some(recording) = &mut self.recording else { return };
        for (fixture_id, key, value) in decoded {
            let at = (fixture_id, key);
            // A value that did not move writes nothing. This is the whole reason a
            // forty-minute take of a rig where six heads move is the size of six
            // heads moving.
            if recording.last.get(&at) == Some(&value) {
                continue;
            }
            recording.last.insert(at.clone(), value.clone());
            recording.points.entry(at).or_default().push(pult_render::TrackPoint {
                ms: at_ms,
                value,
            });
        }
    }

    /// Write the take out and disarm.
    async fn finish_recording(&mut self) {
        let Some(recording) = self.recording.take() else { return };
        let Some(assets) = &self.assets else { return };

        let track = pult_render::Track {
            keys: recording
                .points
                .into_iter()
                .map(|((fixture_id, key), points)| pult_render::TrackKey {
                    fixture_id,
                    key,
                    points,
                })
                .collect(),
        };
        if track.keys.is_empty() {
            info!("[input] the take caught nothing; nothing written");
            self.disarm(recording.timeline_id).await;
            return;
        }
        let bytes = pult_render::encode(&track);
        let sha = match assets.put(crate::infra::assets::TRACK_MIME, &bytes).await {
            Ok(sha) => sha,
            Err(e) => {
                warn!("[input] the take could not be stored: {e}");
                self.disarm(recording.timeline_id).await;
                return;
            }
        };
        info!(
            "[input] take stored: {} parameter(s), {} bytes",
            track.keys.len(),
            bytes.len()
        );

        // Read the row back rather than remembering it: a take runs for the length of
        // a song, and a `tracks` list held from before it started would drop whatever
        // somebody else added in the meantime.
        let path = vec![
            PathSegment::Key("timelines".into()),
            PathSegment::Id(recording.timeline_id),
            PathSegment::Key("tracks".into()),
        ];
        let mut tracks: Vec<TimelineTrack> = self
            .engine
            .get(path.clone())
            .await
            .ok()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        tracks.push(TimelineTrack {
            id: Uuid::new_v4(),
            name: format!("Take {}", tracks.len() + 1),
            asset: sha,
            input_id: Some(recording.input_id),
            offset_ms: 0,
            enabled: true,
        });
        if let Ok(value) = serde_json::to_value(&tracks) {
            if let Err(e) = self.engine.set(path, Lifecycle::Persisted, value).await {
                warn!("[input] the take was stored but could not be added to the timeline: {e}");
            }
        }
        self.disarm(recording.timeline_id).await;
    }

    /// Clear `recording`, which is what tells every station the take is over.
    async fn disarm(&self, timeline_id: Uuid) {
        let path = vec![
            PathSegment::Key("timelines".into()),
            PathSegment::Id(timeline_id),
            PathSegment::Key("recording".into()),
        ];
        let _ = self.engine.set(path, Lifecycle::Synced, serde_json::Value::Null).await;
    }

    // ── Reconciling the sockets ───────────────────────────────────────────────

    async fn reconcile(&mut self, inputs: Vec<InputConfig>) {
        self.configured = inputs.clone();
        let wanted: HashMap<Uuid, InputConfig> = inputs
            .into_iter()
            .filter(|input| input.listens_on(self.node_id))
            .map(|input| (input.id, input))
            .collect();

        let gone: Vec<Uuid> =
            self.listening.keys().copied().filter(|id| !wanted.contains_key(id)).collect();
        for id in gone {
            if let Some(input) = self.listening.remove(&id) {
                info!("[input] stopped {}", input.config.name);
                self.net.forget(&NetService::Input(id));
            }
        }

        for (id, config) in wanted {
            let wire = match self.wire_for(&config) {
                Some(wire) => wire,
                // Told a cable this machine has not got. `Network::bind` has already
                // said so on the row and in the log; a receiver that fell back to
                // every interface would join the guest console's groups on the house
                // LAN, which is the fault this mechanism exists to remove.
                None => {
                    self.listening.remove(&id);
                    continue;
                }
            };
            match self.listening.get_mut(&id) {
                Some(existing) if same_socket(&existing.config, &config) && existing.wire == wire => {
                    existing.status.name = config.name.clone();
                    existing.config = config;
                }
                _ => {
                    self.listening.remove(&id);
                    match open(id, &config, wire, self.frames_tx.clone()).await {
                        Ok(reader) => {
                            info!(
                                "[input] {} listening for {:?}{}",
                                config.name,
                                config.kind,
                                match wire {
                                    Some(addr) => format!(" on {addr}"),
                                    None => String::new(),
                                }
                            );
                            self.listening.insert(
                                id,
                                Listening {
                                    status: InputStatus {
                                        name: config.name.clone(),
                                        kind: format!("{:?}", config.kind).to_lowercase(),
                                        running: true,
                                        ..Default::default()
                                    },
                                    config,
                                    wire,
                                    reader,
                                    universes: BTreeMap::new(),
                                    packets_since_report: 0,
                                    reported_at: std::time::Instant::now(),
                                },
                            );
                        }
                        Err(e) => warn!("[input] {} could not start: {e}", config.name),
                    }
                }
            }
        }
        // A take whose input has just gone has to be written out rather than left
        // half open, and one whose input has just appeared can start.
        self.reconcile_recording().await;
        self.publish_status().await;
    }

    /// Which cable this input listens on. `None` is a refusal, recomputed every
    /// reconcile so that plugging the cable in is enough to start it.
    fn wire_for(&self, config: &InputConfig) -> Option<Option<Ipv4Addr>> {
        let wanted = self.net.for_input(config, self.node_id);
        self.net.bind(NetService::Input(config.id), &config.name, wanted.as_deref()).ok()
    }

    // ── Saying what is happening ──────────────────────────────────────────────

    fn measure_rates(&mut self) {
        let now = std::time::Instant::now();
        for input in self.listening.values_mut() {
            let elapsed = input.reported_at.elapsed();
            if elapsed.as_secs_f32() > 0.0 {
                input.status.packets_per_second =
                    input.packets_since_report as f32 / elapsed.as_secs_f32();
            }
            input.packets_since_report = 0;
            input.reported_at = now;
            // A source that has gone quiet leaves the merge on the timer as well as
            // on the next packet — otherwise an input whose only sender was unplugged
            // would hold that sender's last frame for ever, with nothing arriving to
            // trigger the sweep.
            for merged in input.universes.values_mut() {
                merged.remerge(now);
            }
            input.status.sources =
                input.universes.values().map(|m| m.sources.len()).sum::<usize>() as u16;
        }
    }

    async fn publish_status(&mut self) {
        let statuses: InputStatuses = self
            .listening
            .iter()
            .map(|(id, input)| (id.to_string(), input.status.clone()))
            .collect();
        if let Ok(json) = serde_json::to_value(&statuses) {
            let path = vec![PathSegment::Key("input_status".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }

    /// Draw what is arriving for whoever is watching.
    ///
    /// The same `output_traffic` broadcast an output's view goes out on, carrying the
    /// **input's** row id in `output_id`. Deliberate, and it is what makes a peer's
    /// input watchable with nothing added to the sync protocol: the ask lands in the
    /// same `Viewers` table, crosses the same `OutputWatch` message, and the panel
    /// tells the two apart by which collection the id is in.
    async fn draw_views(&mut self) {
        let Some(watching) = &mut self.watching else { return };
        watching.drawn_at = std::time::Instant::now();
        let at_ms = pult_schema::types::sequence::now_ms();
        let asks = watching.viewers.asks_of(self.node_id);
        let now = std::time::Instant::now();
        let mut alive: std::collections::HashSet<(Uuid, Option<String>)> = Default::default();

        for (input_id, ask) in asks {
            let Some(input) = self.listening.get(&input_id) else { continue };
            for focus in ask {
                let view = OutputView {
                    node_id: self.node_id,
                    output_id: input_id,
                    focus: focus.clone(),
                    at_ms,
                    sections: vec![OutputSection {
                        title: format!("{:?} arriving as {}", input.config.kind, input.config.name),
                        note: None,
                        body: SectionBody::Universes(traffic(input, focus.as_deref(), now)),
                    }],
                };
                let key = (input_id, focus);
                alive.insert(key.clone());
                let Ok(value) = serde_json::to_value(&view) else { continue };
                let mut without_stamp = value.clone();
                if let Some(object) = without_stamp.as_object_mut() {
                    object.remove("at_ms");
                }
                if watching.last.get(&key) == Some(&without_stamp) {
                    continue;
                }
                watching.last.insert(key, without_stamp);
                let path = vec![PathSegment::Key("output_traffic".into())];
                let _ = watching.updates.0.send((path, value));
            }
        }
        watching.last.retain(|key, _| alive.contains(key));
    }
}

/// What one input's universes look like to somebody watching.
fn traffic(input: &Listening, focus: Option<&str>, now: std::time::Instant) -> UniverseTraffic {
    let since = |then: std::time::Instant| now.saturating_duration_since(then).as_millis() as u32;
    let wanted: Option<u16> = focus.and_then(|f| f.parse().ok());
    let focused = wanted
        .and_then(|number| input.universes.get_key_value(&number))
        .or_else(|| input.universes.iter().next())
        .map(|(number, merged)| UniverseFrame {
            universe: *number,
            channels: merged.image.to_vec(),
        });

    UniverseTraffic {
        universes: input
            .universes
            .iter()
            .map(|(number, merged)| UniverseSummary {
                universe: *number,
                live_channels: merged.image.iter().filter(|byte| **byte != 0).count() as u16,
                changed_ms_ago: since(merged.changed_at),
                // "Sent" reads as "last heard from" here, which is the same question
                // one level down: has anything arrived, as against has anything moved.
                sent_ms_ago: since(merged.received_at),
            })
            .collect(),
        focused,
    }
}

/// Would these two configurations want the same socket and the same groups?
fn same_socket(a: &InputConfig, b: &InputConfig) -> bool {
    a.kind == b.kind && a.universes == b.universes
}

async fn wait_for_viewers(watching: Option<&mut Watching>) -> bool {
    match watching {
        Some(watching) => watching.wake.changed().await.is_ok(),
        None => std::future::pending().await,
    }
}

/// Open a listening socket and start reading it.
///
/// **`SO_REUSEADDR` on a fixed port**, which is the whole difference from the output
/// side: an output leaves by an ephemeral port and nothing else wants it, where an
/// input has to sit on 5568 or 6454 — the port the protocol names — alongside whatever
/// else on this machine is listening to the same show LAN. Two inputs on one station,
/// and a station beside another program, both need it.
///
/// Bound to `0.0.0.0` rather than to the named interface even when one is named: a
/// multicast group is joined *on* an interface as a separate act, and binding a
/// receiver to a unicast address is what stops broadcast Art-Net arriving at all.
async fn open(
    id: Uuid,
    config: &InputConfig,
    interface: Option<Ipv4Addr>,
    frames: mpsc::Sender<Received>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let port = match config.kind {
        InputKind::Sacn => super::sacn::SACN_PORT,
        InputKind::Artnet => super::artnet::ARTNET_PORT,
    };
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    // BSD and macOS want this as well before two sockets may share a port; on Linux
    // `SO_REUSEADDR` alone is enough for multicast and this is harmless.
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    let socket = tokio::net::UdpSocket::from_std(std::net::UdpSocket::from(socket))?;

    if config.kind == InputKind::Sacn {
        // One group per **wire** universe, on the named cable. This is the setting
        // that actually decides whether the packets arrive: a station on two networks
        // that joined on the default one hears nothing at all from the lighting LAN,
        // with no error anywhere.
        let on = interface.unwrap_or(Ipv4Addr::UNSPECIFIED);
        for wire in config.universes.keys() {
            if let Err(e) = socket.join_multicast_v4(super::sacn::multicast_group(*wire), on) {
                warn!("[input] {}: cannot join universe {wire} on {on}: {e}", config.name);
            }
        }
    }

    let kind = config.kind;
    Ok(tokio::spawn(async move {
        let mut buffer = vec![0u8; 2048];
        loop {
            let Ok((len, from)) = socket.recv_from(&mut buffer).await else { break };
            let Some(received) = parse(id, kind, from, &buffer[..len]) else { continue };
            // Dropped rather than awaited: a manager that is behind is behind on the
            // whole show, and holding the socket open with a full queue would turn
            // one slow pass into a growing backlog of stale universes.
            if frames.try_send(received).is_err() {
                continue;
            }
        }
    }))
}

/// One datagram, in this manager's terms.
fn parse(
    input_id: Uuid,
    kind: InputKind,
    from: SocketAddr,
    bytes: &[u8],
) -> Option<Received> {
    match kind {
        InputKind::Sacn => {
            let packet = super::sacn::parse_e131(bytes)?;
            Some(Received {
                input_id,
                source: SourceId::Cid(packet.cid),
                priority: packet.priority,
                wire_universe: packet.universe,
                channels: packet.channels,
            })
        }
        InputKind::Artnet => {
            let packet = super::artnet::parse_art_dmx(bytes)?;
            Some(Received {
                input_id,
                source: SourceId::Addr(from.ip()),
                // Art-Net has no priority field, so every source sits at the same
                // number and the merge below degenerates to plain HTP — which is what
                // Art-Net's own specification says a merging node should do.
                priority: 100,
                wire_universe: packet.universe,
                channels: packet.channels,
            })
        }
    }
}

#[cfg(test)]
mod tests;
