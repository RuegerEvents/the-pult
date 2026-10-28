//! What this node knows about the OpenHaunt I/O nodes on the network.
//!
//! LOCAL lifecycle throughout: presence, health, and the broker address are facts
//! about *this* node's view of the network, not about the show. A follower browsing
//! the same mDNS domain sees the same devices and reaches its own conclusions, and
//! the thing that is genuinely shared — the fixture an adopted device became — is
//! an ordinary PERSISTED entity that syncs like everything else.
//!
//! Plain serde rather than `PultSchema`: there is no table, no id, and nothing here
//! is addressed by path beyond the top-level key.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::types::openhaunt::{EffectCapability, NodeDescription};
use crate::types::pixels::NodePixels;
use crate::PultSchema;

/// What a node last said about itself on its health topic.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeviceHealth {
    pub uptime_s: u64,
    pub temperature_c: Option<f32>,
    pub poe_class: Option<u8>,
    pub errors: Vec<String>,
    pub reported_at: Option<DateTime<Utc>>,
}

/// One OpenHaunt node, as discovered and then as adopted.
///
/// Everything down to `caps` comes from the mDNS TXT record, so a node is known
/// before anything has been asked of it — the protocol's "a node is discovered, not
/// configured" applies to the console as much as to the node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiscoveredDevice {
    pub serial: String,
    pub name: String,
    pub host: String,
    pub ip: String,
    pub port: u16,
    pub fw: String,
    pub protocol_version: String,
    /// Module type id, e.g. `0x0002` for the eight-input module.
    pub module_type: u16,
    pub module_name: String,
    pub module_serial: String,
    pub module_rev: String,
    pub caps: Vec<String>,
    /// What the node says its ports are, from `GET /api/v1/info`.
    ///
    /// None until the node has answered — and on firmware that predates
    /// self-description, for good. Adoption needs it: the console has no table to
    /// fall back on, which is the point.
    #[serde(default)]
    pub description: Option<NodeDescription>,
    /// Which of its ports can trace a shape on their own, read from the same `/info`
    /// body but deliberately not from `description` — see
    /// [`crate::types::openhaunt::effect_capability_from`] for why keeping it out of
    /// the description matters.
    #[serde(default)]
    pub effects: Option<EffectCapability>,
    /// What the node can play, from the `audio` object of the same `/info` body. None
    /// on a node without a loudspeaker, and on firmware older than audio.
    ///
    /// Beside `description` rather than in it, for the reason `effects` is: the fixture
    /// type's id hashes the description, and a node that learned to make sound must not
    /// orphan every parameter patched against the type it had before.
    #[serde(default)]
    pub audio: Option<NodeAudio>,
    /// What the node last said on `audio/status`: its voices, its stream, whether it
    /// has the console's clock.
    #[serde(default)]
    pub audio_status: Option<NodeAudioStatus>,
    /// What the node says about its pixel strips, from the `pixels` object of `/info`.
    /// None on a node with no strip output. Beside `description` for the reason
    /// `effects` is.
    #[serde(default)]
    pub pixels: Option<NodePixels>,
    /// The fixture each head other than 0 became, keyed by head. Head 0's is
    /// `adopted_fixture_id`.
    #[serde(default)]
    pub head_fixtures: BTreeMap<u8, Uuid>,
    /// What went wrong putting this node into the shape the show says, in words: a
    /// strip the node refused, a virtual module that wins over its strips.
    #[serde(default)]
    pub faults: Vec<String>,
    /// The module switches mains voltage. The panel says so before anything is
    /// adopted, because the consequence of a mistake here is not a dark light.
    pub is_mains: bool,
    /// Seen on the network right now. An adopted device that goes quiet stays in
    /// the list — its fixture is still patched, and the operator needs to know.
    pub online: bool,
    pub adopted_fixture_id: Option<Uuid>,
    pub health: Option<DeviceHealth>,
    pub last_seen: DateTime<Utc>,
    /// Every address the node says it is on, the path it prefers first, from its
    /// retained `openhaunt/<sn>/net`. Empty for a node that has not said, which is
    /// firmware from before the topic: `ip` is where it is then.
    #[serde(default)]
    pub net: Vec<NodeAddress>,
}

/// One path a node is on, as `openhaunt/<sn>/net` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeAddress {
    /// `ethernet` or `wifi`, as the node names it.
    pub transport: String,
    pub ip: String,
}

