//! Sound out of an OpenHaunt node's loudspeaker.
//!
//! A clip whose destination is a node is not mixed here. The node has its own DAC and
//! its own clock, so the console's part is to get the sound onto it and say *when*:
//! the OpenHaunt protocol's audio extension (`node/docs/13-audio.md`) gives a node a
//! clip store addressed by sha256, voices started at a show time over MQTT, and one RTP
//! stream with a playout delay the console chooses.
//!
//! # Who speaks to the nodes
//!
//! **The station driving them**, which is the leader: it holds the broker and it
//! publishes `openhaunt/clock`, which is the clock every `at` here is read against. A
//! node has one broker and one clock, so no other station can schedule against it.
//! `Timeline::node_id` keeps meaning which station plays a timeline's *station* clips;
//! a scare whose scream is in Dante and whose bang is on a prop is played by two
//! machines, each doing the half it can.
//!
//! # A clip is the region the timeline plays, made once
//!
//! What goes to the node is the clip's trimmed region, faded and resampled
//! ([`pult_audio::mix::bounce`]) and written as a WAV ([`pult_audio::wav`]), so a node
//! looping it loops exactly what a deck would have looped. Ten seconds or less goes as
//! PCM at the node's own rate, because ADPCM's noise floor is audible under a quiet
//! sting; anything longer as IMA ADPCM at 32 kHz, a quarter of the size. Both are
//! deterministic, so the sha is the same on every station and after every restart and
//! nothing is uploaded twice.
//!
//! **Gain travels on `play`**, not in the file, so a fader dragged during a rehearsal
//! does not rewrite flash. A node cannot amplify, so gain above 0 dB plays at 0 dB.
//!
//! # When, and how far ahead
//!
//! Every `at` is worked out from the timeline's anchor: the show time this time round of
//! the clip starts. A locate into the middle of a clip needs no offset, because a node
//! that is told a start already in the past **joins where the clip would be by now**.
//! Plays go out at most [`LOOKAHEAD_MS`] early: the protocol does not say how far ahead
//! a node will hold a command, and a clip a minute into a timeline should not be
//! waiting in a node's memory for a minute.
//!
//! # Uploads are slow, and the show is not
//!
//! An upload is an HTTP PUT of up to megabytes into flash, which stops the node's
//! caches for every sector erase. So they run off the manager's loop, one per node at a
//! time (the node answers 409 to a second), and only for a clip that has stood still
//! for [`SETTLE_MS`]: a trim being dragged is a new file every frame, and none of them
//! should reach a node until the hand lets go.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pult_audio::wav::Encoding;
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        devices::{DrivenNode, NodeAudio, NodeAudioState, NodeAudioStates, NodeReport},
        timeline::{AtEnd, AudioClip, NodeAudioMode, Timeline},
        zone::{node_targets, SpeakerZone},
    },
};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{AudioManager, DRIFT_MS};
use crate::infra::devices::{DeviceDirectory, DeviceEntry, DeviceHandle};

/// How far ahead of a clip's start its `play` is sent.
pub const LOOKAHEAD_MS: u64 = 2_000;

/// How long a clip has to have stood still before it is uploaded.
pub const SETTLE_MS: u64 = 2_000;

/// A clip this long or shorter goes as PCM. Longer is ADPCM.
pub const PCM_UP_TO_MS: f64 = 10_000.0;

/// The rate an ADPCM clip is written at.
pub const ADPCM_RATE: u32 = 32_000;

/// The UDP port a node's stream is sent to. The protocol leaves the port to the
/// console; one stream per node, so one port does.
pub const STREAM_PORT: u16 = 5004;

/// The playout delay a stream runs at when the station's preferences say nothing.
pub const DEFAULT_STREAM_DELAY_MS: u32 = 150;

/// How long a node takes to open its port after `stream start`, which a stream's first
/// sample is heard that much after on top of its delay.
pub const STREAM_OPENS_MS: u64 = 100;

/// Everything the station driving the nodes is keeping about them.
pub struct Nodes {
    pub(super) directory: Option<watch::Receiver<DeviceDirectory>>,
    devices: Option<DeviceHandle>,
    http: reqwest::Client,
    delay_ms: u32,
    /// Whether the timelines have arrived at all. Until they have, nothing is wanted
    /// and every clip on every node would look like one to delete.
    seen_timelines: bool,
    /// Every region made for a node, by what it was made from.
    bounced: HashMap<String, Arc<Bounced>>,
    /// Which region each clip is now, on each node it is going to.
    made: HashMap<(Uuid, String), Arc<Bounced>>,
    /// When each (node, clip) was first wanted in its current form.
    wanted_since: HashMap<(String, String), Instant>,
    stores: BTreeMap<String, Store>,
    /// Nodes with a request in flight, and which clip it is uploading if it is one.
    busy: BTreeMap<String, Option<String>>,
    /// Clips a node refused, and why. Not tried again until its store changes.
    refused: BTreeMap<(String, String), String>,
    pub(super) results: mpsc::Receiver<Done>,
    results_tx: mpsc::Sender<Done>,
    /// What each voice was last told, keyed by node and voice.
    sent: HashMap<(String, String), Sent>,
    /// Whether each node could hear commands at the last look: on the network and on
    /// the broker. A node that comes back has forgotten every voice.
    reachable: BTreeMap<String, bool>,
    streams: HashMap<String, Streamer>,
    /// Why a node clip is not playing, keyed by timeline, then by clip, then by the
    /// node: a zone may hold several, and one of them being down is not all of them.
    pub(super) faults: HashMap<Uuid, BTreeMap<String, BTreeMap<String, String>>>,
    published: Option<NodeAudioStates>,
    /// Whether this station drove the nodes at the last look, which decides whose
    /// faults its status rows carry: its own, or the driving station's report.
    pub(super) driving: bool,
    /// What this station last said in its `node_reports` row.
    reported: Option<NodeReport>,
}

