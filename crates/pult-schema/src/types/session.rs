use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::events::operation::NodeId;

/// A session discovered on the local network via mDNS.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiscoveredSession {
    pub session_id: Uuid,
    pub show_id: Uuid,
    pub show_name: String,
    /// Sync endpoint as "ip:port" string.
    pub sync_addr: String,
}

/// The local node's session state — LOCAL lifecycle:
/// broadcast to connected frontends but not persisted or synced to peers.
/// The engine holds this as `ShowState.session`; the SessionManager is the only writer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionState {
    /// This station's own id. Needed by anything that records which station did
    /// something — an output names the one that sends it. Defaulted so a payload
    /// written before this field existed still reads.
    #[serde(default)]
    pub node_id: Option<NodeId>,
    pub is_advertising: bool,
    pub is_follower: bool,
    pub session_id: Option<Uuid>,
    pub discovered: Vec<DiscoveredSession>,
}
