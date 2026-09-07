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
    types::network::SacnPriority,
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
    ///
    /// A routing rather than a label: the connector renders only these, so two
    /// outputs can split a rig between two interfaces and each evaluates its own
    /// half. Obeyed by every kind, an OpenHaunt output included — the sACN it feeds
    /// its gateway nodes is a universe on a wire like any other.
    #[pult(lifecycle = PERSISTED)]
    pub universes: Vec<u16>,
    #[pult(lifecycle = PERSISTED)]
    pub enabled: bool,
    /// Which station sends this. `None` means *whichever station may*, and what that
    /// comes to depends on the kind — see [`OutputConfig::runs_on`]. The UI fills in
    /// the local station, so the explicit case stays the normal one.
    #[pult(lifecycle = PERSISTED)]
    pub node_id: Option<NodeId>,
    /// Which interface this goes out on, per station.
    ///
    /// A map rather than one string because an output row replicates and an
    /// interface name means a different cable on every machine: `en5` on the booth
    /// console and `eth1` on the stage rack are not the same thing, and one field
    /// would be wrong on all but one of them.
    ///
    /// A station this map does not name has been told nothing, and falls through to
    /// its own `[network]` preference and then to every interface — which is what
    /// this console did before any of this existed, and is why no existing show has
    /// to be edited. A station it *does* name, with an interface that machine has
    /// not got, refuses to send rather than quietly binding everything.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub interfaces: BTreeMap<NodeId, String>,
    /// What priority byte an sACN output puts in its packets. Ignored by the other
    /// kinds, which have no such field in their protocols.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub priority: SacnPriority,
}

impl OutputConfig {
    /// Should this station send this output?
    ///
    /// A row that names a station is that station's, whatever the kind. What `None`
    /// means is decided by whether the protocol can survive two senders.
    ///
    /// **Art-Net and OpenHaunt cannot.** Art-Net has no priority mechanism at all,
    /// so two consoles sending one universe is a rig that flickers with nothing on
    /// any screen to explain it. `None` therefore resolves to *the leader*, which is
    /// deliberately not a refusal: `None` is what every existing show has and is
    /// completely harmless on a single console — a lone station is the leader, so
    /// nothing about a one-console rig changes — and a second station joining
    /// silently stops double-sending instead of starting to fight.
    ///
    /// **sACN can**, because E1.31 has a priority byte and receivers arbitrate on
    /// it. So an unowned sACN output runs everywhere, and [`SacnPriority`] is what
    /// makes that well-defined rather than a race.
    pub fn runs_on(&self, node_id: NodeId, is_leader: bool) -> bool {
        if !self.enabled {
            return false;
        }
        match self.node_id {
            Some(owner) => owner == node_id,
            None => match self.kind {
                OutputKind::Sacn => true,
                OutputKind::Artnet | OutputKind::OpenHaunt => is_leader,
            },
        }
    }

    /// What this row says about which cable this station should use, if anything.
    pub fn interface_for(&self, node_id: NodeId) -> Option<&str> {
        self.interfaces.get(&node_id).map(String::as_str)
    }

    /// Does this output carry the given universe?
    pub fn carries(&self, universe: u16) -> bool {
        carries(&self.universes, universe)
    }
}