/// One region, made for one kind of node.
pub struct Bounced {
    pub sha: String,
    pub bytes: Arc<Vec<u8>>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Store {
    pub stored: BTreeSet<String>,
    pub bytes_total: u64,
    pub bytes_free: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct Sent {
    clip: String,
    at: u64,
    gain: f32,
}

/// A request to a node, finished.
pub struct Done {
    serial: String,
    outcome: Outcome,
}

enum Outcome {
    Listed(Store),
    Uploaded { sha: String },
    Deleted { sha: String },
    /// The node said no in a way that will not change by asking again.
    Refused { sha: String, why: String },
    /// Try again later: busy, unreachable, or answering something unexpected.
    Failed(String),
}

impl Nodes {
    pub fn new(delay_ms: Option<u32>) -> Nodes {
        let (results_tx, results) = mpsc::channel(16);
        let http = reqwest::Client::builder()
            // Long enough for a few megabytes over wifi into flash, short enough that a
            // node that has gone away frees its slot for the next attempt.
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Nodes {
            directory: None,
            devices: None,
            http,
            delay_ms: delay_ms.unwrap_or(DEFAULT_STREAM_DELAY_MS),
            seen_timelines: false,
            bounced: HashMap::new(),
            made: HashMap::new(),
            wanted_since: HashMap::new(),
            stores: BTreeMap::new(),
            busy: BTreeMap::new(),
            refused: BTreeMap::new(),
            results,
            results_tx,
            sent: HashMap::new(),
            reachable: BTreeMap::new(),
            streams: HashMap::new(),
            faults: HashMap::new(),
            published: None,
            driving: false,
            reported: None,
        }
    }

    /// Where the nodes are, and the way to talk to them. A station without these —
    /// every test that is not about nodes — simply sends nothing to any.
    pub fn attach(&mut self, directory: watch::Receiver<DeviceDirectory>, devices: DeviceHandle) {
        self.directory = Some(directory);
        self.devices = Some(devices);
    }

    pub fn saw_timelines(&mut self) {
        self.seen_timelines = true;
    }

    fn directory(&self) -> DeviceDirectory {
        self.directory.as_ref().map(|d| d.borrow().clone()).unwrap_or_default()
    }
}

/// The next time the directory moves; never, for a manager without one.
pub async fn moved(directory: &mut Option<watch::Receiver<DeviceDirectory>>) {
    let Some(directory) = directory else { return std::future::pending().await };
    if directory.changed().await.is_err() {
        // Its sender is gone, and would answer this at once for ever.
        std::future::pending::<()>().await;
    }
}

/// Every clip on this timeline that a node plays, once per node: a clip sent to one
/// node, and one sent to a zone with node loudspeakers in it.
pub fn node_clips<'t>(
    timeline: &'t Timeline,
    zones: &[SpeakerZone],
) -> Vec<(&'t AudioClip, String, NodeAudioMode)> {
    timeline
        .clips
        .iter()
        .flat_map(|clip| {
            node_targets(&clip.destination, zones).into_iter().map(move |(serial, mode)| (clip, serial, mode))
        })
        .collect()
}

/// A voice's name on the node: stable for one clip on one timeline, and inside the
/// protocol's fifteen characters.
pub fn voice_name(timeline: Uuid, clip: Uuid) -> String {
    let t = timeline.simple().to_string();
    let c = clip.simple().to_string();
    // The tails: random in a v4, and different in a test's `from_u128` ids as well.
    format!("{}.{}", &t[25..], &c[25..])
}

/// PCM at the node's rate for a short clip, ADPCM at 32 kHz for a long one — or PCM
/// whatever the length, where the node does not say it can play ADPCM.
pub fn encoding_for(region_ms: f64, node: &NodeAudio) -> (Encoding, u32) {
    let adpcm = node.clips.formats.is_empty() || node.clips.formats.iter().any(|f| f == "ima-adpcm");
    if region_ms <= PCM_UP_TO_MS || !adpcm {
        (Encoding::Pcm16, node.rate.max(8_000))
    } else {
        (Encoding::ImaAdpcm, ADPCM_RATE)
    }
}

/// When this time round of a clip is heard on a running timeline: the show time it
/// starts, and the show time it is over (`None` for as long as the timeline runs).
/// `None` where the timeline is not running or the clip has nothing left to play.
///
/// `region_ms` is how long one time round lasts and `repeats` how many there are; on a
/// timeline that goes round, a clip is cut at the end of the lap and started again at
/// the top of the next one.
pub fn occurrence(
    timeline: &Timeline,
    start_ms: u64,
    region_ms: u64,
    repeats: u32,
    now: u64,
) -> Option<(u64, Option<u64>)> {
    if !timeline.running || timeline.rate <= 0.0 {
        return None;
    }
    let rate = timeline.rate as f64;
    // Measured from the anchor where that is still to come (a count-in), because the
    // playhead holds until then rather than moving from now.
    let base = now.max(timeline.anchor_ms);
    let position = timeline.position_at(base) as f64;
    let show = |at: f64| -> u64 { (base as f64 + (at - position) / rate).round().max(0.0) as u64 };
    let start = start_ms as f64;
    let length = region_ms as f64 * repeats.max(1) as f64;

    match timeline.end.filter(|end| end.at_ms > 0) {
        None => {
            let end = start + length;
            if position >= end {
                return None;
            }
            Some((show(start), end.is_finite().then(|| show(end))))
        }
        Some(end) => {
            let stop = end.at_ms as f64;
            if start >= stop {
                return None;
            }
            let over = (start + length).min(stop);
            match end.then {
                AtEnd::Stop if position >= over => None,
                AtEnd::Stop => Some((show(start), Some(show(over)))),
                AtEnd::Loop => {
                    let mut lap = (position / stop).floor() * stop;
                    if position - lap >= over {
                        lap += stop;
                    }
                    Some((show(lap + start), Some(show(lap + over))))
                }
            }
        }
    }
}

/// The next request for each node that is free to take one.
///
/// A node is asked what it holds before anything else, and then a clip nothing wants is
/// deleted before a wanted one is uploaded, so the store is never full of yesterday's
/// scream when today's does not fit. One request per node, because the node refuses a
/// second upload while one is running.
pub fn plan(
    wanted: &BTreeMap<String, BTreeSet<String>>,
    stores: &BTreeMap<String, Store>,
    busy: &BTreeSet<String>,
    ready: impl Fn(&str, &str) -> bool,
    may_delete: bool,
) -> Vec<Op> {
    let mut ops = Vec::new();
    for (serial, clips) in wanted {
        if busy.contains(serial) {
            continue;
        }
        let Some(store) = stores.get(serial) else {
            ops.push(Op::List(serial.clone()));
            continue;
        };
        if may_delete {
            if let Some(sha) = store.stored.iter().find(|sha| !clips.contains(*sha)) {
                ops.push(Op::Delete { serial: serial.clone(), sha: sha.clone() });
                continue;
            }
        }
        if let Some(sha) = clips.iter().find(|sha| !store.stored.contains(*sha) && ready(serial, sha)) {
            ops.push(Op::Upload { serial: serial.clone(), sha: sha.clone() });
        }
    }
    ops
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    List(String),
    Upload { serial: String, sha: String },
    Delete { serial: String, sha: String },
}

impl AudioManager {
    /// Make the nodes hold what the show wants them to, and start or stop their streams.
    /// Runs whenever the timelines change and once a second.
    pub(super) async fn reconcile_nodes(&mut self) {
        let leading = self.net.standing().is_leader;
        if !leading || self.nodes.devices.is_none() {
            self.let_go_of_nodes().await;
            return;
        }
        let directory = self.nodes.directory();
        self.notice_nodes_coming_back(&directory);

        let timelines = self.timelines.clone();
        let zones = self.zones.clone();
        let mut wanted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut faults: HashMap<Uuid, BTreeMap<String, BTreeMap<String, String>>> = HashMap::new();
        let mut streamed: BTreeMap<String, Vec<(Timeline, AudioClip)>> = BTreeMap::new();

        // Every adopted node with a loudspeaker is in `wanted`, if only with nothing, so a
        // node whose last clip was taken off its timeline is emptied rather than ignored.
        for (serial, entry) in &directory.entries {
            if entry.online && entry.audio.is_some() {
                wanted.entry(serial.clone()).or_default();
            }
        }

        for timeline in &timelines {
            for (clip, serial, mode) in node_clips(timeline, &zones) {
                let serial = serial.as_str();
                let fault = |faults: &mut HashMap<Uuid, BTreeMap<String, BTreeMap<String, String>>>, why: String| {
                    faults
                        .entry(timeline.id)
                        .or_default()
                        .entry(clip.id.to_string())
                        .or_default()
                        .insert(serial.to_string(), why);
                };
                let Some(entry) = directory.entries.get(serial).filter(|e| e.online) else {
                    fault(&mut faults, format!("node {serial} is not on the network"));
                    continue;
                };
                let Some(audio) = entry.audio.clone() else {
                    fault(&mut faults, format!("node {serial} has no loudspeaker"));
                    continue;
                };
                if mode == NodeAudioMode::Stream {
                    // Decoded now, as a station's own clips are the moment they are on a
                    // timeline, and not when Play is pressed: decoding a song took
                    // longer than a count-in on the first Play after a restart, and the
                    // stream started that late, joining the clip part of the way in.
                    if let Err(e) = self.decoded(&clip.asset).await {
                        fault(&mut faults, e);
                    }
                    streamed.entry(serial.to_string()).or_default().push((timeline.clone(), clip.clone()));
                    continue;
                }
                match self.bounce_for(clip, &audio).await {
                    Ok(bounced) => {
                        self.nodes.made.insert((clip.id, serial.to_string()), bounced.clone());
                        let key = (serial.to_string(), bounced.sha.clone());
                        self.nodes.wanted_since.entry(key.clone()).or_insert_with(Instant::now);
                        wanted.entry(serial.to_string()).or_default().insert(bounced.sha.clone());
                        if let Some(why) = self.nodes.refused.get(&key) {
                            fault(&mut faults, why.clone());
                        } else if !self.nodes.stores.get(serial).is_some_and(|s| s.stored.contains(&bounced.sha)) {
                            fault(&mut faults, format!("not on node {serial} yet"));
                        }
                    }
                    Err(e) => fault(&mut faults, e),
                }
            }
        }
        // Forget when a clip was first wanted once nothing wants it, so dragging it
        // back to where it was starts the wait again rather than uploading at once.
        self.nodes
            .wanted_since
            .retain(|(serial, sha), _| wanted.get(serial).is_some_and(|w| w.contains(sha)));
        self.nodes.faults = faults;

        self.start_requests(&wanted, &directory);
        self.reconcile_streams(streamed, &directory).await;
        self.schedule_nodes(pult_schema::clock::now_ms());
        self.publish_nodes(&wanted).await;
        self.nodes.driving = true;
        self.publish_report(&directory).await;
    }

