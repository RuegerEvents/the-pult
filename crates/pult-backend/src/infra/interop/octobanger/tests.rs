use std::collections::BTreeMap;

use pult_octobanger::Controller;
use pult_schema::types::fixture::{
    parameter_key, Fixture, FixtureType, ParameterDefinition, ParameterDirection, ParameterKind, ParameterValue,
};
use pult_schema::types::flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerCondition};
use pult_schema::types::timeline::{AtEnd, AudioDestination, Timeline, TimelineAction};
use pult_schema::types::zone::SpeakerZone;
use uuid::Uuid;

use super::*;

fn corpus(name: &str) -> Controller {
    let path = format!("{}/../../testdata/octobanger/{name}.OBC", env!("CARGO_MANIFEST_DIR"));
    Controller::read(&std::fs::read(path).unwrap()).unwrap()
}

/// A V0-shaped node: four relays and a PIR, as an adopted one types itself.
fn node() -> (FixtureType, Fixture) {
    let mut parameters: Vec<ParameterDefinition> = (0..4)
        .map(|n| ParameterDefinition::on_port(ParameterKind::Switch(n), n, ParameterValue::Bool(false)))
        .collect();
    parameters.push(ParameterDefinition {
        direction: ParameterDirection::Input,
        ..ParameterDefinition::on_port(ParameterKind::Contact(4), 4, ParameterValue::Bool(false))
    });
    let kind = FixtureType { id: Uuid::new_v4(), name: "Node V0".into(), parameters, ..FixtureType::default() };
    let fixture = Fixture { id: Uuid::new_v4(), name: "Area 1".into(), fixture_type_id: kind.id, ..Fixture::default() };
    (kind, fixture)
}

fn zone() -> SpeakerZone {
    SpeakerZone {
        folder: None,
        id: Uuid::new_v4(),
        name: "Area 1".into(),
        color: None,
        width: 1,
        channels: Default::default(),
        nodes: Vec::new(),
        level_db: None,
        notes: String::new(),
    }
}

/// The show as rows, so a plan can be read back the way the engine would hold it.
#[derive(Default)]
struct Show {
    fixtures: Vec<Fixture>,
    fixture_types: Vec<FixtureType>,
    timelines: Vec<Timeline>,
    flows: Vec<Flow>,
    flow_nodes: Vec<FlowNode>,
    flow_edges: Vec<FlowEdge>,
    speaker_zones: Vec<SpeakerZone>,
}

impl Show {
    fn existing(&self) -> Existing<'_> {
        Existing {
            fixtures: &self.fixtures,
            fixture_types: &self.fixture_types,
            timelines: &self.timelines,
            flows: &self.flows,
            flow_nodes: &self.flow_nodes,
            flow_edges: &self.flow_edges,
            speaker_zones: &self.speaker_zones,
        }
    }

    /// Apply a plan's writes, as creates or whole-row replacements.
    fn apply(&mut self, plan: &ImportPlan) {
        fn put<T: serde::de::DeserializeOwned>(rows: &mut Vec<T>, value: &serde_json::Value, id: impl Fn(&T) -> Uuid) {
            let row: T = serde_json::from_value(value.clone()).unwrap();
            rows.retain(|r| id(r) != id(&row));
            rows.push(row);
        }
        for (path, _, value) in &plan.writes {
            let pult_schema::path::PathSegment::Key(table) = &path[0] else { panic!() };
            match table.as_str() {
                "fixtures" => put(&mut self.fixtures, value, |r| r.id),
                "fixture_types" => put(&mut self.fixture_types, value, |r| r.id),
                "timelines" => put(&mut self.timelines, value, |r| r.id),
                "flows" => put(&mut self.flows, value, |r| r.id),
                "flow_nodes" => put(&mut self.flow_nodes, value, |r| r.id),
                "flow_edges" => put(&mut self.flow_edges, value, |r| r.id),
                other => panic!("an import wrote {other}"),
            }
        }
    }

    fn timeline(&self, name: &str) -> &Timeline {
        self.timelines.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no timeline {name}"))
    }
}

