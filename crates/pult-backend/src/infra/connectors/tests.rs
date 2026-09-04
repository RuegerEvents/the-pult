//! Output manager tests.
//!
//! What the manager owns is which plugins exist and whether they are being fed —
//! not what any one of them puts on a wire, which is each connector's own tests.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use pult_schema::types::{
    effect::{Easing, RunningFade},
    fixture::{
        Fixture, FixtureAddress, FixtureType, ParameterDefinition,
        ParameterKind, ParameterValue,
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

/// A plugin that draws its own frames and writes down what it worked out on each.
///
/// The stand-in for a protocol that carries values: it is handed what is *driving*
/// the rig and a moment, and evaluates for itself, which is the whole of what a
/// connector does now.
struct Sampler {
    levels: Arc<std::sync::Mutex<Vec<f32>>>,
    period: std::time::Duration,
}

impl OutputPlugin for Sampler {
    fn name(&self) -> &'static str {
        "sampler"
    }

    fn frames(&self) -> Frames {
        Frames { while_moving: Some(self.period), when_settled: Some(self.period) }
    }

    fn send<'a>(&'a mut self, patch: &'a Patch, _changed: &'a [Uuid], now_ms: u64) -> SendFuture<'a> {
        Box::pin(async move {
            let began = std::time::Instant::now();
            for fixture in &patch.fixtures {
                if let Some(ParameterValue::Float(level)) =
                    patch.value_at(fixture, "Intensity", now_ms)
                {
                    self.levels.lock().unwrap().push(level);
                }
            }
            Ok(Frame::evaluated(began.elapsed()))
        })
    }
}

/// A patch with a fade running on its one fixture, from `t0` for `duration_ms`.
fn a_fading_patch(t0: u64, duration_ms: u32) -> Patch {
    let mut patch = a_dimmer_patch(0.0);
    patch.fixtures[0].live_fades.insert(
        "Intensity".into(),
        RunningFade {
            from: ParameterValue::Float(0.0),
            to: ParameterValue::Float(1.0),
            t0,
            duration_ms,
            easing: Easing::Linear,
            cue_id: Uuid::nil(),
        },
    );
    Patch::new(
        patch.fixtures.clone(),
        patch.fixture_types.values().cloned().collect(),
        vec![],
    )
}

impl OutputPlugin for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }

    fn send<'a>(
        &'a mut self,
        _patch: &'a Patch,
        _changed: &'a [Uuid],
        _now_ms: u64,
    ) -> SendFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fails {
                anyhow::bail!("this output is unplugged");
            }
            Ok(Frame::default())
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
        parameters: vec![ParameterDefinition::new(
            ParameterKind::Intensity,
            ParameterValue::Float(0.0),
        )],
        ..FixtureType::default()
    };
    let mut fixture = Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::dmx(3, 1),
        position: None,
        sensed_values: Default::default(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    };
    // A parameter sits at a value by having a landed fade on it. Nothing stores the
    // number, so this is the shape of "the console is holding this light at a level".
    fixture.live_fades.insert(
        "Intensity".into(),
        RunningFade {
            from: ParameterValue::Float(level),
            to: ParameterValue::Float(level),
            t0: 0,
            duration_ms: 0,
            easing: Easing::Step,
            cue_id: Uuid::nil(),
        },
    );
    Patch::new(vec![fixture], vec![fixture_type], vec![])
}