    /// No longer the station driving the nodes: stop every stream and forget what was
    /// sent. Whoever leads now has the broker, and tells the nodes for itself.
    async fn let_go_of_nodes(&mut self) {
        for (_, streamer) in std::mem::take(&mut self.nodes.streams) {
            streamer.stop().await;
        }
        self.nodes.sent.clear();
        self.nodes.faults.clear();
        self.nodes.stores.clear();
        if self.nodes.published.as_ref().is_some_and(|p| !p.is_empty()) {
            self.nodes.published = None;
            self.write_node_audio(&NodeAudioStates::new()).await;
        }
        self.nodes.driving = false;
        self.publish_report(&DeviceDirectory::default()).await;
    }

    /// Say what this station knows about the nodes' sound in its `node_reports` row:
    /// everything, where it drives them, and that it does not, where it does not. See
    /// [`NodeReport`] for why the other stations read this rather than their own view.
    ///
    /// Only when it moved *and* the row is still there: joining a session replaces
    /// this station's state with the leader's, which `station_audio` found the hard way.
    async fn publish_report(&mut self, directory: &DeviceDirectory) {
        let driving = self.nodes.driving;
        let nodes = directory
            .entries
            .iter()
            .filter(|(_, e)| driving && e.adopted && e.audio.is_some())
            .map(|(serial, e)| {
                (
                    serial.clone(),
                    DrivenNode {
                        name: e.name.clone(),
                        online: e.online,
                        on_broker: e.on_broker,
                        audio: e.audio.clone(),
                        store: self.nodes.published.as_ref().and_then(|p| p.get(serial)).cloned(),
                        summary: e.audio_summary.clone(),
                    },
                )
            })
            .collect();
        let clip_faults = self
            .nodes
            .faults
            .iter()
            .map(|(timeline, clips)| (timeline.to_string(), clips.clone()))
            .collect();
        let row = NodeReport { id: self.node_id.0, driving, nodes, clip_faults };
        let Ok(value) = serde_json::to_value(&row) else { return };
        let path = vec![PathSegment::Key("node_reports".into()), PathSegment::Id(row.id)];
        if self.nodes.reported.as_ref() == Some(&row)
            && self.engine.get(path.clone()).await.is_ok_and(|held| held == value)
        {
            return;
        }
        if self.engine.set(path, Lifecycle::Synced, value.clone()).await.is_err() {
            let create = vec![PathSegment::Key("node_reports".into()), PathSegment::Key("__create".into())];
            if let Err(e) = self.engine.set(create, Lifecycle::Synced, value).await {
                debug!("[audio] could not publish what this station knows about the nodes: {e}");
                return;
            }
        }
        self.nodes.reported = Some(row);
    }

