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

use crate::{
    events::operation::NodeId,
    types::fixture::{Fixture, FixtureAddress},
    PultSchema,
};

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
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

/// Fixtures that no configured output reaches, and the output that would.
///
/// A fixture is patched somewhere; an output is what carries that somewhere onto
/// a wire. Nothing ties the two together, so a show can have a fixture on
/// universe 3 and no output that sends universe 3, or a node adopted and no
/// output that drives nodes — and the only symptom is a fader that does nothing.
/// A gap names the fixtures and the kind of output (with its universe, for DMX)
/// that would close it, which is enough for a panel to offer the fix as a button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputGap {
    /// The kind of output that would cover these fixtures.
    pub kind: OutputKind,
    /// The universe an sACN or Art-Net output would have to carry. None for
    /// OpenHaunt nodes, which are reached by serial rather than by universe.
    pub universe: Option<u16>,
    pub fixture_ids: Vec<Uuid>,
    pub fixture_names: Vec<String>,
}

/// The LOCAL `output_coverage` path: what the show's outputs leave unreached.
///
/// Computed from show data alone, so every station arrives at the same answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputCoverage {
    pub gaps: Vec<OutputGap>,
}

impl OutputCoverage {
    /// Which fixtures no enabled output reaches.
    ///
    /// A DMX fixture is reached by an enabled Art-Net or sACN output carrying its
    /// universe, or by an adopted gateway node that forwards that universe — which
    /// itself needs an OpenHaunt output. A node fixture is reached by any enabled
    /// OpenHaunt output. Which station an output runs on is not judged here: an
    /// output owned by a station that is switched off is a different problem, and
    /// one the Outputs panel already shows.
    pub fn of(outputs: &[OutputConfig], fixtures: &[Fixture]) -> Self {
        let enabled: Vec<&OutputConfig> = outputs.iter().filter(|o| o.enabled).collect();
        let nodes_driven = enabled.iter().any(|o| o.kind == OutputKind::OpenHaunt);
        let gateway_universes: Vec<u16> = fixtures
            .iter()
            .filter_map(|f| match f.address {
                FixtureAddress::OpenHaunt { universe, .. } => universe,
                FixtureAddress::Dmx { .. } => None,
            })
            .collect();
        let dmx_carried = |universe: u16| {
            enabled.iter().any(|o| {
                matches!(o.kind, OutputKind::Artnet | OutputKind::Sacn) && o.carries(universe)
            }) || (nodes_driven && gateway_universes.contains(&universe))
        };

        // Keyed so that every fixture on one universe lands in one gap, in a
        // stable order: nodes first, then universes ascending.
        let mut gaps: BTreeMap<(u8, u16), OutputGap> = BTreeMap::new();
        for fixture in fixtures {
            let key = match fixture.address {
                FixtureAddress::OpenHaunt { .. } if !nodes_driven => (0, 0),
                FixtureAddress::Dmx { universe, .. } if !dmx_carried(universe) => (1, universe),
                _ => continue,
            };
            let gap = gaps.entry(key).or_insert_with(|| OutputGap {
                kind: if key.0 == 0 { OutputKind::OpenHaunt } else { OutputKind::Sacn },
                universe: (key.0 == 1).then_some(key.1),
                fixture_ids: Vec::new(),
                fixture_names: Vec::new(),
            });
            gap.fixture_ids.push(fixture.id);
            gap.fixture_names.push(fixture.name.clone());
        }
        OutputCoverage { gaps: gaps.into_values().collect() }
    }
}

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

    fn a_fixture(name: &str, address: FixtureAddress) -> Fixture {
        Fixture {
            id: Uuid::new_v4(),
            name: name.into(),
            fixture_type_id: Uuid::new_v4(),
            address,
            position: None,
            live_values: Default::default(),
            active_preset: None,
        }
    }

    fn node(serial: &str, universe: Option<u16>) -> FixtureAddress {
        FixtureAddress::OpenHaunt { serial: serial.into(), universe }
    }

    fn dmx(universe: u16) -> FixtureAddress {
        FixtureAddress::Dmx { universe, address: 1 }
    }

    #[test]
    fn an_adopted_node_with_no_openhaunt_output_is_a_gap() {
        let fixtures = [a_fixture("Strip", node("1a2b3c", None)), a_fixture("Relay", node("4d5e6f", None))];

        let coverage = OutputCoverage::of(&[], &fixtures);
        assert_eq!(coverage.gaps.len(), 1);
        assert_eq!(coverage.gaps[0].kind, OutputKind::OpenHaunt);
        assert_eq!(coverage.gaps[0].universe, None);
        assert_eq!(coverage.gaps[0].fixture_names, vec!["Strip", "Relay"]);

        let driven = OutputCoverage::of(&[an_output(OutputKind::OpenHaunt)], &fixtures);
        assert!(driven.gaps.is_empty(), "one OpenHaunt output reaches every node");

        let mut off = an_output(OutputKind::OpenHaunt);
        off.enabled = false;
        assert_eq!(OutputCoverage::of(&[off], &fixtures).gaps.len(), 1, "a disabled output reaches nothing");
    }

    #[test]
    fn a_dmx_fixture_needs_an_output_carrying_its_universe() {
        let fixtures = [a_fixture("Spot", dmx(1)), a_fixture("Wash", dmx(3)), a_fixture("Blinder", dmx(3))];

        let coverage = OutputCoverage::of(&[], &fixtures);
        assert_eq!(coverage.gaps.len(), 2, "one gap per universe");
        assert_eq!(coverage.gaps[0].universe, Some(1));
        assert_eq!(coverage.gaps[1].universe, Some(3));
        assert_eq!(coverage.gaps[1].fixture_names, vec!["Wash", "Blinder"]);
        assert_eq!(coverage.gaps[1].kind, OutputKind::Sacn, "sACN is what is suggested");

        let everything = an_output(OutputKind::Sacn);
        assert!(OutputCoverage::of(&[everything], &fixtures).gaps.is_empty(), "empty universes means all");

        let mut only_one = an_output(OutputKind::Artnet);
        only_one.universes = vec![1];
        let coverage = OutputCoverage::of(&[only_one], &fixtures);
        assert_eq!(coverage.gaps.len(), 1);
        assert_eq!(coverage.gaps[0].universe, Some(3));
    }

    #[test]
    fn a_gateway_node_carries_its_universe_when_nodes_are_driven() {
        let fixtures = [a_fixture("Gateway", node("e2e-gate", Some(5))), a_fixture("Spot", dmx(5))];

        let none = OutputCoverage::of(&[], &fixtures);
        assert_eq!(none.gaps.len(), 2, "neither the node nor the universe behind it is reached");

        let driven = OutputCoverage::of(&[an_output(OutputKind::OpenHaunt)], &fixtures);
        assert!(driven.gaps.is_empty(), "the gateway forwards universe 5 once nodes are driven");
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