/// Does a universe list carry this universe? Empty means every one of them.
///
/// A free function as well as a method because the thing that has to *obey* the
/// filter is a connector, and a connector is handed a wire and a list rather than a
/// whole `OutputConfig` — it has no id, no name and no station to care about. One
/// predicate rather than two, because the panel's coverage warnings and the socket
/// disagreeing about which universes an output carries is precisely the defect this
/// filter existed with for as long as nobody read it.
pub fn carries(universes: &[u16], universe: u16) -> bool {
    universes.is_empty() || universes.contains(&universe)
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
        // Every kind here is asked the same question in the same order — does an
        // enabled output *carrying this universe* reach it — because the connectors
        // now obey `universes` and a coverage answer that judged one kind by the
        // filter and another by its existence would go back to describing a routing
        // nobody implements.
        let dmx_carried = |universe: u16| {
            enabled.iter().any(|o| {
                o.carries(universe)
                    && match o.kind {
                        OutputKind::Artnet | OutputKind::Sacn => true,
                        // A gateway node forwards the universe it was adopted on, so
                        // an OpenHaunt output reaches a universe only where one of
                        // them is listening for it.
                        OutputKind::OpenHaunt => gateway_universes.contains(&universe),
                    }
            })
        };

        // Keyed so that every fixture on one universe lands in one gap, in a
        // stable order: nodes first, then universes ascending.
        //
        // Per *break* rather than per fixture, because a fixture with a separate
        // dimmer break sits in two universes and one of them can be carried while the
        // other is not — a gap a per-fixture answer cannot say.
        let mut gaps: BTreeMap<(u8, u16), OutputGap> = BTreeMap::new();
        for fixture in fixtures {
            let mut keys: Vec<(u8, u16)> = Vec::new();
            match &fixture.address {
                FixtureAddress::OpenHaunt { .. } if !nodes_driven => keys.push((0, 0)),
                FixtureAddress::Dmx { breaks, .. } => {
                    for entry in breaks {
                        if !dmx_carried(entry.universe) && !keys.contains(&(1, entry.universe)) {
                            keys.push((1, entry.universe));
                        }
                    }
                }
                _ => {}
            }
            for key in keys {
                let gap = gaps.entry(key).or_insert_with(|| OutputGap {
                    kind: if key.0 == 0 { OutputKind::OpenHaunt } else { OutputKind::Sacn },
                    universe: (key.0 == 1).then_some(key.1),
                    fixture_ids: Vec::new(),
                    fixture_names: Vec::new(),
                });
                gap.fixture_ids.push(fixture.id);
                gap.fixture_names.push(fixture.name.clone());
            }
        }
        OutputCoverage { gaps: gaps.into_values().collect() }
    }
}


// ── What is actually on the wire ──────────────────────────────────────────────

/// What one connector says it is putting on a wire, for somebody watching.
///
/// A view is **asked for, never published**. A universe image is 512 bytes forty
/// times a second, and a station that broadcast that continuously — to its browsers
/// or, worse, across the sync link — would be putting a stream nobody is reading on
/// the network that is carrying the show. So this exists only while a viewer is
/// open on this output, is drawn at the panel's rate rather than the wire's, and is
/// not sent again when it has not changed.
///
/// It carries where it came from, because it travels alone: one push is one
/// connector's answer, and a panel watching two stations' outputs at once files each
/// by `(node_id, output_id)` without being told separately what it is looking at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputView {
    pub node_id: NodeId,
    pub output_id: Uuid,
    /// What part of this connector's traffic was asked for, in the connector's own
    /// terms — a universe number, a node's serial, whatever it named in its own
    /// sections. Opaque here on purpose: the seam has to carry a connector nobody
    /// has written yet, and a field per protocol is exactly what that forbids.
    pub focus: Option<String>,
    /// Console milliseconds when the connector was asked.
    pub at_ms: u64,
    /// What this connector's traffic is made of, in the order a panel should stack
    /// it. Several, because one connector is not always one shape of thing: an
    /// OpenHaunt output tells nodes about their ports *and* feeds sACN to the
    /// gateways among them, and a viewer that could show only one of those would be
    /// lying about half of what left the station.
    pub sections: Vec<OutputSection>,
}

/// One part of a connector's traffic, named and shaped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputSection {
    /// What to call it in the panel. The connector's words, since it is the only
    /// thing that knows what it is doing.
    pub title: String,
    /// A sentence under the title where one is worth having, for the thing a
    /// connector knows and a viewer cannot infer — that per-port commands do not
    /// travel inside a frame, say.
    pub note: Option<String>,
    pub body: SectionBody,
}

