//! Outputs: where the show goes, as show data rather than as a command line.
//!
//! Until now an output was an `--artnet` flag read once at startup, which meant the
//! one part of the system that actually puts light on stage was the one part an
//! operator could not see or change. An `OutputConfig` is an ordinary PERSISTED
//! entity: it saves with the show, replicates to peers, and can be switched off
//! from the console at half past six.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{events::operation::NodeId, PultSchema};

/// Which protocol an output speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum OutputKind {
    Artnet,
    Sacn,
    /// Adopted OpenHaunt nodes: their ports, and sACN to any DMX gateway among them.
    OpenHaunt,
}

impl OutputKind {
    /// Does this kind send to an address someone has to type in?
    ///
    /// sACN has a multicast group per universe and OpenHaunt knows where its own
    /// nodes are, so both can work with nothing filled in.
    pub fn needs_target(self) -> bool {
        matches!(self, OutputKind::Artnet)
    }
}

/// One configured output.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "outputs")]
pub struct OutputConfig {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub kind: OutputKind,
    /// Where to send, as `host` or `host:port`. Required for Art-Net; for sACN an
    /// address means unicast to a receiver that multicast cannot reach.
    #[pult(lifecycle = PERSISTED)]
    pub target: Option<String>,
    /// Which universes to send. Empty means every universe in the patch.
    #[pult(lifecycle = PERSISTED)]
    pub universes: Vec<u16>,
    #[pult(lifecycle = PERSISTED)]
    pub enabled: bool,
    /// Which station sends this. `None` means every one of them, which puts the same
    /// frames on the wire once per node — useful on purpose for a redundant path,
    /// and a surprise by accident, so the UI fills in the local station.
    #[pult(lifecycle = PERSISTED)]
    pub node_id: Option<NodeId>,
}

impl OutputConfig {
    /// Should this station send this output?
    pub fn runs_on(&self, node_id: NodeId) -> bool {
        self.enabled && self.node_id.map(|owner| owner == node_id).unwrap_or(true)
    }

    /// Does this output carry the given universe?
    pub fn carries(&self, universe: u16) -> bool {
        self.universes.is_empty() || self.universes.contains(&universe)
    }
}

/// What one output has actually been doing.
///
/// LOCAL: it describes this station's sockets, and the station next to it running
/// the same show has its own answer. Reported because a mistyped Art-Net address is
/// otherwise completely silent — which is most of the reason the *Outputs* tab is
/// worth having at all.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputStatus {
    pub name: String,
    pub kind: String,
    /// Running on this station. False for one owned elsewhere, or disabled.
    pub running: bool,
    pub last_send: Option<DateTime<Utc>>,
    pub frames_per_second: f32,
    pub error_count: u64,
    /// What went wrong most recently, if anything has.
    pub last_error: Option<String>,
}

/// Every output's status, keyed by config id: the LOCAL `output_status` path.
pub type OutputStatuses = BTreeMap<String, OutputStatus>;

#[cfg(test)]
mod tests {
    use super::*;

    fn an_output(kind: OutputKind) -> OutputConfig {
        OutputConfig {
            id: Uuid::new_v4(),
            name: "House".into(),
            kind,
            target: None,
            universes: vec![],
            enabled: true,
            node_id: None,
        }
    }

    #[test]
    fn only_art_net_needs_somewhere_to_send_to() {
        assert!(OutputKind::Artnet.needs_target());
        assert!(!OutputKind::Sacn.needs_target(), "sACN has a group per universe");
        assert!(!OutputKind::OpenHaunt.needs_target(), "a node says where it is");
    }

    #[test]
    fn an_output_with_no_station_runs_everywhere() {
        let output = an_output(OutputKind::Sacn);
        assert!(output.runs_on(NodeId::new()));
        assert!(output.runs_on(NodeId::new()));
    }

    #[test]
    fn an_owned_output_runs_only_on_its_own_station() {
        let mine = NodeId::new();
        let theirs = NodeId::new();
        let mut output = an_output(OutputKind::Artnet);
        output.node_id = Some(mine);

        assert!(output.runs_on(mine));
        assert!(!output.runs_on(theirs), "two stations sending is two copies on the wire");
    }

    #[test]
    fn a_disabled_output_runs_nowhere() {
        let mut output = an_output(OutputKind::Artnet);
        output.enabled = false;
        assert!(!output.runs_on(NodeId::new()));

        output.node_id = Some(NodeId::new());
        assert!(!output.runs_on(output.node_id.unwrap()));
    }

    #[test]
    fn an_empty_universe_list_means_all_of_them() {
        let output = an_output(OutputKind::Sacn);
        assert!(output.carries(1));
        assert!(output.carries(512));
    }

    #[test]
    fn a_universe_list_is_a_filter() {
        let mut output = an_output(OutputKind::Sacn);
        output.universes = vec![1, 5];
        assert!(output.carries(1));
        assert!(output.carries(5));
        assert!(!output.carries(2));
    }
}
