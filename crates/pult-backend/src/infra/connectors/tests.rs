//! Output manager tests.
//!
//! What the manager owns is which plugins exist and whether they are being fed —
//! not what any one of them puts on a wire, which is each connector's own tests.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use pult_schema::types::{
    fixture::{
        Fixture, FixtureAddress, FixtureType, ParameterBinding, ParameterDefinition,
        ParameterDirection, ParameterKind, ParameterValue,
    },
    output::{OutputConfig, OutputKind},
};

use super::*;
use crate::{engine::ShowEngine, infra::showfile};

// ── Harness ───────────────────────────────────────────────────────────────────

async fn an_engine() -> EngineHandle {
    let pool = std::sync::Arc::new(showfile::open_in_memory().await.unwrap());
    let (engine, handle, _broadcast) = ShowEngine::new(NodeId::new(), pool, None);
    tokio::spawn(engine.run());
    handle
}

fn an_output(kind: OutputKind, target: Option<&str>) -> OutputConfig {
    OutputConfig {
        id: Uuid::new_v4(),
        name: "House".into(),
        kind,
        target: target.map(str::to_string),
        universes: vec![],
        enabled: true,
        node_id: None,
    }
}

/// A plugin that counts calls and can be told to fail, standing in for a second
/// protocol without needing one implemented.
struct Recorder {
    calls: Arc<AtomicUsize>,
    fails: bool,
}

impl OutputPlugin for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }

    fn send<'a>(&'a mut self, _patch: &'a Patch, _changed: &'a [Uuid]) -> SendFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fails {
                anyhow::bail!("this output is unplugged");
            }
            Ok(())
        })
    }
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn count(c: &Arc<AtomicUsize>) -> usize {
    c.load(Ordering::SeqCst)
}

fn a_dimmer_patch(level: f32) -> Patch {
    let fixture_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition {
            kind: ParameterKind::Intensity,
            direction: ParameterDirection::Output,
            binding: ParameterBinding::Dmx { channel: 1 },
            default_value: ParameterValue::Float(0.0),
        }],
    };
    let mut fixture = Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::Dmx { universe: 3, address: 1 },
        position: None,
        live_values: Default::default(),
        live_effects: Default::default(),
        live_fades: Default::default(),
    };
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(level));
    Patch {
        fixtures: vec![fixture],
        fixture_types: [(fixture_type.id, fixture_type)].into_iter().collect(),
    }
}

async fn push_a_patch(handle: &OutputHandle, patch: Patch) {
    handle.push(
        patch.fixtures.clone(),
        patch.fixture_types.values().cloned().collect(),
        vec![],
    );
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
}

async fn a_receiver() -> (tokio::net::UdpSocket, std::net::SocketAddr) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    (socket, addr)
}

async fn recv(socket: &tokio::net::UdpSocket) -> Vec<u8> {
    let mut buffer = vec![0u8; 2048];
    let n = tokio::time::timeout(std::time::Duration::from_secs(1), socket.recv(&mut buffer))
        .await
        .expect("a packet within a second")
        .unwrap();
    buffer.truncate(n);
    buffer
}

async fn coverage(engine: &EngineHandle) -> OutputCoverage {
    let value = engine
        .get(vec![PathSegment::Key("output_coverage".into())])
        .await
        .expect("output_coverage is a LOCAL path and always answers");
    serde_json::from_value(value).unwrap()
}

async fn statuses(engine: &EngineHandle) -> OutputStatuses {
    let value = engine
        .get(vec![PathSegment::Key("output_status".into())])
        .await
        .expect("output_status is a LOCAL path and always answers");
    serde_json::from_value(value).unwrap()
}

// ── Feeding the plugins ───────────────────────────────────────────────────────

