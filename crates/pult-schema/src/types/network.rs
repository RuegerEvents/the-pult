//! Which cable a service goes out on.
//!
//! A console at a venue has more than one interface — a house LAN, an isolated
//! lighting network with no route to anything, sometimes a wifi the tablet is on —
//! and until now nothing here said which one anything used. Every service picked for
//! itself, or let the operating system pick: the mDNS daemons bound every interface
//! they could find, sACN's multicast left by whatever the route table said, and the
//! address a station *advertised* to its peers came from `local_ipv4`, which finds
//! the interface with the default route. On a console with a house LAN and a show
//! LAN the default route is the house one, so the address a peer was told to dial
//! for the show was decided by which cable reaches the internet.
//!
//! Three rules hold this module together, and each is here rather than at a call
//! site so that six services cannot answer the same question six ways.
//!
//! **A name or an address, resolved to an address.** [`resolve`] parses what it was
//! given as an `IpAddr` first and falls back to treating it as an interface name.
//! The forms cannot be confused — an address never parses as a name — and both are
//! wanted: a name survives a DHCP lease where an address does not, and an address is
//! the only way to say *which* alias on a NIC carrying both 2.0.0.x and 10.0.0.x,
//! which is an ordinary Art-Net setup.
//!
//! **Two absences, two answers.** Being told nothing is not the same as being told
//! something wrong. A service that names no interface falls through to the station
//! preference and then to every interface, which is what this console did before any
//! of this existed and is why no show has to be edited. A service that names an
//! interface the machine has not got **refuses**, visibly, and does not quietly bind
//! everything — a console silently offering the show network's traffic on the house
//! LAN is the failure the setting exists to prevent.
//!
//! **IPv4 only, and say so.** Art-Net and sACN are IPv4 by specification, and a v6
//! link-local address needs a scope id to bind at all. A v6 literal is refused by
//! name rather than accepted and bound to nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{events::operation::NodeId, PultSchema};

/// One of this machine's network interfaces, as a station reports it.
///
/// On the SYNCED `Station` row rather than LOCAL, so a console in the booth can fill
/// in a dropdown for the stage rack's Art-Net cable without anybody walking over —
/// the same argument `ClockSync` makes for being on that row.
///
/// Addresses are plural because an aliased NIC is the case that makes a name
/// insufficient. They are strings on the wire because that is what an address is to
/// a panel; [`NetInterface::ipv4`] is how anything here reads them back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NetInterface {
    /// What the operating system calls it: `en0`, `eth0`, a GUID on Windows.
    pub name: String,
    /// Every IPv4 address on it, in the order the machine reports them.
    pub addresses: Vec<String>,
    /// False only where the machine says `Down`. `Unknown` — which is what Linux
    /// reports for loopback — is not a claim that the interface is unusable, and
    /// reading it as one would hide the interface a dev station actually binds.
    pub up: bool,
    /// Loopback is included and flagged rather than excluded, unlike the throughput
    /// figures beside it on the row: `demo.sh` and the tests legitimately bind it,
    /// and an operator looking at a console that only talks to itself needs to see
    /// that this is what it is doing.
    pub loopback: bool,
}

impl NetInterface {
    /// The addresses that parsed, which is every one this module can bind.
    pub fn ipv4(&self) -> impl Iterator<Item = Ipv4Addr> + '_ {
        self.addresses.iter().filter_map(|a| a.parse::<Ipv4Addr>().ok())
    }
}

/// Why an interface setting could not be turned into an address to bind.
///
/// A type rather than a string so that the panel, the log line and the test all say
/// the same thing about the same condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum InterfaceError {
    /// No interface of that name on this machine.
    Unknown(String),
    /// The interface is here and has no IPv4 address on it.
    NoAddress(String),
    /// An address literal that is on none of this machine's interfaces.
    NotHere(String),
    /// An IPv6 literal. Refused by name rather than bound to nothing.
    NotIpv4(String),
}

impl std::fmt::Display for InterfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterfaceError::Unknown(name) => {
                write!(f, "no interface called {name} on this machine")
            }
            InterfaceError::NoAddress(name) => write!(f, "{name} has no IPv4 address"),
            InterfaceError::NotHere(addr) => {
                write!(f, "{addr} is not an address of any interface here")
            }
            InterfaceError::NotIpv4(wanted) => {
                write!(f, "{wanted} is not IPv4, and this console binds IPv4 only")
            }
        }
    }
}