fn sound(seconds: u64) -> Sound {
    Sound { sha: format!("{seconds:064}"), bytes: Some(vec![seconds as u8]), duration_ms: seconds * 1_000 }
}

fn recording(plan: &ImportPlan) -> pult_render::track::Track {
    let (_, bytes) = plan.assets.iter().find(|(mime, _)| mime == assets::TRACK_MIME).expect("a recording");
    pult_render::track::decode(bytes).unwrap()
}

#[test]
fn unmapped_channels_go_to_placeholders_and_nothing_is_lost() {
    let donkey = corpus("Donkey");
    let mut show = Show::default();
    let plan = plan_import(&donkey, &Mapping::default(), &BTreeMap::new(), &show.existing()).unwrap();
    show.apply(&plan);

    assert_eq!(show.fixture_types.len(), 1, "one placeholder type");
    assert!(
        show.fixture_types[0].mode("Default").channels.iter().all(|c| c.offsets.is_empty()),
        "which reaches no wire: a switch on a port has no byte"
    );
    let names: Vec<&str> = show.fixtures.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Donkey · 1 - Drop", "Donkey · 2 - Kopfbewegung"], "the two used channels, not the strobe");

    let scare = show.timeline("Donkey");
    assert_eq!(scare.tracks.len(), 2);
    assert_eq!(scare.end.unwrap().at_ms, 5049 + 15_000, "on through the reset delay");
    assert_eq!(scare.end.unwrap().then, AtEnd::Stop);
    assert!(scare.clips.is_empty());
    assert!(plan.report.warnings.iter().any(|w| w.contains("nowhere was chosen")));

    let drop = &recording(&plan).keys[0];
    let points: Vec<(u32, &ParameterValue)> = drop.points.iter().map(|p| (p.ms, &p.value)).collect();
    assert_eq!(points, [(0, &ParameterValue::Bool(true)), (4950, &ParameterValue::Bool(false))]);

    assert_eq!(show.flow_nodes.len(), 2, "with no trigger, a button and the play");
    assert!(show.flow_nodes.iter().any(|n| n.kind == FlowNodeKind::Button && n.name.as_deref() == Some("Trigger Donkey")));
}

#[test]
fn a_mapped_prop_is_a_scare_an_ambience_and_a_flow() {
    let donkey = corpus("Donkey");
    let (kind, area) = node();
    let speakers = zone();
    let mut show = Show {
        fixtures: vec![area.clone()],
        fixture_types: vec![kind],
        speaker_zones: vec![speakers.clone()],
        ..Show::default()
    };
    let target = |channel: u8, relay: u8| ChannelTarget {
        channel,
        fixture_id: Some(area.id),
        key: Some(parameter_key(&ParameterKind::Switch(relay))),
    };
    let mapping = Mapping {
        channels: vec![target(1, 0), target(2, 1)],
        trigger: Some(Target { fixture_id: area.id, key: parameter_key(&ParameterKind::Contact(4)) }),
        destination: Some(AudioDestination::Zone { zone: speakers.id }),
        ambience: true,
        ..Mapping::default()
    };
    let sounds = BTreeMap::from([("Donkey_Trigger.mp3".to_string(), sound(6)), ("Heinz_Ambient.mp3".to_string(), sound(60))]);
    let plan = plan_import(&donkey, &mapping, &sounds, &show.existing()).unwrap();
    show.apply(&plan);

    assert!(show.fixture_types.iter().all(|t| t.name != "Unassigned OctoBanger output"), "nothing needed a placeholder");
    assert_eq!(plan.assets.iter().filter(|(mime, _)| mime == "audio/mpeg").count(), 2, "both sounds stored");

    let scare = show.timeline("Donkey");
    assert_eq!(scare.clips[0].length_ms, Some(5049), "cut where the sequence ends");
    let ambience = show.timeline("Donkey ambience");
    assert_eq!(ambience.end.unwrap().then, AtEnd::Loop);
    assert_eq!(ambience.end.unwrap().at_ms, 60_000, "goes round at the end of its file");
    let events: Vec<(u32, &TimelineAction)> = scare.events.iter().map(|e| (e.at_ms, &e.action)).collect();
    assert!(matches!(events[0], (0, TimelineAction::StopTimeline { timeline_id }) if *timeline_id == ambience.id));
    assert!(matches!(events[1], (5049, TimelineAction::PlayTimeline { timeline_id, .. }) if *timeline_id == ambience.id));

    let keys: Vec<(Uuid, String)> = recording(&plan).keys.iter().map(|k| (k.fixture_id, k.key.clone())).collect();
    assert_eq!(keys, [(area.id, "Switch:0".into()), (area.id, "Switch:1".into())]);

    assert_eq!(show.flow_nodes.len(), 4);
    assert_eq!(show.flow_edges.len(), 3);
    assert!(
        show.flow_nodes.iter().any(|n| n.kind == FlowNodeKind::Condition(TriggerCondition::RisingEdge)),
        "Donkey rests low, so a PIR taking it high is the trigger"
    );
}