async fn push_a_patch(handle: &OutputHandle, patch: Patch) {
    handle.push(
        patch.fixtures.clone(),
        patch.fixture_types.values().cloned().collect(),
        patch.programmer.clone(),
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

/// A plugin that describes itself, and remembers whether anybody was looking.
///
/// The stand-in for a connector with a viewer of its own — which is what the seam has
/// to carry, since the whole point of describing traffic by *shape* is that a
/// protocol nobody has written yet can be watched.
struct Describer {
    /// What it says it is doing, changed by the test to make a view move.
    saying: Arc<std::sync::Mutex<String>>,
    /// How many times it has been asked, and whether it was told anybody is reading.
    looks: Arc<AtomicUsize>,
    watched: Arc<std::sync::atomic::AtomicBool>,
}

impl OutputPlugin for Describer {
    fn name(&self) -> &'static str {
        "describer"
    }

    fn send<'a>(&'a mut self, _p: &'a Patch, _c: &'a [Uuid], _n: u64) -> SendFuture<'a> {
        Box::pin(async move { Ok(Frame::default()) })
    }

    fn watched(&mut self, watching: bool) {
        self.watched.store(watching, Ordering::SeqCst);
    }

    fn observe(&mut self, focus: Option<&str>) -> Option<Vec<OutputSection>> {
        self.looks.fetch_add(1, Ordering::SeqCst);
        Some(vec![OutputSection {
            title: format!("focus {}", focus.unwrap_or("none")),
            note: None,
            body: pult_schema::types::output::SectionBody::Messages(
                pult_schema::types::output::MessageTraffic {
                    messages: vec![pult_schema::types::output::OutputMessage {
                        at_ms: 0,
                        to: "somewhere".into(),
                        what: self.saying.lock().unwrap().clone(),
                        detail: String::new(),
                    }],
                    dropped: 0,
                },
            ),
        }])
    }
}

/// The manager, with somebody able to watch it.
fn a_watchable_manager(
    node_id: NodeId,
    engine: EngineHandle,
) -> (OutputManager, OutputHandle, Viewers, crate::engine::UpdateBroadcast) {
    let (manager, handle, _costs) = OutputManager::new(node_id, engine, None);
    let viewers = Viewers::default();
    let updates = crate::engine::UpdateBroadcast::new();
    (manager.watchable(viewers.clone(), updates.clone()), handle, viewers, updates)
}

fn a_describer() -> (Describer, Arc<std::sync::Mutex<String>>, Arc<AtomicUsize>, Arc<std::sync::atomic::AtomicBool>) {
    let saying = Arc::new(std::sync::Mutex::new("first".to_string()));
    let looks = counter();
    let watched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    (
        Describer { saying: saying.clone(), looks: looks.clone(), watched: watched.clone() },
        saying,
        looks,
        watched,
    )
}

/// The next view pushed at the browsers, or nothing within the time given.
async fn next_view(
    rx: &mut tokio::sync::broadcast::Receiver<(pult_schema::path::Path, serde_json::Value)>,
    within: std::time::Duration,
) -> Option<pult_schema::types::output::OutputView> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Ok((path, value))) => {
                if path.first() == Some(&PathSegment::Key("output_traffic".into())) {
                    return serde_json::from_value(value).ok();
                }
            }
            Ok(Err(_)) => return None,
            Err(_) => return None,
        }
    }
}

// ── Being looked at ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_connector_nobody_is_watching_is_never_asked() {
    let node_id = NodeId::new();
    let (mut manager, handle, _viewers, _updates) =
        a_watchable_manager(node_id, an_engine().await);
    let (describer, _saying, looks, watched) = a_describer();
    manager.preload(an_output(OutputKind::Artnet, None), Box::new(describer));
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert_eq!(count(&looks), 0, "drawing a view for nobody is the cost this design refuses");
    assert!(!watched.load(Ordering::SeqCst));
}

#[tokio::test]
async fn watching_starts_the_drawing_and_letting_go_stops_it() {
    let node_id = NodeId::new();
    let (mut manager, _handle, viewers, updates) = a_watchable_manager(node_id, an_engine().await);
    let output = an_output(OutputKind::Artnet, None);
    let id = output.id;
    let (describer, saying, looks, watched) = a_describer();
    manager.preload(output, Box::new(describer));
    let mut rx = updates.0.subscribe();
    tokio::spawn(manager.run());

    let alice = Uuid::new_v4();
    viewers.watch(node_id, id, alice, Some("3".into()));

    let view = next_view(&mut rx, std::time::Duration::from_secs(1))
        .await
        .expect("a view within a second of asking");
    assert_eq!(view.output_id, id);
    assert_eq!(view.focus.as_deref(), Some("3"));
    assert_eq!(view.sections[0].title, "focus 3", "the focus reaches the connector as asked");
    assert!(watched.load(Ordering::SeqCst), "and the connector was told somebody is reading");

    let while_watched = count(&looks);
    viewers.forget(alice);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let after = count(&looks);
    assert!(after > 0);
    *saying.lock().unwrap() = "second".into();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(count(&looks), after, "nobody is looking, so nothing is drawn");
    assert!(while_watched > 0);
    assert!(!watched.load(Ordering::SeqCst), "and the connector was told to stop keeping it");
}

