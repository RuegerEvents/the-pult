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
    /// type's id hashes the description, and a node that learnt to make sound must not
    /// orphan every parameter patched against the type it had before.
    #[serde(default)]
    pub audio: Option<NodeAudio>,
    /// What the node last said on `audio/status`: its voices, its stream, whether it
    /// has the console's clock.
    #[serde(default)]
    pub audio_status: Option<NodeAudioStatus>,
    /// The module switches mains voltage. The panel says so before anything is
    /// adopted, because the consequence of a mistake here is not a dark light.
    pub is_mains: bool,
    /// Seen on the network right now. An adopted device that goes quiet stays in
    /// the list — its fixture is still patched, and the operator needs to know.
    pub online: bool,
    pub adopted_fixture_id: Option<Uuid>,
    pub health: Option<DeviceHealth>,
    pub last_seen: DateTime<Utc>,
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