    /// Why each node clip is not playing, from whoever can say: this station's own
    /// view where it drives the nodes, and the driving station's report otherwise. A
    /// report from a station that has gone (no `stations` row any more) is not believed.
    pub(super) async fn node_faults(&self) -> HashMap<Uuid, BTreeMap<String, BTreeMap<String, String>>> {
        if self.nodes.driving {
            return self.nodes.faults.clone();
        }
        let read = |table: &'static str| async move {
            match self.engine.get(vec![PathSegment::Key(table.into())]).await {
                Ok(serde_json::Value::Array(rows)) => rows,
                _ => Vec::new(),
            }
        };
        let live: BTreeSet<String> =
            read("stations").await.iter().filter_map(|s| s["id"].as_str().map(String::from)).collect();
        let report = read("node_reports")
            .await
            .into_iter()
            .filter_map(|row| serde_json::from_value::<NodeReport>(row).ok())
            .filter(|r| r.driving && r.id != self.node_id.0 && live.contains(&r.id.to_string()))
            .max_by_key(|r| r.id);
        let Some(report) = report else { return HashMap::new() };
        report
            .clip_faults
            .into_iter()
            .filter_map(|(timeline, clips)| Some((timeline.parse().ok()?, clips)))
            .collect()
    }

    /// A node that was away has forgotten every voice it was playing, and may have been
    /// re-flashed: what it holds is asked again and what it was playing is sent again.
    fn notice_nodes_coming_back(&mut self, directory: &DeviceDirectory) {
        for (serial, entry) in &directory.entries {
            let reachable = entry.online && entry.on_broker;
            let was = self.nodes.reachable.insert(serial.clone(), reachable);
            if reachable && was != Some(true) {
                self.nodes.sent.retain(|(node, _), _| node != serial);
                self.nodes.stores.remove(serial);
            }
        }
    }

    /// A clip's region, made for this kind of node — or the one made already.
    async fn bounce_for(&mut self, clip: &AudioClip, node: &NodeAudio) -> Result<Arc<Bounced>, String> {
        let decoded = self.decoded(&clip.asset).await?;
        let source = pult_audio::Source {
            samples: decoded.samples.clone(),
            channels: decoded.channels as usize,
            sample_rate: decoded.sample_rate,
        };
        let placement = super::placement(clip);
        let region = placement.region_ms(source.duration_ms());
        if region <= 0.0 {
            return Err("the clip is trimmed to nothing".into());
        }
        let (encoding, rate) = encoding_for(region, node);
        let channels = (decoded.channels.max(1)).min(node.channels());
        let key = serde_json::json!([
            clip.asset, clip.offset_ms, clip.length_ms, clip.fade_in_ms, clip.fade_out_ms,
            clip.times() > 1, rate, channels, encoding == Encoding::ImaAdpcm,
        ])
        .to_string();
        if let Some(bounced) = self.nodes.bounced.get(&key) {
            return Ok(bounced.clone());
        }
        // Rendered and encoded off the runtime: a three-minute bed is a few hundred
        // milliseconds of arithmetic.
        let made = tokio::task::spawn_blocking(move || {
            let unity = pult_audio::Placement { gain: 1.0, ..placement };
            let samples = pult_audio::mix::bounce(&source, unity, channels as usize, rate);
            let bytes = pult_audio::wav::encode(&samples, channels, rate, encoding)
                .map_err(|e| e.to_string())?;
            let frames = samples.len() / channels.max(1) as usize;
            Ok::<_, String>(Bounced {
                sha: crate::infra::assets::digest(&bytes),
                duration_ms: (frames as u64 * 1_000) / rate as u64,
                bytes: Arc::new(bytes),
            })
        })
        .await
        .map_err(|e| format!("the clip could not be made for a node: {e}"))??;
        let made = Arc::new(made);
        self.nodes.bounced.insert(key, made.clone());
        Ok(made)
    }