#[tokio::test]
async fn a_view_that_has_not_changed_is_not_sent_again() {
    let node_id = NodeId::new();
    let (mut manager, _handle, viewers, updates) = a_watchable_manager(node_id, an_engine().await);
    let output = an_output(OutputKind::Artnet, None);
    let id = output.id;
    let (describer, saying, _looks, _watched) = a_describer();
    manager.preload(output, Box::new(describer));
    let mut rx = updates.0.subscribe();
    tokio::spawn(manager.run());

    viewers.watch(node_id, id, Uuid::new_v4(), None);
    let first = next_view(&mut rx, std::time::Duration::from_secs(1)).await;
    assert!(first.is_some());

    // Several draws pass and the connector says the same thing every time, which is
    // what a settled rig looks like: the wire should stay quiet.
    assert!(
        next_view(&mut rx, std::time::Duration::from_millis(500)).await.is_none(),
        "an unchanged view is not news, however often it is drawn"
    );

    *saying.lock().unwrap() = "something else".into();
    let moved = next_view(&mut rx, std::time::Duration::from_secs(1)).await;
    assert!(moved.is_some(), "and a changed one arrives without being asked for again");
}

#[tokio::test]
async fn a_connector_that_does_not_describe_itself_says_so_by_saying_nothing() {
    let node_id = NodeId::new();
    let (mut manager, _handle, viewers, updates) = a_watchable_manager(node_id, an_engine().await);
    let output = an_output(OutputKind::Artnet, None);
    let id = output.id;
    manager.preload(output, Box::new(Recorder { calls: counter(), fails: false }));
    let mut rx = updates.0.subscribe();
    tokio::spawn(manager.run());

    viewers.watch(node_id, id, Uuid::new_v4(), None);
    assert!(
        next_view(&mut rx, std::time::Duration::from_millis(500)).await.is_none(),
        "the default answer is nothing, and the panel says so rather than drawing an empty sheet"
    );
}

// ── Feeding the plugins ───────────────────────────────────────────────────────

#[tokio::test]
async fn every_plugin_receives_the_patch() {
    let first = counter();
    let second = counter();
    let (mut manager, handle, _costs) =
        OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (mut manager, handle, _costs) =
        OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());

    handle.configure(vec![an_output(OutputKind::Artnet, Some(&addr.to_string()))]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(recv(&receiver).await[18], 255);
}

#[tokio::test]
async fn removing_an_output_stops_it() {
    let (receiver, addr) = a_receiver().await;
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(node_id, an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
async fn narrowing_and_widening_the_universes_takes_effect_while_the_show_is_up() {
    // The configured filter reaching the connector at all, and an operator changing
    // it at half past six being obeyed. `same_wire` counts `universes`, so the change
    // rebuilds the plugin and the fresh dedup cache is what puts the first frame out.
    let (receiver, addr) = a_receiver().await;
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
    tokio::spawn(manager.run());
    let mut output = an_output(OutputKind::Artnet, Some(&addr.to_string()));
    output.universes = vec![9]; // the rig is on universe 3

    handle.configure(vec![output.clone()]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    let anything =
        tokio::time::timeout(std::time::Duration::from_millis(150), recv(&receiver)).await;
    assert!(anything.is_err(), "this node was configured for a universe nothing is patched to");

    output.universes = vec![3];
    handle.configure(vec![output]);
    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    let packet = recv(&receiver).await;
    assert_eq!(packet[14], 3);
    assert_eq!(packet[18], 255);
}

#[tokio::test]
async fn an_art_net_output_with_no_address_is_refused_rather_than_guessed_at() {
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), an_engine().await, None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), engine.clone(), None);
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
    let (mut manager, handle, _costs) = OutputManager::new(NodeId::new(), engine.clone(), None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), engine.clone(), None);
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
    let (manager, handle, _costs) = OutputManager::new(NodeId::new(), engine.clone(), None);
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