/// A node's loudspeaker, as `GET /api/v1/info` describes it. See the OpenHaunt
/// protocol's audio extension (`node/docs/13-audio.md`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeAudio {
    /// The rate the node mixes at. Anything else it is sent is resampled there.
    pub rate: u32,
    /// How many clips play at once. The stream is one more, beside these.
    pub voices: u32,
    pub outputs: Vec<NodeAudioOutput>,
    pub clips: NodeClipBudget,
    #[serde(default)]
    pub stream: Option<NodeStreamCaps>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeAudioOutput {
    /// `line` or `speaker` on the V0 boards.
    pub id: String,
    pub name: String,
    /// `line` or `amp`. An amp output is not a line out, and the panel says so.
    pub kind: String,
    pub channels: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeClipBudget {
    #[serde(default)]
    pub formats: Vec<String>,
    pub bytes_total: u64,
    pub bytes_free: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeStreamCaps {
    pub payload: String,
    pub max_buffer_ms: u32,
    /// The UDP port the node listens on for the stream. Absent is 5004; a node that
    /// shares an address with others (a simulator) names its own.
    #[serde(default)]
    pub port: Option<u16>,
}

impl NodeAudio {
    /// The `audio` object of an `/info` body. None where it is absent, which is what a
    /// node without a loudspeaker says.
    pub fn from_info(info: &serde_json::Value) -> Option<NodeAudio> {
        serde_json::from_value(info.get("audio")?.clone()).ok()
    }

    /// The most channels any output takes: two on a board with a line out, one where
    /// there is only the amplifier. Never more than two, which is what the node takes.
    pub fn channels(&self) -> u16 {
        self.outputs.iter().map(|o| o.channels).max().unwrap_or(1).clamp(1, 2)
    }
}

/// One `openhaunt/<sn>/audio/status` message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeAudioStatus {
    #[serde(default)]
    pub voices: Vec<NodeVoice>,
    #[serde(default)]
    pub stream: Option<NodeStreamStatus>,
    #[serde(default)]
    pub underruns: u64,
    #[serde(default)]
    pub master: f32,
    /// The node has the console's clock. Until it does, every `at` is off by however
    /// far apart the two clocks are.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub reported_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeVoice {
    pub voice: String,
    #[serde(default)]
    pub clip: String,
    #[serde(default)]
    pub pos_ms: u64,
    #[serde(default, rename = "loop")]
    pub looped: bool,
    #[serde(default)]
    pub gain: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeStreamStatus {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub depth_ms: u32,
    #[serde(default)]
    pub late: u64,
    #[serde(default)]
    pub missing: u64,
    #[serde(default)]
    pub resyncs: u64,
    /// What reached the node's receiving task, by RTP sequence number (firmware
    /// from 2026-10-28 on; zero from anything older). `seq_lost` is packets that never
    /// reached it — lost on the wire, or dropped by the node's own stack with its queue
    /// full — where `missing` is frames the buffer did not have when they were played.
    #[serde(default)]
    pub packets: u64,
    #[serde(default)]
    pub seq_lost: u64,
    #[serde(default)]
    pub reordered: u64,
    #[serde(default)]
    pub duplicates: u64,
    #[serde(default)]
    pub foreign: u64,
    /// The longest the node's receiving task went between two packets, since its last
    /// status and since the session began. Past its socket queue (6 packets, 30 ms) is
    /// a queue that overflowed while the task was not running.
    #[serde(default)]
    pub rx_gap_ms: u32,
    #[serde(default)]
    pub rx_gap_max_ms: u32,
}

/// What the station driving the nodes has made of each one's clip store: the LOCAL
/// `node_audio` path, keyed by serial.
///
/// LOCAL, and written only by that station, for the reason `output_status` is: it is
/// this station's conversation with the node, and a follower is having none.
pub type NodeAudioStates = BTreeMap<String, NodeAudioState>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeAudioState {
    /// The clips the node holds, by the sha of the bytes it holds.
    pub stored: Vec<String>,
    pub bytes_total: u64,
    pub bytes_free: u64,
    /// The clip being uploaded now, if one is.
    pub uploading: Option<String>,
    /// How many clips the show wants on this node and does not have yet.
    pub pending: u32,
    /// An RTP stream is going to this node.
    pub streaming: bool,
    /// Why something is not on the node, in words.
    pub fault: Option<String>,
}

/// A node's audio status, coarse enough to replicate: what changes when something
/// starts, stops or goes wrong, and nothing that moves every second — no stream depth,
/// no voice positions. The full status stays in the driving station's LOCAL `devices`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeAudioSummary {
    /// The node has the console's clock.
    pub locked: bool,
    pub underruns: u64,
    /// How many clips it is sounding.
    pub voices: u32,
    /// Its stream is active.
    pub streaming: bool,
    /// The stream's packets that arrived after they should have been heard, that never
    /// arrived, and the times the node gave up on its mapping and started over — the
    /// node's own counts. They move only when something goes wrong, which is what lets
    /// them be here and the buffer depth not.
    #[serde(default)]
    pub late: u64,
    #[serde(default)]
    pub missing: u64,
    #[serde(default)]
    pub resyncs: u64,
    /// Packets that never reached the node's receiving task, and the longest it went
    /// without one — both only ever go up within a session, so they move only when
    /// something went wrong.
    #[serde(default)]
    pub seq_lost: u64,
    #[serde(default)]
    pub rx_gap_max_ms: u32,
}

impl NodeAudioSummary {
    pub fn of(status: &NodeAudioStatus) -> Self {
        let stream = status.stream.as_ref();
        NodeAudioSummary {
            locked: status.locked,
            underruns: status.underruns,
            voices: status.voices.len() as u32,
            streaming: stream.is_some_and(|s| s.active),
            late: stream.map_or(0, |s| s.late),
            missing: stream.map_or(0, |s| s.missing),
            resyncs: stream.map_or(0, |s| s.resyncs),
            seq_lost: stream.map_or(0, |s| s.seq_lost),
            rx_gap_max_ms: stream.map_or(0, |s| s.rx_gap_max_ms),
        }
    }
}

/// How a node's stream has gone since the station driving it started this session:
/// the sender's half, which only that station can see, beside what the node counted.
///
/// For finding a dropout after the fact. A gap in a streamed scare is one of three
/// things, and they read differently here: the **network** lost packets (`missing`
/// climbs, typically on wifi), the **sender** fell behind (`stalls`, a small
/// `lowest_lead_ms`), or the **node** ran dry with everything arriving (`underruns`
/// alone). Every counter is since `since_ms`, so a restarted stream starts clean, and
/// each one moves only when something goes wrong — the row is not rewritten while a
/// stream is healthy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeStreamHealth {
    /// The show time this session started.
    pub since_ms: u64,
    /// How many sessions this station has started to the node since it began driving
    /// it. More than one is a stream that was stopped and started again, which is a
    /// gap the length of the node's buffer each time.
    pub sessions: u32,
    /// Where the packets go, with how the node is reached there: `wifi` or `ethernet`.
    pub paths: Vec<NodeAddress>,
    /// How far ahead of being heard the stream is sent: the playout delay.
    pub delay_ms: u32,
    /// The least time any packet had between leaving and being heard. Near `delay_ms`
    /// is a sender keeping up; near zero or below is one that fell behind.
    pub lowest_lead_ms: Option<i64>,
    /// Times the sender fell behind by more than half the delay.
    pub stalls: u64,
    /// Times it fell so far behind (half a second) that it skipped ahead.
    pub skips: u64,
    /// Packets the operating system would not send.
    pub send_errors: u64,
    /// The node's counts, since the session started. `missing` is in the node's
    /// output frames — samples it had to play with nothing there — not packets.
    pub underruns: u64,
    pub late: u64,
    pub missing: u64,
    pub resyncs: u64,
    /// The rate the node mixes at, which is what `missing` is counted in.
    #[serde(default)]
    pub node_rate: u32,
    /// Packets that never reached the node's receiving task, since the session
    /// started, and the longest it has gone without one (newer firmware only).
    #[serde(default)]
    pub seq_lost: u64,
    #[serde(default)]
    pub rx_gap_max_ms: u32,
    /// The latest thing that went wrong, in words, and when (show time).
    pub last_incident: Option<String>,
    pub last_incident_ms: Option<u64>,
}