#[tokio::test]
async fn every_plugin_receives_the_patch() {
    let first = counter();
    let second = counter();
    let (mut manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    manager.preload(an_output(OutputKind::Artnet, None), Box::new(Recorder { calls: first.clone(), fails: false }));
    manager.preload(an_output(OutputKind::Sacn, None), Box::new(Recorder { calls: second.clone(), fails: false }));
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(count(&first), 1);
    assert_eq!(count(&second), 1);
}

#[tokio::test]
async fn one_failing_plugin_does_not_silence_the_others() {
    let broken = counter();
    let working = counter();
    let (mut manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    manager.preload(an_output(OutputKind::Artnet, None), Box::new(Recorder { calls: broken.clone(), fails: true }));
    manager.preload(an_output(OutputKind::Sacn, None), Box::new(Recorder { calls: working.clone(), fails: false }));
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    push_a_patch(&handle, a_dimmer_patch(0.5)).await;

    assert_eq!(count(&broken), 2, "a failing plugin is still asked next time");
    assert_eq!(
        count(&working), 2,
        "an unplugged interface must not stop the output that is working",
    );
}

// ── Reconciling against the configuration ─────────────────────────────────────

#[tokio::test]
async fn a_configured_output_starts_sending() {
    let (receiver, addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());

    handle.configure(vec![an_output(OutputKind::Artnet, Some(&addr.to_string()))]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(recv(&receiver).await[18], 255);
}

#[tokio::test]
async fn removing_an_output_stops_it() {
    let (receiver, addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());
    let output = an_output(OutputKind::Artnet, Some(&addr.to_string()));

    handle.configure(vec![output.clone()]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    let _ = recv(&receiver).await;

    handle.configure(vec![]);
    push_a_patch(&handle, a_dimmer_patch(0.5)).await;

    let anything = tokio::time::timeout(std::time::Duration::from_millis(150), recv(&receiver)).await;
    assert!(anything.is_err(), "an output that was deleted must go quiet");
}

#[tokio::test]
async fn disabling_an_output_stops_it_without_deleting_it() {
    let (receiver, addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());
    let mut output = an_output(OutputKind::Artnet, Some(&addr.to_string()));

    handle.configure(vec![output.clone()]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    let _ = recv(&receiver).await;

    output.enabled = false;
    handle.configure(vec![output]);
    push_a_patch(&handle, a_dimmer_patch(0.5)).await;

    let anything = tokio::time::timeout(std::time::Duration::from_millis(150), recv(&receiver)).await;
    assert!(anything.is_err());
}

#[tokio::test]
async fn an_output_owned_by_another_station_does_not_run_here() {
    let (receiver, addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());

    let mut output = an_output(OutputKind::Artnet, Some(&addr.to_string()));
    output.node_id = Some(NodeId::new()); // somebody else's
    handle.configure(vec![output]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    let anything = tokio::time::timeout(std::time::Duration::from_millis(150), recv(&receiver)).await;
    assert!(anything.is_err(), "two stations sending is two copies on the wire");
}

#[tokio::test]
async fn an_output_owned_by_this_station_runs() {
    let (receiver, addr) = a_receiver().await;
    let node_id = NodeId::new();
    let (manager, handle) = OutputManager::new(node_id, an_engine().await, None);
    tokio::spawn(manager.run());

    let mut output = an_output(OutputKind::Artnet, Some(&addr.to_string()));
    output.node_id = Some(node_id);
    handle.configure(vec![output]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(recv(&receiver).await[18], 255);
}

#[tokio::test]
async fn renaming_an_output_does_not_interrupt_it() {
    // Rebuilding the plugin would re-open the socket and reset its dedup cache, so
    // an unchanged universe would be re-sent — a visible blip for a rename.
    let (receiver, addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());
    let mut output = an_output(OutputKind::Artnet, Some(&addr.to_string()));

    handle.configure(vec![output.clone()]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    let first = recv(&receiver).await;

    output.name = "Front of house".into();
    handle.configure(vec![output]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    let again = tokio::time::timeout(std::time::Duration::from_millis(150), recv(&receiver)).await;
    assert!(again.is_err(), "the same frame must not be re-sent after a rename");
    assert_eq!(first[18], 255);
}

#[tokio::test]
async fn re_addressing_an_output_moves_it() {
    let (old_receiver, old_addr) = a_receiver().await;
    let (new_receiver, new_addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());
    let mut output = an_output(OutputKind::Artnet, Some(&old_addr.to_string()));

    handle.configure(vec![output.clone()]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    let _ = recv(&old_receiver).await;

    output.target = Some(new_addr.to_string());
    handle.configure(vec![output]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(recv(&new_receiver).await[18], 255, "the new address gets the show");
}

#[tokio::test]
async fn an_art_net_output_with_no_address_is_refused_rather_than_guessed_at() {
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    let engine = an_engine().await;
    let _ = &engine;
    tokio::spawn(manager.run());

    handle.configure(vec![an_output(OutputKind::Artnet, None)]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    // Nothing to assert on a wire; the point is that it does not panic or send.
}

#[tokio::test]
async fn two_outputs_feed_two_nodes_at_once() {
    let (first_node, first_addr) = a_receiver().await;
    let (second_node, second_addr) = a_receiver().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());

    handle.configure(vec![
        an_output(OutputKind::Artnet, Some(&first_addr.to_string())),
        an_output(OutputKind::Artnet, Some(&second_addr.to_string())),
    ]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(recv(&first_node).await[18], 255);
    assert_eq!(recv(&second_node).await[18], 255);
}

// ── Status ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_working_output_reports_that_it_is_sending() {
    let (_receiver, addr) = a_receiver().await;
    let engine = an_engine().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), engine.clone(), None);
    tokio::spawn(manager.run());

    let output = an_output(OutputKind::Artnet, Some(&addr.to_string()));
    handle.configure(vec![output.clone()]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    // The report runs on a one-second timer.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let statuses = statuses(&engine).await;
    let status = statuses.get(&output.id.to_string()).expect("the output is reported");
    assert_eq!(status.name, "House");
    assert_eq!(status.kind, "artnet");
    assert!(status.running);
    assert!(status.last_send.is_some(), "a mistyped address is otherwise silent");
    assert_eq!(status.error_count, 0);
    assert!(status.frames_per_second > 0.0);
}

#[tokio::test]
async fn a_failing_output_reports_what_went_wrong() {
    let engine = an_engine().await;
    let (mut manager, handle) = OutputManager::new(NodeId::new(), engine.clone(), None);
    let output = an_output(OutputKind::Artnet, None);
    manager.preload(output.clone(), Box::new(Recorder { calls: counter(), fails: true }));
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let statuses = statuses(&engine).await;
    let status = &statuses[&output.id.to_string()];
    assert_eq!(status.error_count, 1);
    assert_eq!(status.last_error.as_deref(), Some("this output is unplugged"));
}

#[tokio::test]
async fn an_output_that_stops_running_stops_being_reported() {
    let (_receiver, addr) = a_receiver().await;
    let engine = an_engine().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), engine.clone(), None);
    tokio::spawn(manager.run());

    handle.configure(vec![an_output(OutputKind::Artnet, Some(&addr.to_string()))]);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(statuses(&engine).await.len(), 1);

    handle.configure(vec![]);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert!(statuses(&engine).await.is_empty());
}

// ── Targets ───────────────────────────────────────────────────────────────────

#[test]
fn a_bare_address_takes_the_protocol_s_own_port() {
    assert_eq!(
        parse_target("10.0.0.5", 6454),
        Some("10.0.0.5:6454".parse().unwrap()),
    );
    assert_eq!(
        parse_target("10.0.0.5:1234", 6454),
        Some("10.0.0.5:1234".parse().unwrap()),
    );
    assert_eq!(parse_target("not an address", 6454), None);
}

// ── Coverage ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_fixture_no_output_carries_is_reported_until_one_does() {
    let engine = an_engine().await;
    let (manager, handle) = OutputManager::new(NodeId::new(), engine.clone(), None);
    tokio::spawn(manager.run());

    // A dimmer on universe 3 and nothing configured: a gap, naming both.
    push_a_patch(&handle, a_dimmer_patch(0.5)).await;
    let gaps = coverage(&engine).await.gaps;
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].universe, Some(3));
    assert_eq!(gaps[0].fixture_names, vec!["Spot"]);

    // An sACN output for every universe closes it — even one owned by another
    // station, because coverage is about the show, not this socket.
    let mut elsewhere = an_output(OutputKind::Sacn, None);
    elsewhere.node_id = Some(NodeId::new());
    handle.configure(vec![elsewhere]);
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert!(coverage(&engine).await.gaps.is_empty());

    // Deleting it reopens the gap, with nothing else having moved.
    handle.configure(vec![]);
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert_eq!(coverage(&engine).await.gaps.len(), 1);
}