// ── A connector's own frames ──────────────────────────────────────────────────

/// The bargain this change made, on one connector: the engine says what is driving
/// the rig once, and the connector draws a moving value out of it for as long as it
/// likes without hearing anything more.
#[tokio::test]
async fn a_connector_draws_a_moving_value_from_one_patch() {
    let levels: Arc<std::sync::Mutex<Vec<f32>>> = Default::default();
    let (mut manager, handle, _costs) =
        OutputManager::new(NodeId::new(), an_engine().await, None);
    manager.preload(
        an_output(OutputKind::Artnet, None),
        Box::new(Sampler {
            levels: levels.clone(),
            period: std::time::Duration::from_millis(20),
        }),
    );
    tokio::spawn(manager.run());

    // A fade starting now and running for half a second. One push, and nothing else
    // is ever said to this connector.
    let now = pult_schema::types::sequence::now_ms();
    push_a_patch(&handle, a_fading_patch(now, 500)).await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let seen = levels.lock().unwrap().clone();
    assert!(seen.len() > 5, "the connector drew its own frames: {seen:?}");
    assert!(
        seen.windows(2).any(|w| w[1] > w[0]),
        "and the value moved between them, with nobody writing anything: {seen:?}",
    );
    assert!(
        seen.last().is_some_and(|last| *last > 0.4),
        "and it got most of the way up: {seen:?}",
    );
}

/// The other half: once the fade has landed the connector drops to its protocol's
/// keep-alive rather than going on drawing at frame rate.
#[tokio::test]
async fn a_settled_patch_drops_to_the_keep_alive_rate() {
    let levels: Arc<std::sync::Mutex<Vec<f32>>> = Default::default();
    let (mut manager, handle, _costs) =
        OutputManager::new(NodeId::new(), an_engine().await, None);
    manager.preload(
        an_output(OutputKind::Artnet, None),
        Box::new(Sampler {
            levels: levels.clone(),
            period: std::time::Duration::from_millis(20),
        }),
    );
    tokio::spawn(manager.run());

    // A fade that landed long ago: nothing in this patch is moving.
    push_a_patch(&handle, a_fading_patch(0, 0)).await;
    let after_the_push = levels.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let drawn = levels.lock().unwrap().len() - after_the_push;
    assert!(
        drawn <= 12,
        "a settled patch should not be drawn at frame rate: {drawn} frames in 200 ms",
    );
}

/// Each connector on its own account, because their rates and their costs are their
/// own — and a connector that emitted nothing at all carries no figure rather than a
/// figure of zero.
#[tokio::test]
async fn each_connector_reports_its_own_frame_cost() {
    let (mut manager, handle, mut costs) =
        OutputManager::new(NodeId::new(), an_engine().await, None);
    let busy = an_output(OutputKind::Artnet, None);
    let mut quiet = an_output(OutputKind::Sacn, None);
    quiet.name = "Guest console".into();
    manager.preload(
        busy.clone(),
        Box::new(Sampler {
            levels: Default::default(),
            period: std::time::Duration::from_millis(20),
        }),
    );
    // No frames of its own, and one push is all it will ever be given.
    manager.preload(quiet.clone(), Box::new(Recorder { calls: counter(), fails: false }));
    tokio::spawn(manager.run());

    let now = pult_schema::types::sequence::now_ms();
    push_a_patch(&handle, a_fading_patch(now, 4_000)).await;

    // The manager closes a window every second.
    let published = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            costs.changed().await.expect("the manager is still running");
            let now = costs.borrow().clone();
            if now.len() == 2 {
                return now;
            }
        }
    })
    .await
    .expect("both connectors to report inside three seconds");

    let house = published.iter().find(|c| c.output == busy.name).expect("the busy one");
    let guest = published.iter().find(|c| c.output == quiet.name).expect("the quiet one");
    assert!(house.frames > guest.frames, "{house:?} vs {guest:?}");
    assert!(house.evaluating_mean_ms <= house.mean_ms, "the half is inside the whole");
    assert_eq!(guest.frames, 1, "one push, and no frames of its own");

    // And once nothing is pushed and the quiet one has no rate, it carries no figure
    // at all rather than a figure of zero.
    let later = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            costs.changed().await.expect("the manager is still running");
            let now = costs.borrow().clone();
            if !now.iter().any(|c| c.output == quiet.name) {
                return now;
            }
        }
    })
    .await
    .expect("the quiet connector to fall silent inside three seconds");
    assert!(later.iter().all(|c| c.frames > 0), "nothing reports a window of no frames");
}