/// Turn what somebody wrote down into an address to bind.
///
/// The whole of the name-or-address rule, in one place because six services and
/// three connectors ask it. An address literal is checked against the machine rather
/// than trusted: pinning an alias that is not there is being told something wrong,
/// which is the case that has to refuse.
pub fn resolve(wanted: &str, interfaces: &[NetInterface]) -> Result<Ipv4Addr, InterfaceError> {
    let wanted = wanted.trim();
    match wanted.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) => {
            if interfaces.iter().flat_map(|i| i.ipv4()).any(|a| a == addr) {
                Ok(addr)
            } else {
                Err(InterfaceError::NotHere(addr.to_string()))
            }
        }
        Ok(IpAddr::V6(_)) => Err(InterfaceError::NotIpv4(wanted.to_string())),
        Err(_) => match interfaces.iter().find(|i| i.name == wanted) {
            Some(interface) => interface
                .ipv4()
                .next()
                .ok_or_else(|| InterfaceError::NoAddress(wanted.to_string())),
            None => Err(InterfaceError::Unknown(wanted.to_string())),
        },
    }
}

/// Which service an interface setting is about.
///
/// The four that bind or advertise on their own, plus one per configured output —
/// because an output names its interface on its own row, per station, and its fault
/// belongs beside the others rather than in a second place.
///
/// Art-Net and sACN are not variants: `[network] artnet` and `sacn` are the
/// *fallback* an output row takes when it names no interface of its own, and the
/// thing that actually binds is the output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum NetService {
    /// The page, the WebSocket, and a hosted MVR-xchange group.
    Http,
    /// `_pult._tcp` and the sync listener: who this console is in a session with.
    Session,
    /// `_mvrxchange._tcp` and its listener.
    MvrXchange,
    /// The OpenHaunt device browser and the MQTT broker this station runs for them.
    OpenHaunt,
    /// One configured output, by its row id.
    Output(Uuid),
}

/// What one service was told, what it managed, and what went wrong.
///
/// On the station row so that "why can the previz not see us" is answerable from a
/// different console than the broken one, which is how it is usually asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceBinding {
    pub service: NetService,
    /// A name for the panel to print: the output's own name, or the service's.
    pub label: String,
    /// What the preference or the row said, if anything. `None` is "told nothing",
    /// which is the case that falls through rather than the case that refuses.
    pub wanted: Option<String>,
    /// The address it resolved to and is bound on.
    pub bound: Option<String>,
    /// Why it is not running, where it is not.
    pub fault: Option<InterfaceError>,
}

/// What one station's networking looks like: what it has, and what it did with it.
///
/// **A collection of its own rather than fields on the `Station` row, and the reason is
/// a measurement.** That row is one reading of a machine at one moment — CPU, memory,
/// frame costs — replaced whole every two seconds because half of an old reading beside
/// half of a new one is not a state the machine was ever in. An interface list is not
/// like that at all: it is an *inventory*, and it changes when somebody plugs a cable
/// in, which is to say almost never.
///
/// Carried on that row it was **57% of it** — 2517 bytes against 1081, on a laptop with
/// twenty-four interfaces of which three have an address — rewritten every two seconds
/// and logged to the oplog like any SYNCED write, for names nobody had changed. Here it
/// is written only when it differs from what was last published, and the whole-row rule
/// next door goes back to being exactly true rather than approximately.
///
/// SYNCED and not PERSISTED, for the reason `Station` is: a showfile travels, and which
/// cards are in a particular machine does not travel with it. Keyed by the station's
/// `NodeId`, so a console in the booth can read the stage rack's cabling — which is the
/// whole reason this is replicated at all, since "why can the previz not see us" is
/// almost never asked at the console that is broken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "station_networks")]
pub struct StationNetwork {
    /// The station's `NodeId`, so the row is stable across restarts and every node
    /// writes to the same key for the same machine.
    #[pult(lifecycle = SYNCED, primary_key)]
    pub id: Uuid,
    /// Every interface the machine has, in a stable order.
    #[pult(lifecycle = SYNCED)]
    pub interfaces: Vec<NetInterface>,
    /// What each service was told, what it managed, and what went wrong.
    #[pult(lifecycle = SYNCED)]
    pub bindings: Vec<ServiceBinding>,
    /// When any of the above last actually moved — which is *not* when it was last
    /// looked at. A station that has been up all afternoon with nothing replugged
    /// should say so rather than claim it noticed something two seconds ago.
    #[pult(lifecycle = SYNCED)]
    pub changed_at: DateTime<Utc>,
}

