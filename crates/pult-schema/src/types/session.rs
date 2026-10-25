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

/// A station on the local network, found over mDNS whether or not it is in a session:
/// what can be invited into this one.
///
/// A session's own advert only exists while a station leads one, so a station on its
/// own — a Pi under the desk that was just switched on — was invisible to every other
/// console until somebody went to it and pressed Join.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NearbyStation {
    pub node_id: NodeId,
    pub hostname: String,
    /// Where its HTTP and WebSocket API answers, as `ip:port`, from here.
    pub http_addr: String,
    /// Where its sync link answers, as `ip:port`, from here: what joining it dials.
    pub sync_addr: String,
    /// The show it has open, by name, or empty with none open. Joining a session
    /// replaces it, which is why an invitation names it.
    pub show_name: String,
    /// The session it is in, or `None` on its own.
    pub session_id: Option<Uuid>,
    /// Whether it follows another station. Joining one that does would make this a
    /// follower of a follower, so it is that station's leader that has to be joined.
    pub follows: bool,
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
    /// Every other station on the network, in a session or not. Defaulted, so a
    /// payload written before this field existed still reads.
    #[serde(default)]
    pub nearby: Vec<NearbyStation>,
}
