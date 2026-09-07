//! Inputs: DMX arriving, so that a recording can exist.
//!
//! An input is [`super::output::OutputConfig`] read backwards, and it is deliberately
//! the same shape: a PERSISTED row with a kind, a station, a per-station interface map
//! and a universe filter, so the I/O panel is one panel and a console does not learn a
//! second vocabulary for the same cable.
//!
//! Two things differ, and both follow from a socket being one machine's.
//!
//! **There is no leader fallback.** An output with no `node_id` resolves to the
//! leader, because Art-Net cannot arbitrate between two senders and *something* has to
//! send. Nothing has to listen: a row that names no station means nobody is listening,
//! and the panel says so. Falling back to the leader would move a receiver's socket
//! every time a console failed over, which for an input is exactly wrong — the guest
//! console's cable is plugged into one machine.
//!
//! **The universe map is explicit and is a routing, not a filter.**
//! `universes` maps *wire universe* to *patch universe*, so the other console's 1 can
//! be this show's 5. An empty map listens to nothing at all, which is the honest
//! default: an input that guessed the identity mapping would decode a guest console's
//! universe 1 straight over the house rig's.
//!
//! And what arrives never becomes show state. A universe is 512 bytes forty times a
//! second per source; the merged images stay in the connector, LOCAL, drawn on demand
//! through the same `output.watch` a wire viewer already asks with. Nothing crosses
//! the sync link until somebody grabs or records.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{events::operation::NodeId, PultSchema};

/// Which protocol an input listens for.
///
/// No OpenHaunt: a node reports its own sensed values through its own protocol, and
/// those are already a fixture's `sensed_values`. This enum is about somebody else's
/// console putting a universe on a wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum InputKind {
    Artnet,
    Sacn,
}

/// One configured input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "inputs")]
pub struct InputConfig {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub kind: InputKind,
    /// Which station holds the socket. `None` is nobody, and stays nobody: see the
    /// module header for why there is no leader fallback here.
    #[pult(lifecycle = PERSISTED)]
    pub node_id: Option<NodeId>,
    /// Which interface to listen on, per station — an address on a NIC for Art-Net,
    /// and for sACN the interface each multicast group is joined on, which is the
    /// setting that actually decides whether the packets arrive.
    ///
    /// A map for the reason an output's is one: the row replicates and `en5` names a
    /// different cable on every machine.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub interfaces: BTreeMap<NodeId, String>,
    /// Wire universe → patch universe. Empty listens to nothing.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub universes: BTreeMap<u16, u16>,
    #[pult(lifecycle = PERSISTED)]
    pub enabled: bool,
}

impl InputConfig {
    /// Should this station be listening on this input?
    pub fn listens_on(&self, node_id: NodeId) -> bool {
        self.enabled && self.node_id == Some(node_id)
    }

    /// What this row says about which cable this station should listen on.
    pub fn interface_for(&self, node_id: NodeId) -> Option<&str> {
        self.interfaces.get(&node_id).map(String::as_str)
    }

    /// Which patch universe a wire universe lands in, if this input carries it.
    pub fn patch_universe(&self, wire: u16) -> Option<u16> {
        self.universes.get(&wire).copied()
    }
}

/// What one input has actually been hearing.
///
/// LOCAL, and for the same reason [`super::output::OutputStatus`] is: it describes a
/// socket on this machine. A silent input is the failure this exists to make visible —
/// a wrong interface, a universe nobody mapped and a cable in the wrong port all look
/// identical from the desk, and all three show up here as no packets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InputStatus {
    pub name: String,
    pub kind: String,
    /// Listening on this station. False for one owned elsewhere, or disabled.
    pub running: bool,
    pub packets_per_second: f32,
    pub last_packet: Option<DateTime<Utc>>,
    /// How many distinct senders are currently being merged.
    ///
    /// The figure that answers "is the guest console the only thing talking": two
    /// sources on one universe is legitimate and is also what a mis-set priority looks
    /// like, and nothing else on the panel would show it.
    pub sources: u16,
    pub error_count: u64,
    pub last_error: Option<String>,
}

/// Every input's status, keyed by config id: the LOCAL `input_status` path.
pub type InputStatuses = BTreeMap<String, InputStatus>;

#[cfg(test)]
mod tests {
    use super::*;

    fn an_input(node: Option<NodeId>, enabled: bool) -> InputConfig {
        InputConfig {
            id: Uuid::nil(),
            name: "Guest console".into(),
            kind: InputKind::Sacn,
            node_id: node,
            interfaces: BTreeMap::new(),
            universes: [(1, 5)].into_iter().collect(),
            enabled,
        }
    }

    /// The rule that differs from an output's, written down as a test because it is
    /// the one somebody will try to "fix": nobody named means nobody listens.
    #[test]
    fn an_input_that_names_no_station_is_listened_to_by_nobody() {
        let here = NodeId(Uuid::from_u128(1));
        let there = NodeId(Uuid::from_u128(2));

        assert!(!an_input(None, true).listens_on(here));
        assert!(!an_input(None, true).listens_on(there));
        assert!(an_input(Some(here), true).listens_on(here));
        assert!(!an_input(Some(here), true).listens_on(there));
        assert!(!an_input(Some(here), false).listens_on(here), "switched off");
    }

    #[test]
    fn the_universe_map_is_a_routing_and_an_unmapped_wire_is_not_carried() {
        let input = an_input(Some(NodeId(Uuid::nil())), true);
        assert_eq!(input.patch_universe(1), Some(5));
        assert_eq!(input.patch_universe(2), None);
        assert_eq!(input.patch_universe(5), None, "the map is one-way");
    }
}