/// One node with a loudspeaker, as the station driving it sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DrivenNode {
    pub name: String,
    /// Seen on the network by the driving station.
    pub online: bool,
    /// On the driving station's broker, which is the only way it hears a `play`.
    pub on_broker: bool,
    pub audio: Option<NodeAudio>,
    pub store: Option<NodeAudioState>,
    pub summary: Option<NodeAudioSummary>,
    /// How its stream is going, while one is.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub stream: Option<NodeStreamHealth>,
}

/// What a station knows about the nodes' sound: the SYNCED `node_reports` collection,
/// keyed by the station's `NodeId`, one row per station.
///
/// **Only the station driving the nodes can say**: it holds the broker, it hears each
/// node's `audio/status`, it uploads the clips and streams. A station that is not
/// driving them has its own mDNS browse and nothing else, so its own view said a node
/// was offline, or not there at all, while the leader was playing through it (found at
/// the gig, 2026-10-25). So the driving station publishes what it knows here, and every
/// other station and page reads the driving row instead of guessing. Rewritten only
/// when something in it moved, like `station_audio`: the per-second parts of a node's
/// status are left out on purpose ([`NodeAudioSummary`]).
///
/// A row is read only while its station still has a `stations` row, so a driver that
/// died mid-show stops being believed when its station row is pruned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "node_reports")]
pub struct NodeReport {
    /// The station's `NodeId`.
    #[pult(lifecycle = SYNCED, primary_key)]
    pub id: Uuid,
    /// This station is the one driving the nodes. A row with this false says nothing.
    #[pult(lifecycle = SYNCED)]
    pub driving: bool,
    /// Every node with a loudspeaker, by serial.
    #[pult(lifecycle = SYNCED)]
    pub nodes: BTreeMap<String, DrivenNode>,
    /// Why a node clip is not playing: by timeline id, then clip id, then node serial.
    #[pult(lifecycle = SYNCED)]
    pub clip_faults: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
}

/// This node's view of the OpenHaunt devices around it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DevicesState {
    /// Keyed by serial, so a re-resolve updates rather than duplicates.
    pub discovered: BTreeMap<String, DiscoveredDevice>,
    /// Where adopted devices are told to publish, once there is a broker to name.
    pub broker_addr: Option<String>,
    /// Whether this node is the one driving the devices. Followers browse mDNS and
    /// show what they find, but do not adopt, configure, or command anything.
    pub active: bool,
}