/// The shapes a viewer knows how to draw.
///
/// Tagged by **shape rather than by protocol**, which is the whole of what makes a
/// new output cheap: one that carries whole universes gets the DMX sheet for
/// nothing, and one that says discrete things gets the message list. A connector
/// whose traffic looks like neither adds a variant here and a component beside the
/// others in the frontend's registry, and touches no panel — and until it does, a
/// shape an older console has never heard of is drawn as itself rather than
/// silently missing, the same rule a layout follows for a panel id it does not know.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "shape", content = "of", rename_all = "camelCase")]
#[ts(export)]
pub enum SectionBody {
    Universes(UniverseTraffic),
    Messages(MessageTraffic),
}

/// Whole universes of channel data: Art-Net, sACN, and the sACN a gateway is fed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UniverseTraffic {
    /// Every universe this connector carries, whether or not it is being looked at.
    /// Cheap, and it is what the viewer offers to look at next.
    pub universes: Vec<UniverseSummary>,
    /// The 512 bytes of the one being looked at. Only one, because the sheet shows
    /// one and forty of them at panel rate is a megabyte a second for a picture
    /// nobody can read.
    pub focused: Option<UniverseFrame>,
}

/// A universe, without its bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UniverseSummary {
    pub universe: u16,
    /// How many of the 512 are not at zero. The one figure that says at a glance
    /// whether a universe is carrying a rig or carrying nothing.
    pub live_channels: u16,
    /// Since this universe's image last actually *changed*, which is not the same as
    /// since it was last sent: the DMX family re-sends a settled universe on its
    /// keep-alive, and a viewer that showed only the send would report every idle
    /// universe as busy.
    pub changed_ms_ago: u32,
    pub sent_ms_ago: u32,
}

/// One universe as it went out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UniverseFrame {
    pub universe: u16,
    /// 512 channels, one byte each, channel 1 first.
    pub channels: Vec<u8>,
}

/// Discrete things said, rather than a picture of a state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageTraffic {
    /// Oldest first, and **drained**: what a connector has said since the viewer last
    /// looked. The panel keeps the history, because the connector's ring is bounded
    /// by what it can afford and the reader's by what it can read.
    pub messages: Vec<OutputMessage>,
    /// How many were thrown away because the ring filled between two looks. Said
    /// rather than swallowed, for the reason the log counts a gap: a silent hole in
    /// a diagnostic is worse than a visible one.
    pub dropped: u64,
}

