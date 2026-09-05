//! MVR-xchange: what the show settles, and what the console is currently doing.
//!
//! Two halves that must not be confused. [`XchangeSettings`] is show data — which
//! group, which mode, whether it is on at all — and it is PERSISTED, travels in the
//! showfile and is the same on every station of a session. [`XchangeState`] is what
//! is happening right now: who has been found, what they say they have, and whether
//! this station is the one holding the connections.
//!
//! # One client per show, hosted by the leader
//!
//! An MVR-xchange client is one participant in a group, and a pult session is several
//! stations replicating one show. If every station advertised, a single commit from a
//! previz would arrive at all of them and each would import the same file into the
//! same replicated show — three plans, three gestures, racing. So **only the leader is
//! on the wire**, and the identity it presents is the *show's*: [`station_uuid`] is a
//! v5 over the show id, so a failover reads to a peer as one console that moved rather
//! than one that vanished and another that appeared. `StationName` is the show's name
//! for the same reason.
//!
//! # And the state reaches every station
//!
//! Which leaves an operator at a follower unable to see an exchange their own console
//! is running. So the leader pushes this state down the sync link and every station
//! writes it to its own LOCAL `xchange` path — the shape `LogLines` and `OutputTraffic`
//! already have, and deliberately *not* a SYNCED entity: a discovered laptop appearing
//! on the LAN is not an operation, has no author, and has no business in anybody's
//! undo stack or in the History panel.
//!
//! [`station_uuid`]: XchangeState::station_uuid

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// The namespace `station_uuid` is derived in.
///
/// Fixed for ever: change it and every console on the network sees this show's station
/// disappear and a stranger take its place.
const STATION_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6d, 0x76, 0x72, 0x78, 0x63, 0x68, 0x67, 0x2d, 0x70, 0x75, 0x6c, 0x74, 0x73, 0x74, 0x61, 0x74,
]);

/// The station uuid this show presents to a group.
///
/// A v5 over the show id, so it is the same on every station and across restarts, and
/// survives a leader moving — which is what the specification's "persistent across
/// multiple start-ups of the same software on the same computer" is *for*, even though
/// it is not what it says.
pub fn station_uuid_for(show_id: Uuid) -> Uuid {
    Uuid::new_v5(&STATION_NAMESPACE, show_id.as_bytes())
}

/// How this console reaches its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum XchangeMode {
    /// mDNS under `<group>._mvrxchange._tcp.local.`, and the protocol's own framing.
    /// No configuration, no server, and what a show LAN actually runs.
    #[default]
    Tcp,
    /// Join a WebSocket host somebody else is running, at [`XchangeSettings::url`].
    WebSocket,
    /// Be that host, on the port already serving the console's own page.
    WebSocketHost,
}

/// What the show says about its exchange.
///
/// Show data rather than a station preference, and the leader moving is the reason:
/// anything kept per station would change group on a failover, which is the one thing
/// this must not do. A station can still refuse to take part at all — that veto lives
/// in `preferences.toml`, because it is a fact about the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct XchangeSettings {
    /// Off until somebody switches it on, like grandMA3's Enable. A console that has
    /// never been asked to share a rig should not appear on anybody's network.
    #[serde(default)]
    pub enabled: bool,
    /// The mDNS sub-service name in TCP mode. The group *is* the address, which is why
    /// no message carries one.
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub mode: XchangeMode,
    /// The host to join in [`XchangeMode::WebSocket`]. Ignored in the other two.
    #[serde(default)]
    pub url: String,
}

fn default_group() -> String {
    "Default".to_string()
}

impl Default for XchangeSettings {
    fn default() -> Self {
        XchangeSettings {
            enabled: false,
            group: default_group(),
            mode: XchangeMode::default(),
            url: String::new(),
        }
    }
}

/// Why the exchange is not running, in the words the panel prints.
///
/// A closed enum rather than a string, so a station that is off for a reason a build
/// does not know still says *something* — and so the panel cannot invent a reason of
/// its own that drifts from the one the station acted on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum XchangeIdle {
    /// The show has not switched it on.
    NotEnabled,
    /// This machine's `preferences.toml` refuses to take part at all.
    RefusedByStation,
    /// Another station of this session is the one on the wire.
    AnotherStationIsLeading,
    /// There is no show open, so there is no group and nothing to share.
    NoShow,
    /// It tried and could not — the message says what happened.
    Failed(String),
}