// ── sACN priority ─────────────────────────────────────────────────────────────

/// What the leader sends at, and the top of the ladder.
pub const SACN_PRIORITY_LEADER: u8 = 100;
/// The gap between one station and the next below it.
pub const SACN_PRIORITY_STEP: u8 = 10;
/// The bottom of the ladder. Stations past the tenth share it: E1.31 reads 0 as
/// "do not use this source", so stepping to zero would silence a backup rather than
/// rank it, and sharing a floor is the honest end of a finite ladder.
pub const SACN_PRIORITY_FLOOR: u8 = 10;
/// E1.31's ceiling.
pub const SACN_PRIORITY_MAX: u8 = 200;

/// How an sACN output decides the priority byte it puts in every packet.
///
/// Art-Net has no priority mechanism at all, which is why it is the kind that must
/// go from one station; E1.31 has this, which is what makes several stations sending
/// one universe a defined thing rather than a race. So an sACN output may run
/// everywhere, and this is what makes that well-defined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SacnPriority {
    /// Derived from the session: the leader at 100, every other station in a slot
    /// below it. Nobody types a number and a failover is the receiver's arbitration.
    Auto,
    /// A number per station, for a rig where another desk is already at a known
    /// priority. A station this map does not name takes its `Auto` number rather
    /// than a default — being unnamed is being told nothing, which is the same
    /// fall-through rule the interfaces map follows.
    Manual(BTreeMap<NodeId, u8>),
}

impl Default for SacnPriority {
    fn default() -> Self {
        SacnPriority::Auto
    }
}

impl SacnPriority {
    /// The byte this station sends at, given the slot it holds.
    ///
    /// `slot` is what [`claim_slot`] worked out and is ignored for a leader, which
    /// is always at the top of the ladder.
    pub fn byte_for(&self, node: NodeId, slot: u8, is_leader: bool) -> u8 {
        let auto = if is_leader { SACN_PRIORITY_LEADER } else { slot };
        match self {
            SacnPriority::Auto => auto,
            SacnPriority::Manual(map) => {
                map.get(&node).copied().map(|p| p.min(SACN_PRIORITY_MAX)).unwrap_or(auto)
            }
        }
    }
}

/// Every slot below the leader, highest first.
pub fn sacn_slots() -> impl Iterator<Item = u8> {
    let mut next = SACN_PRIORITY_LEADER;
    std::iter::from_fn(move || {
        next = next.saturating_sub(SACN_PRIORITY_STEP);
        (next >= SACN_PRIORITY_FLOOR).then_some(next)
    })
}

/// Which slot this station takes, given what the others are holding.
///
/// Self-claimed rather than handed out: every station sees every `Station` row, so
/// each works out the lowest slot nobody with a better claim is holding and writes
/// it into its own row. No leader is involved, no assignment table is replicated,
/// and nothing new goes on the wire.
///
/// **A station defers only to lower node ids.** That is what makes it converge in a
/// pass and settle without churn: the lowest id is blocked by nobody, the next is
/// blocked only by it, and so on down. Two stations that pick the same slot in the
/// same instant see each other's row on the next tick and the higher id moves.
///
/// **And a remembered slot is kept where it is free**, which is the whole point of
/// the ladder being sticky: `preferences.toml` carries the number across a restart,
/// so a console that reboots during the interval comes back at the priority the
/// receivers last heard it at rather than wherever the gap happened to be.
pub fn claim_slot(me: NodeId, remembered: Option<u8>, others: &[(NodeId, u8)]) -> u8 {
    let blocked: BTreeSet<u8> =
        others.iter().filter(|(id, _)| *id < me).map(|(_, slot)| *slot).collect();
    if let Some(slot) = remembered {
        if !blocked.contains(&slot) && slot >= SACN_PRIORITY_FLOOR && slot < SACN_PRIORITY_LEADER {
            return slot;
        }
    }
    sacn_slots().find(|slot| !blocked.contains(slot)).unwrap_or(SACN_PRIORITY_FLOOR)
}

#[cfg(test)]
mod tests;