/// One thing a connector said to something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputMessage {
    pub at_ms: u64,
    /// Who it went to: a node's serial, an address, whatever names the far end.
    pub to: String,
    /// What kind of thing it was, in a word or two — the column a reader scans.
    pub what: String,
    /// The payload, as text. The connector decides how much of it is worth showing.
    pub detail: String,
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
            interfaces: BTreeMap::new(),
            priority: SacnPriority::default(),
        }
    }

    fn a_fixture(name: &str, address: FixtureAddress) -> Fixture {
        Fixture {
            id: Uuid::new_v4(),
            name: name.into(),
            fixture_type_id: Uuid::new_v4(),
            address,
            position: None,
            sensed_values: Default::default(),
            live_effects: Default::default(),
            live_fades: Default::default(),
            home_values: Default::default(),
            ..Fixture::default()
        }
    }

    fn node(serial: &str, universe: Option<u16>) -> FixtureAddress {
        FixtureAddress::OpenHaunt { serial: serial.into(), universe }
    }

    fn dmx(universe: u16) -> FixtureAddress {
        FixtureAddress::dmx(universe, 1)
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
    fn a_restricted_openhaunt_output_reaches_only_the_gateways_it_carries() {
        // The gateway half of an OpenHaunt output is sACN like any other, so it obeys
        // the same field — and coverage has to be told the same thing the connector
        // is, or the panel goes back to describing a routing nobody implements.
        let fixtures = [
            a_fixture("Gateway A", node("gate-a", Some(1))),
            a_fixture("Gateway B", node("gate-b", Some(2))),
            a_fixture("Spot", dmx(1)),
            a_fixture("Wash", dmx(2)),
        ];

        let mut only_one = an_output(OutputKind::OpenHaunt);
        only_one.universes = vec![1];
        let coverage = OutputCoverage::of(&[only_one], &fixtures);

        assert_eq!(coverage.gaps.len(), 1, "the nodes are driven; universe 2 is not carried");
        assert_eq!(coverage.gaps[0].universe, Some(2));
        assert_eq!(coverage.gaps[0].fixture_names, vec!["Wash"]);
    }

    #[test]
    fn two_outputs_can_split_a_rig_between_them() {
        let fixtures = [a_fixture("Spot", dmx(1)), a_fixture("Wash", dmx(5))];
        let mut downstage = an_output(OutputKind::Artnet);
        downstage.universes = vec![1, 2, 3, 4];
        let mut upstage = an_output(OutputKind::Artnet);
        upstage.universes = vec![5, 6, 7, 8];

        assert!(
            OutputCoverage::of(&[downstage.clone(), upstage], &fixtures).gaps.is_empty(),
            "between them they carry everything"
        );
        assert_eq!(
            OutputCoverage::of(&[downstage], &fixtures).gaps.len(),
            1,
            "and one of them alone leaves the other half unreached"
        );
    }

    #[test]
    fn only_art_net_needs_somewhere_to_send_to() {
        assert!(OutputKind::Artnet.needs_target());
        assert!(!OutputKind::Sacn.needs_target(), "sACN has a group per universe");
        assert!(!OutputKind::OpenHaunt.needs_target(), "a node says where it is");
    }

    #[test]
    fn an_unowned_sacn_output_runs_everywhere() {
        // E1.31 arbitrates on the priority byte, so several stations sending one
        // universe is defined rather than a race.
        let output = an_output(OutputKind::Sacn);
        assert!(output.runs_on(NodeId::new(), false));
        assert!(output.runs_on(NodeId::new(), true));
    }

    #[test]
    fn an_unowned_art_net_output_runs_on_the_leader_only() {
        // Art-Net has no priority mechanism, so two consoles on one universe is a
        // rig that flickers. `None` is what every existing show has, so this is
        // resolved rather than refused: a lone console is the leader and nothing
        // about a one-station rig changes, and a second station joining stops
        // double-sending instead of starting to fight.
        for kind in [OutputKind::Artnet, OutputKind::OpenHaunt] {
            let output = an_output(kind);
            assert!(output.runs_on(NodeId::new(), true));
            assert!(!output.runs_on(NodeId::new(), false));
        }
    }

    #[test]
    fn a_named_station_owns_it_whether_or_not_it_leads() {
        let mine = NodeId::new();
        let mut output = an_output(OutputKind::Artnet);
        output.node_id = Some(mine);
        assert!(output.runs_on(mine, false), "a follower still sends what is its own");
    }

    #[test]
    fn two_stations_on_one_art_net_universe_are_reported_and_not_refused() {
        // The split this makes possible — stage left out one cable, stage right out
        // another — is indistinguishable from a collision by anything here, so it
        // is said rather than blocked.
        let a = NodeId::new();
        let b = NodeId::new();
        let mut left = an_output(OutputKind::Artnet);
        left.name = "Stage left".into();
        left.node_id = Some(a);
        let mut right = an_output(OutputKind::Artnet);
        right.name = "Stage right".into();
        right.node_id = Some(b);

        let found = contentions(&[left.clone(), right.clone()], &[1, 2]);
        assert_eq!(found.len(), 2, "an empty universe list is every universe");
        assert_eq!(found[0].universe, 1);
        assert_eq!(found[0].outputs.len(), 2);

        // One station's two outputs are not a contention: it is one sender.
        right.node_id = Some(a);
        assert!(contentions(&[left.clone(), right.clone()], &[1]).is_empty());

        // Nor is an unowned row, which resolves to the leader and so is one sender.
        right.node_id = None;
        assert!(contentions(&[left.clone(), right], &[1]).is_empty());
    }

    #[test]
    fn sacn_never_contends() {
        let mut left = an_output(OutputKind::Sacn);
        left.node_id = Some(NodeId::new());
        let mut right = an_output(OutputKind::Sacn);
        right.node_id = Some(NodeId::new());
        assert!(contentions(&[left, right], &[1]).is_empty());
    }

    #[test]
    fn a_row_names_a_cable_per_station() {
        let mine = NodeId::new();
        let mut output = an_output(OutputKind::Artnet);
        output.interfaces.insert(mine, "en5".into());
        assert_eq!(output.interface_for(mine), Some("en5"));
        // A station the map does not name has been told nothing, which is not the
        // same as being told something wrong.
        assert_eq!(output.interface_for(NodeId::new()), None);
    }

    #[test]
    fn an_owned_output_runs_only_on_its_own_station() {
        let mine = NodeId::new();
        let theirs = NodeId::new();
        let mut output = an_output(OutputKind::Artnet);
        output.node_id = Some(mine);

        assert!(output.runs_on(mine, false));
        assert!(!output.runs_on(theirs, true), "two stations sending is two copies on the wire");
    }

    #[test]
    fn a_disabled_output_runs_nowhere() {
        let mut output = an_output(OutputKind::Artnet);
        output.enabled = false;
        assert!(!output.runs_on(NodeId::new(), true));

        output.node_id = Some(NodeId::new());
        assert!(!output.runs_on(output.node_id.unwrap(), true));
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

/// Two stations putting the same universe on a wire.
///
/// **A warning and never a refusal**, which is the honest position: once an output
/// names its own interface, station A sending universe 1 out `en5` to the stage-left
/// rack and station B sending it out `en6` to stage-right is a legitimate split, and
/// it is indistinguishable from two consoles shouting at one rack by anything this
/// code can see — they differ only in whether the interfaces reach the same
/// broadcast domain, which is a fact about a building. Refusing would make the split
/// unbuildable; saying nothing would leave the collision silent. So it says so.
///
/// sACN is left out because it is the one kind where this is defined rather than a
/// race: E1.31 receivers arbitrate on the priority byte, which is the whole reason
/// [`crate::types::network::SacnPriority`] exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputContention {
    pub universe: u16,
    pub kind: OutputKind,
    /// The outputs that carry it, by name, in id order.
    pub outputs: Vec<String>,
}

/// Every universe two differently-owned outputs of a kind that cannot share one
/// would both send.
///
/// Only rows that name *different* stations contend: an unowned Art-Net row resolves
/// to the leader, so it is one sender by construction and never fights anybody.
pub fn contentions(outputs: &[OutputConfig], universes: &[u16]) -> Vec<OutputContention> {
    let mut found = Vec::new();
    for kind in [OutputKind::Artnet, OutputKind::OpenHaunt] {
        let owned: Vec<&OutputConfig> = outputs
            .iter()
            .filter(|o| o.enabled && o.kind == kind && o.node_id.is_some())
            .collect();
        for &universe in universes {
            let sending: Vec<&&OutputConfig> =
                owned.iter().filter(|o| o.carries(universe)).collect();
            let stations: std::collections::BTreeSet<NodeId> =
                sending.iter().filter_map(|o| o.node_id).collect();
            if stations.len() > 1 {
                let mut outputs: Vec<&&OutputConfig> = sending;
                outputs.sort_by_key(|o| o.id);
                found.push(OutputContention {
                    universe,
                    kind,
                    outputs: outputs.iter().map(|o| o.name.clone()).collect(),
                });
            }
        }
    }
    found
}