// ── When the next frame goes ──────────────────────────────────────────────────
//
// `schedule` is the whole of a connector's rate, and it was measured from the
// moment the loop woke rather than from the deadline it woke to. Nothing wakes
// exactly on time, so every frame was late by the sum of every lateness before it:
// a persistent 2.4 ms on a 25 ms period made a 40 Hz connector draw at 36.

/// A plugin that wants a frame at whatever rate the test says, and a different one
/// while settled, so a change of gait can be asked about.
struct Paced {
    moving: std::time::Duration,
    settled: std::time::Duration,
}

impl OutputPlugin for Paced {
    fn name(&self) -> &'static str {
        "paced"
    }
    fn frames(&self) -> Frames {
        Frames { while_moving: Some(self.moving), when_settled: Some(self.settled) }
    }
    fn send<'a>(&'a mut self, _: &'a Patch, _: &'a [Uuid], _: u64) -> SendFuture<'a> {
        Box::pin(async move { Ok(Frame::evaluated(std::time::Duration::ZERO)) })
    }
}

fn a_paced(moving_ms: u64, settled_ms: u64) -> Running {
    Running::new(
        an_output(OutputKind::Artnet, None),
        Box::new(Paced {
            moving: std::time::Duration::from_millis(moving_ms),
            settled: std::time::Duration::from_millis(settled_ms),
        }),
    )
}

#[test]
fn lateness_is_jitter_about_a_rate_and_not_a_debt_that_compounds() {
    let mut output = a_paced(25, 800);
    let start = std::time::Instant::now();
    output.next_frame = Some(start);

    // Twenty frames, each woken 3 ms after its deadline — which is what the loop
    // actually does on a real machine.
    let late = std::time::Duration::from_millis(3);
    let mut due = start;
    for _ in 0..20 {
        output.schedule(due + late, true);
        due = output.next_frame.expect("a paced connector always wants another");
    }

    assert_eq!(
        due.duration_since(start),
        std::time::Duration::from_millis(20 * 25),
        "twenty 25 ms frames have to land 500 ms after the first however late each \
         wake was. Measured from the wake instead of the deadline this is 560 ms, \
         which is 36 Hz on a connector asking for 40."
    );
}

#[test]
fn a_connector_short_of_frame_is_asked_again_at_once() {
    let mut output = a_paced(25, 800);
    let start = std::time::Instant::now();
    output.next_frame = Some(start);

    // Woken 40 ms after a deadline on a 25 ms period: the chained deadline is already
    // in the past, and there is no catching that up by scheduling into it.
    let woke = start + std::time::Duration::from_millis(40);
    output.schedule(woke, true);

    assert_eq!(
        output.next_frame, Some(woke),
        "a connector that cannot hold its rate draws flat out rather than accruing a \
         backlog of deadlines it will burst through later"
    );
}

#[test]
fn a_settled_connector_that_starts_moving_does_not_wait_out_its_keep_alive() {
    let mut output = a_paced(25, 800);
    let start = std::time::Instant::now();
    // Settled: the next keep-alive is 800 ms out.
    output.schedule(start, false);
    assert_eq!(output.next_frame, Some(start + std::time::Duration::from_millis(800)));

    // The keep-alive comes round, and by then a cue is running.
    let woke = start + std::time::Duration::from_millis(800);
    output.schedule(woke, true);

    assert_eq!(
        output.next_frame,
        Some(woke + std::time::Duration::from_millis(25)),
        "chaining from the deadline must not carry the old gait's period with it: an \
          800 ms keep-alive deadline plus 25 ms would make the second frame of a cue \
         arrive when the first light had already got where it was going"
    );
}