    /// Hand each idle node its next request.
    fn start_requests(&mut self, wanted: &BTreeMap<String, BTreeSet<String>>, directory: &DeviceDirectory) {
        let busy: BTreeSet<String> = self.nodes.busy.keys().cloned().collect();
        let settled = Duration::from_millis(SETTLE_MS);
        let ready = |serial: &str, sha: &str| {
            !self.nodes.refused.contains_key(&(serial.to_string(), sha.to_string()))
                && self
                    .nodes
                    .wanted_since
                    .get(&(serial.to_string(), sha.to_string()))
                    .is_some_and(|since| since.elapsed() >= settled)
        };
        let ops = plan(wanted, &self.nodes.stores, &busy, ready, self.nodes.seen_timelines);
        for op in ops {
            let serial = match &op {
                Op::List(s) | Op::Upload { serial: s, .. } | Op::Delete { serial: s, .. } => s.clone(),
            };
            let Some(entry) = directory.entries.get(&serial) else { continue };
            let base = format!("http://{}:{}/api/v1/audio/clips", entry.ip, entry.port);
            let http = self.nodes.http.clone();
            let done = self.nodes.results_tx.clone();
            let uploading = match &op {
                Op::Upload { sha, .. } => Some(sha.clone()),
                _ => None,
            };
            let bytes = match &op {
                Op::Upload { sha, .. } => match self.nodes.bounced.values().find(|b| &b.sha == sha) {
                    Some(b) => Some(b.bytes.clone()),
                    None => continue,
                },
                _ => None,
            };
            if let Some(sha) = &uploading {
                info!("[audio] uploading a clip to node {serial} ({sha:.12})");
            }
            self.nodes.busy.insert(serial.clone(), uploading);
            tokio::spawn(async move {
                let outcome = request(&http, &base, op, bytes).await;
                let _ = done.send(Done { serial, outcome }).await;
            });
        }
    }

    /// A request to a node has finished.
    pub(super) async fn node_request_done(&mut self, done: Done) {
        let Done { serial, outcome } = done;
        self.nodes.busy.remove(&serial);
        match outcome {
            Outcome::Listed(store) => {
                self.forget_refusals_if_roomier(&serial, &store);
                self.nodes.stores.insert(serial, store);
            }
            Outcome::Uploaded { sha } => {
                info!("[audio] node {serial} holds clip {sha:.12}");
                // Asked again rather than worked out, so `bytes_free` is the node's own.
                self.nodes.stores.remove(&serial);
            }
            Outcome::Deleted { sha } => {
                debug!("[audio] node {serial} let go of clip {sha:.12}");
                self.nodes.refused.retain(|(node, _), _| node != &serial);
                self.nodes.stores.remove(&serial);
            }
            Outcome::Refused { sha, why } => {
                warn!("[audio] node {serial} refused clip {sha:.12}: {why}");
                self.nodes.refused.insert((serial, sha), why);
            }
            Outcome::Failed(why) => debug!("[audio] node {serial}: {why}"),
        }
        self.reconcile_nodes().await;
    }

    fn forget_refusals_if_roomier(&mut self, serial: &str, store: &Store) {
        let roomier = self.nodes.stores.get(serial).is_none_or(|old| store.bytes_free > old.bytes_free);
        if roomier {
            self.nodes.refused.retain(|(node, _), _| node != serial);
        }
    }

    // ── Playing ───────────────────────────────────────────────────────────────

    /// Tell each node's voices what the timelines say they should be doing. The 25 ms
    /// pass, and whatever the timelines changed.
    pub(super) fn schedule_nodes(&mut self, now: u64) {
        let Some(devices) = self.nodes.devices.clone() else { return };
        if !self.net.standing().is_leader {
            return;
        }
        let mut wanted: HashMap<(String, String), Sent> = HashMap::new();
        for timeline in &self.timelines {
            for (clip, serial, mode) in node_clips(timeline, &self.zones) {
                let serial = serial.as_str();
                if mode != NodeAudioMode::Clip || !self.nodes.reachable.get(serial).copied().unwrap_or(false) {
                    continue;
                }
                let Some(bounced) = self.held_bounce(clip, serial) else { continue };
                let Some((at, over)) =
                    occurrence(timeline, clip.start_ms, bounced.duration_ms, clip.times(), now)
                else {
                    continue;
                };
                if over.is_some_and(|over| now >= over) || at > now + LOOKAHEAD_MS {
                    continue;
                }
                wanted.insert(
                    (serial.to_string(), voice_name(timeline.id, clip.id)),
                    Sent { clip: bounced.sha.clone(), at, gain: clip.gain().min(1.0) },
                );
            }
        }

        // Voices that should no longer be sounding.
        let stale: Vec<(String, String)> =
            self.nodes.sent.keys().filter(|key| !wanted.contains_key(*key)).cloned().collect();
        for key in stale {
            self.nodes.sent.remove(&key);
            devices.audio(key.0, "stop", serde_json::json!({ "voice": key.1, "fade_ms": 10 }));
        }

        for (key, want) in wanted {
            let held = self.nodes.sent.get(&key);
            let same_start = held.is_some_and(|h| h.clip == want.clip && (h.at as i64 - want.at as i64).abs() <= DRIFT_MS);
            if same_start {
                if held.is_some_and(|h| h.gain != want.gain) {
                    devices.audio(
                        key.0.clone(),
                        "gain",
                        serde_json::json!({ "voice": key.1, "gain": want.gain, "fade_ms": 50 }),
                    );
                    self.nodes.sent.insert(key, want);
                }
                continue;
            }
            // A voice that is sounding now and has been moved to start later is stopped
            // first: the protocol does not say whether a replacing `play` takes over
            // when it arrives or at its `at`, and a locate back must not leave the old
            // sound running until then.
            if held.is_some_and(|h| h.at <= now) && want.at > now {
                devices.audio(key.0.clone(), "stop", serde_json::json!({ "voice": key.1, "fade_ms": 10 }));
            }
            // A clip that plays more than once is one time round on the node, looped,
            // and stopped at the end of its last time round: the stop above, once it
            // is no longer wanted.
            let looped = self
                .timelines
                .iter()
                .flat_map(|t| node_clips(t, &self.zones).into_iter().map(move |(c, _, _)| (t.id, c)))
                .find(|(t, c)| voice_name(*t, c.id) == key.1)
                .is_some_and(|(_, c)| c.times() > 1);
            devices.audio(
                key.0.clone(),
                "play",
                serde_json::json!({
                    "voice": key.1,
                    "clip": want.clip,
                    "at": want.at,
                    "loop": looped,
                    "gain": want.gain,
                }),
            );
            self.nodes.sent.insert(key, want);
        }
    }

    /// The region a node holds for this clip, if it holds one.
    fn held_bounce(&self, clip: &AudioClip, serial: &str) -> Option<Arc<Bounced>> {
        let made = self.nodes.made.get(&(clip.id, serial.to_string()))?;
        let store = self.nodes.stores.get(serial)?;
        store.stored.contains(&made.sha).then(|| made.clone())
    }