#[test]
fn a_contact_to_ground_triggers_on_the_falling_edge_unless_told_otherwise() {
    let cabinet = corpus("Schaltschrank");
    let (kind, area) = node();
    let trigger = Some(Target { fixture_id: area.id, key: parameter_key(&ParameterKind::Contact(4)) });
    let show = Show { fixtures: vec![area.clone()], fixture_types: vec![kind], ..Show::default() };
    let condition = |mapping: &Mapping| {
        let mut show = Show { fixtures: show.fixtures.clone(), fixture_types: show.fixture_types.clone(), ..Show::default() };
        let plan = plan_import(&cabinet, mapping, &BTreeMap::new(), &show.existing()).unwrap();
        show.apply(&plan);
        show.flow_nodes.into_iter().find_map(|n| match n.kind {
            FlowNodeKind::Condition(c) => Some(c),
            _ => None,
        })
    };
    assert_eq!(condition(&Mapping { trigger: trigger.clone(), ..Mapping::default() }), Some(TriggerCondition::FallingEdge));
    assert_eq!(
        condition(&Mapping { trigger, edge: Some(TriggerCondition::RisingEdge), ..Mapping::default() }),
        Some(TriggerCondition::RisingEdge),
        "a sensor wired the other way on the node"
    );
}

#[test]
fn eight_channels_over_two_nodes() {
    let cabinet = corpus("Schaltschrank");
    let (kind, first) = node();
    let second = Fixture { id: Uuid::new_v4(), name: "Area 1b".into(), ..first.clone() };
    let mut show = Show { fixtures: vec![first.clone(), second.clone()], fixture_types: vec![kind], ..Show::default() };
    let channels = (1..=8u8)
        .map(|channel| ChannelTarget {
            channel,
            fixture_id: Some(if channel <= 4 { first.id } else { second.id }),
            key: Some(parameter_key(&ParameterKind::Switch((channel - 1) % 4))),
        })
        .collect();
    let plan = plan_import(&cabinet, &Mapping { channels, ..Mapping::default() }, &BTreeMap::new(), &show.existing()).unwrap();
    show.apply(&plan);
    assert_eq!(show.timeline("Schaltschrank").tracks.len(), 8);
    assert_eq!(show.fixtures.len(), 2, "no placeholders");
    let beacon = &recording(&plan).keys[2];
    assert_eq!(beacon.points.last().map(|p| p.ms), Some(17_090), "off at the end, not past it");
}

