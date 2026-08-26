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