/// One MVR-xchange client, as seen from here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct XchangeStation {
    pub station_uuid: Uuid,
    pub station_name: String,
    /// The application on the other end, as it names itself.
    #[serde(default)]
    pub provider: String,
    /// Where it was found, for a person diagnosing a network rather than for code.
    #[serde(default)]
    pub address: String,
    /// True for this console's own row. A group lists everybody including us, and a
    /// panel that hid us would leave an operator unable to see what their own station
    /// is telling the room.
    #[serde(default)]
    pub is_us: bool,
    /// Whether it has sent us `MVR_JOIN`. Discovery and membership are different
    /// things: a station can be advertising and not have joined this group.
    #[serde(default)]
    pub joined: bool,
}

/// A revision somebody announced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct XchangeCommit {
    pub file_uuid: Uuid,
    pub station_uuid: Uuid,
    /// Who made it, resolved for the panel. Empty where the station has since gone.
    #[serde(default)]
    pub station_name: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub file_size: u64,
    /// Made by this console.
    #[serde(default)]
    pub ours: bool,
    /// The bytes are in this station's cache, so a request for it can be answered.
    ///
    /// Ours and *here* are not the same claim: after a failover the new leader holds
    /// the show's commits and none of the old leader's files, and it announces only
    /// what it actually has.
    #[serde(default)]
    pub here: bool,
    /// Console milliseconds when this console learned of it.
    #[serde(default)]
    pub at_ms: i64,
}

/// Somebody has asked this group to move.
///
/// Kept rather than acted on: `MVR_NEW_SESSION_HOST` arrives from an unauthenticated
/// station on the LAN and names a host to connect to, so following it without asking
/// would let anything on the network redirect this console's exchange wherever it
/// liked. The panel prints who asked and where, and a person decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingHost {
    pub from_station: Uuid,
    #[serde(default)]
    pub from_name: String,
    /// Exactly one of these is set, which is what the specification requires and what
    /// this console checks before offering it to anybody.
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub service_url: String,
}

/// Something a person asked the exchange to do.
///
/// Carried on the sync link as well as raised locally: only the leader is on the wire,
/// so an operator standing at a follower has their ask relayed to the station that can
/// carry it out. The user id travels with it, which is the whole of the attribution
/// rule — an applied commit belongs to the person who clicked, from whichever station
/// they clicked at, and it is their Ctrl-Z that takes it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum XchangeAsk {
    /// Export the rig, keep it, and tell the group about it.
    Commit { comment: String, user_id: Uuid },
    /// Fetch a commit and import it, in one act.
    Apply { file_uuid: Uuid, user_id: Uuid },
    /// Answer an `MVR_NEW_SESSION_HOST` that is waiting for a person.
    FollowHost { follow: bool },
}

/// What the exchange is doing, on whichever station is doing it.
///
/// LOCAL on every station and pushed down the sync link by the leader — see the module
/// docs for why that rather than a SYNCED entity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct XchangeState {
    /// Running, and on this station.
    #[serde(default)]
    pub running: bool,
    /// Why not, where it is not.
    #[serde(default)]
    pub idle: Option<XchangeIdle>,
    /// The station currently holding the connections, so a follower's panel can say
    /// where the exchange is rather than only that it is elsewhere.
    #[serde(default)]
    pub on_station: String,
    #[serde(default)]
    pub settings: XchangeSettings,
    #[serde(default)]
    pub station_uuid: Uuid,
    #[serde(default)]
    pub station_name: String,
    /// Where this console is reachable, for the panel to print — an address in TCP
    /// mode, a URL where it is hosting.
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub stations: Vec<XchangeStation>,
    #[serde(default)]
    pub commits: Vec<XchangeCommit>,
    #[serde(default)]
    pub pending_host: Option<PendingHost>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_show_s_station_uuid_is_the_same_every_time() {
        let show = Uuid::from_u128(0x1234);
        assert_eq!(station_uuid_for(show), station_uuid_for(show));
        assert_ne!(station_uuid_for(show), station_uuid_for(Uuid::from_u128(0x1235)));
    }

    /// The one value in this file nothing may change. A different namespace makes
    /// every console on the network treat this show as a station it has never met.
    #[test]
    fn the_station_namespace_is_pinned() {
        assert_eq!(
            station_uuid_for(Uuid::from_u128(1)).to_string(),
            "a00a16a4-cff0-50eb-96af-e5787ef5c072"
        );
    }

    #[test]
    fn a_show_that_says_nothing_is_off_and_in_the_default_group() {
        let settings = XchangeSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.group, "Default");
        assert_eq!(settings.mode, XchangeMode::Tcp);
    }
}