#[test]
fn a_mapping_that_cannot_be_is_refused_before_anything_is_written() {
    let donkey = corpus("Donkey");
    let (kind, area) = node();
    let show = Show { fixtures: vec![area.clone()], fixture_types: vec![kind], ..Show::default() };
    let to = |channel: u8, key: &str| ChannelTarget { channel, fixture_id: Some(area.id), key: Some(key.into()) };
    let refused = |channels: Vec<ChannelTarget>| {
        plan_import(&donkey, &Mapping { channels, ..Mapping::default() }, &BTreeMap::new(), &show.existing()).err().unwrap()
    };
    assert!(refused(vec![to(1, "Switch:0"), to(2, "Switch:0")]).contains("both go to"));
    assert!(refused(vec![to(1, "Contact:4")]).contains("not an output"));
    assert!(refused(vec![to(1, "Switch:9")]).contains("has no"));
    assert!(refused(vec![ChannelTarget { channel: 1, fixture_id: Some(Uuid::new_v4()), key: Some("Switch:0".into()) }])
        .contains("does not have"));

    let not_an_input = Mapping { trigger: Some(Target { fixture_id: area.id, key: "Switch:0".into() }), ..Mapping::default() };
    assert!(plan_import(&donkey, &not_an_input, &BTreeMap::new(), &show.existing()).err().unwrap().contains("not an input"));
}

#[test]
fn importing_again_rewrites_what_the_last_import_wrote() {
    let donkey = corpus("Donkey");
    let (kind, area) = node();
    let mut show = Show { fixtures: vec![area.clone()], fixture_types: vec![kind], ..Show::default() };
    let first = plan_import(&donkey, &Mapping::default(), &BTreeMap::new(), &show.existing()).unwrap();
    show.apply(&first);
    let rows = show.fixtures.len() + show.timelines.len() + show.flow_nodes.len();

    // The drop has moved onto the node since; the head is still unassigned.
    let mapping = Mapping {
        channels: vec![ChannelTarget { channel: 1, fixture_id: Some(area.id), key: Some("Switch:0".into()) }],
        ..Mapping::default()
    };
    let second = plan_import(&donkey, &mapping, &BTreeMap::new(), &show.existing()).unwrap();
    assert_eq!(second.report.created, 0, "nothing doubled: {:?}", second.report);
    show.apply(&second);
    assert_eq!(show.fixtures.len() + show.timelines.len() + show.flow_nodes.len(), rows);
    assert_eq!(second.report.missing, ["Donkey · 1 - Drop is no longer used by this prop"], "listed, never deleted");
    let keys: Vec<Uuid> = recording(&second).keys.iter().map(|k| k.fixture_id).collect();
    assert_eq!(keys[0], area.id);
}

#[test]
fn a_recording_on_the_same_output_elsewhere_is_warned_about() {
    let donkey = corpus("Donkey");
    let whips = corpus("Air Whips");
    let (kind, area) = node();
    let mut show = Show { fixtures: vec![area.clone()], fixture_types: vec![kind], ..Show::default() };
    let on_relay_0 = Mapping {
        channels: vec![ChannelTarget { channel: 1, fixture_id: Some(area.id), key: Some("Switch:0".into()) }],
        ..Mapping::default()
    };
    show.apply(&plan_import(&whips, &on_relay_0, &BTreeMap::new(), &show.existing()).unwrap());
    let plan = plan_import(&donkey, &on_relay_0, &BTreeMap::new(), &show.existing()).unwrap();
    assert!(plan.report.warnings.iter().any(|w| w.contains("also recorded on Air Whips")), "{:?}", plan.report.warnings);
}

#[test]
fn every_prop_in_the_corpus_plans() {
    for name in ["Donkey", "Sample Project using Shield", "Air Whips", "Aussenszenerie", "Kotzender", "Schaltschrank"] {
        let controller = corpus(name);
        let mut show = Show::default();
        let plan = plan_import(&controller, &Mapping::default(), &BTreeMap::new(), &show.existing()).unwrap();
        show.apply(&plan);
        let used = (1..=8).filter(|&c| controller.used(c)).count();
        assert_eq!(show.timeline(name).tracks.len(), used, "{name}");
    }
}

#[test]
fn what_the_arduino_needed_is_said_and_not_carried() {
    let notes = notes(&corpus("Kotzender"));
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].starts_with("1 - Bauch was Active Low"));
    assert!(super::notes(&corpus("Donkey")).is_empty());
}