    // ── Streaming ─────────────────────────────────────────────────────────────

    async fn reconcile_streams(
        &mut self,
        streamed: BTreeMap<String, Vec<(Timeline, AudioClip)>>,
        directory: &DeviceDirectory,
    ) {
        let mut keep = BTreeSet::new();
        for (serial, clips) in streamed {
            let Some(entry) = directory.entries.get(&serial) else { continue };
            let Some(audio) = entry.audio.clone() else { continue };
            if !entry.on_broker || !clips.iter().any(|(t, _)| t.running) {
                continue;
            }
            let port = audio.stream.as_ref().and_then(|s| s.port).unwrap_or(STREAM_PORT);
            let targets = stream_targets(entry, port);
            if targets.is_empty() {
                continue;
            }
            let channels = audio.channels() as usize;
            let rate = audio.rate.max(8_000);

            let mut decks = Vec::new();
            let mut built = Vec::new();
            let mut by_timeline: BTreeMap<Uuid, (Timeline, Vec<AudioClip>)> = BTreeMap::new();
            for (timeline, clip) in clips {
                by_timeline.entry(timeline.id).or_insert_with(|| (timeline.clone(), Vec::new())).1.push(clip);
            }
            for (timeline, clips) in by_timeline.into_values() {
                let mut voices = Vec::new();
                for clip in &clips {
                    let Ok(decoded) = self.decoded(&clip.asset).await else { continue };
                    let outputs: Vec<u16> = (1..=channels as u16).collect();
                    let Ok(routes) = pult_audio::mix::route(decoded.channels as usize, &outputs, channels) else {
                        continue;
                    };
                    voices.push(pult_audio::Voice {
                        source: pult_audio::Source {
                            samples: decoded.samples.clone(),
                            channels: decoded.channels as usize,
                            sample_rate: decoded.sample_rate,
                        },
                        placement: super::placement(clip),
                        routes,
                    });
                }
                built.push(serde_json::json!([
                    timeline.id, timeline.running, timeline.anchor_ms, timeline.position_at_anchor_ms,
                    timeline.rate, timeline.end, clips,
                ]));
                decks.push(StreamDeck {
                    id: timeline.id.as_u128(),
                    running: timeline.running,
                    transport: timeline.transport(),
                    end: super::deck_end(&timeline),
                    voices,
                });
            }
            let built = serde_json::Value::Array(built).to_string();
            keep.insert(serial.clone());
            match self.nodes.streams.get_mut(&serial) {
                // The same session, wherever the node is now. A node that moved from
                // wifi to its cable listens on the same port on every interface, so
                // the packets are simply sent somewhere else: same ssrc, same mapping,
                // and nothing for the node to notice. A new session would be a stop
                // and a start, and a gap the length of the node's buffer.
                Some(streamer) if streamer.same_session(port, rate, channels) => {
                    streamer.follow(&serial, targets);
                    if streamer.built != built {
                        streamer.built = built;
                        streamer.send(StreamMessage::Decks(decks));
                    }
                }
                _ => {
                    // Gone before its successor starts, stop and all: a `stop` names
                    // no session, so one arriving after the new `start` stops the new
                    // stream.
                    if let Some(old) = self.nodes.streams.remove(&serial) {
                        old.stop().await;
                    }
                    let Some(devices) = self.nodes.devices.clone() else { continue };
                    info!(
                        "[audio] streaming to node {serial} at {}, {} ms ahead",
                        list(&targets),
                        self.nodes.delay_ms
                    );
                    let heard_after = self.nodes.delay_ms as u64 + STREAM_OPENS_MS;
                    if heard_after >= pult_schema::types::timeline::STREAM_LEAD_MS {
                        warn!(
                            "[audio] a stream delay of {} ms is heard {heard_after} ms after it starts, past the {} ms a Play \
                             is anchored ahead: the head of a clip will be lost on node {serial}",
                            self.nodes.delay_ms,
                            pult_schema::types::timeline::STREAM_LEAD_MS
                        );
                    }
                    let streamer = Streamer::start(
                        serial.clone(), targets, port, rate, channels, self.nodes.delay_ms, devices, decks, built,
                    );
                    self.nodes.streams.insert(serial.clone(), streamer);
                }
            }
        }
        let gone: Vec<String> = self.nodes.streams.keys().filter(|s| !keep.contains(*s)).cloned().collect();
        for serial in gone {
            if let Some(streamer) = self.nodes.streams.remove(&serial) {
                info!("[audio] the stream to node {serial} is over");
                streamer.stop().await;
            }
        }
    }

    /// Send every running stream wherever its node is now.
    ///
    /// Run whenever the directory moves rather than once a second with the rest: a
    /// node says on `net` within a second of its cable going, and every packet sent to
    /// the old address after that is sound nobody hears.
    pub(super) fn follow_nodes(&mut self) {
        let directory = self.nodes.directory();
        for (serial, streamer) in &mut self.nodes.streams {
            if let Some(entry) = directory.entries.get(serial) {
                let targets = stream_targets(entry, streamer.port);
                if !targets.is_empty() {
                    streamer.follow(serial, targets);
                }
            }
        }
    }

    // ── Saying so ─────────────────────────────────────────────────────────────

    async fn publish_nodes(&mut self, wanted: &BTreeMap<String, BTreeSet<String>>) {
        let mut states = NodeAudioStates::new();
        for (serial, clips) in wanted {
            let store = self.nodes.stores.get(serial);
            let pending = clips.iter().filter(|sha| !store.is_some_and(|s| s.stored.contains(*sha))).count();
            let fault = self
                .nodes
                .refused
                .iter()
                .find(|((node, _), _)| node == serial)
                .map(|(_, why)| why.clone());
            states.insert(
                serial.clone(),
                NodeAudioState {
                    stored: store.map(|s| s.stored.iter().cloned().collect()).unwrap_or_default(),
                    bytes_total: store.map_or(0, |s| s.bytes_total),
                    bytes_free: store.map_or(0, |s| s.bytes_free),
                    uploading: self.nodes.busy.get(serial).cloned().flatten(),
                    pending: pending as u32,
                    streaming: self.nodes.streams.contains_key(serial),
                    fault,
                },
            );
        }
        if self.nodes.published.as_ref() != Some(&states) {
            self.write_node_audio(&states).await;
            self.nodes.published = Some(states);
        }
    }

    async fn write_node_audio(&self, states: &NodeAudioStates) {
        if let Ok(json) = serde_json::to_value(states) {
            let path = vec![PathSegment::Key("node_audio".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }
}

/// One request, made and answered.
async fn request(http: &reqwest::Client, base: &str, op: Op, bytes: Option<Arc<Vec<u8>>>) -> Outcome {
    match op {
        Op::List(_) => {
            let response = match http.get(base).send().await {
                Ok(r) => r,
                Err(e) => return Outcome::Failed(format!("could not ask what it holds: {e}")),
            };
            let Ok(body) = response.json::<serde_json::Value>().await else {
                return Outcome::Failed("answered its clip list with something else".into());
            };
            Outcome::Listed(Store {
                stored: body["clips"]
                    .as_array()
                    .map(|list| list.iter().filter_map(|c| c["sha"].as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
                bytes_total: body["bytes_total"].as_u64().unwrap_or(0),
                bytes_free: body["bytes_free"].as_u64().unwrap_or(0),
            })
        }
        Op::Upload { sha, .. } => {
            let body = bytes.map(|b| b.as_ref().clone()).unwrap_or_default();
            let length = body.len();
            let response = http
                .put(format!("{base}/{sha}"))
                .header("content-type", "audio/wav")
                .header("content-length", length)
                .body(body)
                .send()
                .await;
            match response.map(|r| r.status().as_u16()) {
                Ok(200 | 201) => Outcome::Uploaded { sha },
                Ok(413) => Outcome::Refused {
                    sha,
                    why: format!("does not fit on the node ({} kB)", length / 1_024),
                },
                Ok(400) => Outcome::Refused { sha, why: "the node could not play the file it was sent".into() },
                Ok(409) => Outcome::Failed("busy with another upload".into()),
                Ok(other) => Outcome::Failed(format!("answered an upload with {other}")),
                Err(e) => Outcome::Failed(format!("the upload did not arrive: {e}")),
            }
        }
        Op::Delete { sha, .. } => match http.delete(format!("{base}/{sha}")).send().await.map(|r| r.status().as_u16()) {
            Ok(204 | 404) => Outcome::Deleted { sha },
            // Playing it: tried again on the next pass.
            Ok(409) => Outcome::Failed("still playing a clip it was asked to delete".into()),
            Ok(other) => Outcome::Failed(format!("answered a delete with {other}")),
            Err(e) => Outcome::Failed(format!("the delete did not arrive: {e}")),
        },
    }
}

// ── The stream ────────────────────────────────────────────────────────────────

/// One running timeline's stream clips for one node, as the sender renders them.
pub struct StreamDeck {
    pub id: u128,
    pub running: bool,
    pub transport: pult_render::Transport,
    pub end: Option<pult_audio::mix::End>,
    pub voices: Vec<pult_audio::Voice>,
}

pub enum StreamMessage {
    Decks(Vec<StreamDeck>),
    /// Send every packet to these from now on, in the same session.
    Retarget(Vec<SocketAddr>),
    Stop,
}

/// Where a node's stream goes: the address it prefers, or every address it is on
/// where its fixture asks for a redundant stream. The node keeps whichever copy of a
/// frame arrives first and drops the other (OpenHaunt docs/13).
pub fn stream_targets(entry: &DeviceEntry, port: u16) -> Vec<SocketAddr> {
    let mut targets: Vec<SocketAddr> = Vec::new();
    for ip in entry.paths() {
        let Ok(ip) = ip.parse::<std::net::IpAddr>() else { continue };
        let target = SocketAddr::new(ip, port);
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    if !entry.redundant_stream {
        targets.truncate(1);
    }
    targets
}

fn list(targets: &[SocketAddr]) -> String {
    targets.iter().map(SocketAddr::to_string).collect::<Vec<_>>().join(" and ")
}

/// The thread sending one node its stream.
///
/// Its own thread, timed from the **show clock** and never from a sound card: the
/// node has its own DAC and plays each sample at the show time the session maps it
/// to, so what matters is that every sample leaves a playout delay before then.
pub struct Streamer {
    targets: Vec<SocketAddr>,
    /// The port the node was told in `start`, which every target shares.
    port: u16,
    rate: u32,
    channels: usize,
    built: String,
    tx: std::sync::mpsc::Sender<StreamMessage>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Streamer {
    #[allow(clippy::too_many_arguments)]
    fn start(
        serial: String,
        targets: Vec<SocketAddr>,
        port: u16,
        rate: u32,
        channels: usize,
        delay_ms: u32,
        devices: DeviceHandle,
        decks: Vec<StreamDeck>,
        built: String,
    ) -> Streamer {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(StreamMessage::Decks(decks));
        let spawned = std::thread::Builder::new()
            .name(format!("node-stream-{serial}"))
            .spawn({
                let targets = targets.clone();
                move || stream(serial, targets, port, rate, channels, delay_ms, devices, rx)
            });
        let thread = spawned.map_err(|e| warn!("[audio] the stream sender could not start: {e}")).ok();
        Streamer { targets, port, rate, channels, built, tx, thread }
    }

    /// Whether a stream on `port` at this rate and width is this one, sent somewhere
    /// else. All three are in the `start` the node was told, so any of them changing
    /// needs a session of its own.
    fn same_session(&self, port: u16, rate: u32, channels: usize) -> bool {
        self.port == port && self.rate == rate && self.channels == channels
    }

    /// The same session, sent wherever the node is now. A node that moved from wifi to
    /// its cable listens on the same port on every path, so it notices nothing: same
    /// ssrc, same mapping, no `stop` and no new `start`.
    fn follow(&mut self, serial: &str, targets: Vec<SocketAddr>) {
        if self.targets != targets {
            info!("[audio] the stream to node {serial} goes to {} now", list(&targets));
            self.targets = targets.clone();
            self.send(StreamMessage::Retarget(targets));
        }
    }

    fn send(&self, message: StreamMessage) {
        let _ = self.tx.send(message);
    }

    /// Stop, and return once the thread has said so to the node.
    ///
    /// Waited for because the `stop` leaves from the thread and a successor's `start`
    /// from its own: told and not waited for, the old one could still be finishing a
    /// packet when the new one announced, and its `stop` reached the node second.
    async fn stop(mut self) {
        let _ = self.tx.send(StreamMessage::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
    }
}

/// Where the session maps RTP timestamp zero, and how far ahead each packet leaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Session {
    /// The show time RTP timestamp `ts` is heard at.
    pub at: u64,
    pub ts: u32,
    pub rate: u32,
}

impl Session {
    /// The show time the frame `frames` after `ts` is heard at.
    pub fn heard_at(&self, frames: u64) -> f64 {
        self.at as f64 + frames as f64 * 1_000.0 / self.rate as f64
    }
}

/// How far a timeline has got at a show time, with its end applied.
pub fn deck_position(transport: &pult_render::Transport, end: Option<pult_audio::mix::End>, show_ms: f64) -> f64 {
    let elapsed = (show_ms - transport.anchor_ms as f64).max(0.0) * transport.rate.max(0.0) as f64;
    let position = transport.position_at_anchor_ms as f64 + elapsed;
    match end.filter(|e| e.at_ms > 0.0) {
        Some(end) if position >= end.at_ms => match end.looped {
            true => position % end.at_ms,
            false => end.at_ms,
        },
        _ => position,
    }
}

#[allow(clippy::too_many_arguments)]
fn stream(
    serial: String,
    mut targets: Vec<SocketAddr>,
    port: u16,
    rate: u32,
    channels: usize,
    delay_ms: u32,
    devices: DeviceHandle,
    rx: std::sync::mpsc::Receiver<StreamMessage>,
) {
    let socket = match UdpSocket::bind(("0.0.0.0", 0)) {
        Ok(socket) => socket,
        Err(e) => {
            warn!("[audio] no socket to stream to node {serial} from: {e}");
            return;
        }
    };
    let ssrc = Uuid::new_v4().as_u128() as u32;
    let frames = pult_audio::rtp::frames_per_packet(rate, channels);
    let mut rtp = pult_audio::rtp::Packetiser::new(ssrc, 0);
    let mut mixer = pult_audio::Mixer::new(channels, rate, super::device::MAX_DECKS);
    let mut buffer = vec![0.0f32; frames * channels];
    let mut decks: Vec<StreamDeck> = Vec::new();

    // The node opens its port within [`STREAM_OPENS_MS`] of `start`, so the first sample
    // is heard that much later than a delay from now.
    let now = pult_schema::clock::now_ms();
    let mut session = Session { at: now + delay_ms as u64 + STREAM_OPENS_MS, ts: 0, rate };
    let mut sent_frames: u64 = 0;
    let announce = |session: &Session| {
        devices.audio(
            serial.clone(),
            "stream",
            serde_json::json!({
                "op": "start", "port": port, "rate": rate, "channels": channels,
                "ssrc": ssrc, "ts": session.ts, "at": session.at, "gain": 1.0,
            }),
        );
    };
    announce(&session);
    let mut announced = Instant::now();
    let mut rebuild = true;

    loop {
        loop {
            match rx.try_recv() {
                Ok(StreamMessage::Decks(new)) => {
                    decks = new;
                    rebuild = true;
                }
                Ok(StreamMessage::Retarget(to)) => targets = to,
                Ok(StreamMessage::Stop) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    devices.audio(
                        serial.clone(),
                        "stream",
                        serde_json::json!({ "op": "stop", "fade_ms": 50 }),
                    );
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        let heard = session.heard_at(sent_frames);
        let now = pult_schema::clock::now_ms() as f64;
        // A machine that slept, or a clock that stepped: far behind, so the mapping is
        // moved on rather than a backlog of sound nobody will hear being sent at once.
        if now + (delay_ms as f64) - heard > 500.0 {
            session = Session {
                at: (now as u64) + delay_ms as u64,
                ts: rtp.timestamp,
                rate,
            };
            sent_frames = 0;
            announce(&session);
            announced = Instant::now();
            rebuild = true;
            continue;
        }
        if heard > now + delay_ms as f64 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        if rebuild {
            // Every deck put back where its timeline is at the show time of the next
            // frame, so a locate, a play and an edit are all the same act.
            for deck in mixer.decks().iter().map(|d| d.id).collect::<Vec<_>>() {
                mixer.remove(deck);
            }
            for deck in &decks {
                if !deck.running {
                    continue;
                }
                let mut built = pult_audio::Deck::new(
                    deck.id,
                    deck_position(&deck.transport, deck.end, heard),
                    deck.voices.clone(),
                );
                // Played with the anchor still to come, as a count-in is: the stream
                // is already running, and sends silence until the anchor is heard.
                built.wait_ms = (deck.transport.anchor_ms as f64 - heard).max(0.0);
                built.rate = deck.transport.rate.max(0.0) as f64;
                built.end = deck.end;
                let _ = mixer.put(Box::new(built), false);
            }
            rebuild = false;
        }
        mixer.render(&mut buffer);
        let packet = rtp.packet(&buffer, channels);
        // Every copy the same packet: a node on two paths keeps whichever arrives first.
        for target in &targets {
            if let Err(e) = socket.send_to(&packet, target) {
                debug!("[audio] a packet to node {serial} at {target} did not leave: {e}");
            }
        }
        sent_frames += frames as u64;

        // Said again every few seconds with the same mapping, which re-anchors nothing:
        // a node that rebooted mid-stream has forgotten the session and picks it up.
        if announced.elapsed() > Duration::from_secs(5) {
            let first = session.ts;
            announce(&Session { ts: first, ..session });
            announced = Instant::now();
        }
    }
}

#[cfg(test)]
#[path = "nodes_tests.rs"]
mod tests;
