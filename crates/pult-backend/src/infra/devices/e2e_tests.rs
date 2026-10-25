//! End to end, against a simulated node.
//!
//! The tests beside these drive the manager's own inputs. These drive nothing but
//! the node: it is resolved, adopted, and then told to do something, and every step
//! in between — the HTTP config push, the MQTT connection the node makes on its own,
//! the topic it publishes to — is the real one. `openhaunt-node-sim` shares no code with
//! the console, so agreeing here means the two ends genuinely agree.

use openhaunt_node_sim::{Input, ModuleKind, SimConfig, SimHandle};
use pult_schema::{path::PathSegment, types::openhaunt};

use std::sync::Arc;

use pult_schema::events::operation::NodeId;
use uuid::Uuid;

use super::tests::*;
use crate::infra::showfile;

/// Start a node and hand it to the manager as though mDNS had found it.
async fn a_simulated_node(h: &Harness, module: ModuleKind, serial: &str) -> SimHandle {
    let sim = openhaunt_node_sim::start(SimConfig::new(module, serial))
        .await
        .expect("the simulator binds an ephemeral port");
    // Advertising is off, so discovery is injected rather than waited for. The
    // simulator binds 0.0.0.0; a real node would resolve to a routable address, and
    // here that is loopback.
    h.resolve_at(serial, module.type_id(), loopback(sim.http_addr)).await;
    sim
}

fn loopback(addr: std::net::SocketAddr) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], addr.port()))
}

#[tokio::test]
async fn adopting_a_node_and_pressing_a_button_moves_a_live_value() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-in").await;
    let fixture_id = h.devices.adopt("e2e-in".into()).await.unwrap();

    // The console told it where to publish, which is the only configuration a node
    // ever receives.
    let mut config = sim.received_config.clone();
    eventually("the node to be configured", || {
        let config = config.clone();
        async move { config.borrow().is_some() }
    })
    .await;
    let broker = config.borrow_and_update().clone().expect("a config just arrived");
    assert_eq!(broker["mqtt"]["broker"], serde_json::json!(h.state().await.broker_addr));

    // Pressed until it registers. The node connects to the broker on its own
    // schedule, so a press can land before the console has finished subscribing —
    // which is fine for a button, and would not be fine for a cue.
    eventually("the contact to close in the show", || {
        let (inputs, h) = (sim.inputs.clone(), &h);
        async move {
            let _ = inputs.send(Input::Contact { port: 2, state: true }).await;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            live_value(h, fixture_id, "Contact:2").await
                == Some(serde_json::json!({ "type": "Bool", "value": true }))
        }
    })
    .await;
}

#[tokio::test]
async fn a_sensor_node_reports_readings_into_the_show() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::Environment, "e2e-env").await;
    let fixture_id = h.devices.adopt("e2e-env".into()).await.unwrap();

    eventually("the temperature to arrive", || {
        let (inputs, h) = (sim.inputs.clone(), &h);
        async move {
            let _ = inputs.send(Input::Reading { port: 0, value: 19.5 }).await;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            live_value(h, fixture_id, "Temperature").await
                == Some(serde_json::json!({ "type": "Float", "value": 19.5 }))
        }
    })
    .await;
}

#[tokio::test]
async fn driving_an_output_reaches_the_node_it_is_addressed_to() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::MainsRelay, "e2e-relay").await;
    h.devices.adopt("e2e-relay".into()).await.unwrap();

    // Wait until the node has announced itself, so the console reaches it over the
    // broker rather than falling back to HTTP.
    let config = sim.received_config.clone();
    eventually("the node to be configured", || {
        let config = config.clone();
        async move { config.borrow().is_some() }
    })
    .await;

    let state = sim.state.clone();
    eventually("the relay to close", || {
        let state = state.clone();
        let devices = h.devices.clone();
        async move {
            devices.set_output("e2e-relay".into(), 0, serde_json::json!({ "state": true }));
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            state.borrow().get("0") == Some(&serde_json::json!({ "state": true }))
        }
    })
    .await;
}

#[tokio::test]
async fn a_node_answers_for_its_own_module_type() {
    // The mains warning comes from the node's descriptor, not from this console's
    // guess about what a module id means.
    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::MainsRelay, "e2e-mains").await;

    let state = h.state().await;
    assert!(state.discovered["e2e-mains"].is_mains);
    assert_eq!(state.discovered["e2e-mains"].module_type, 0x0004);
}

#[tokio::test]
async fn a_node_describes_its_own_ports_and_the_console_reads_them() {
    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::Environment, "e2e-desc").await;

    let state = h.state().await;
    let description =
        state.discovered["e2e-desc"].description.as_ref().expect("the node described itself");
    assert_eq!(description.inputs(), 3, "three readings, and nothing to drive");
    assert_eq!(description.outputs(), 0);
    assert!(description.dmx.is_none(), "a sensor forwards no universe");

    let fixture_id = h.devices.adopt("e2e-desc".into()).await.unwrap();
    let fixture = h.fixtures().await.into_iter().find(|f| f.id == fixture_id).unwrap();
    let types: Vec<pult_schema::types::fixture::FixtureType> = serde_json::from_value(
        h.engine.get(vec![PathSegment::Key("fixture_types".into())]).await.unwrap(),
    )
    .unwrap();
    let fixture_type = types.iter().find(|t| t.id == fixture.fixture_type_id).unwrap();
    assert_eq!(
        fixture_type.id,
        openhaunt::fixture_type_id(0x0007, description),
        "the id follows from what the node said, and nothing else",
    );
    assert_eq!(fixture_type.parameters.len(), 3);
}

#[tokio::test]
async fn forgetting_a_node_leaves_it_running_and_unpatched() {
    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-forget").await;
    h.devices.adopt("e2e-forget".into()).await.unwrap();
    assert_eq!(h.fixtures().await.len(), 1);

    h.devices.forget("e2e-forget".into()).await.unwrap();

    assert!(h.fixtures().await.is_empty());
    let value = h.engine.get(vec![PathSegment::Key("devices".into())]).await.unwrap();
    assert!(value["discovered"]["e2e-forget"]["online"].as_bool().unwrap());
}

#[tokio::test]
async fn a_gateway_node_receives_the_universe_it_was_adopted_onto() {
    use crate::infra::connectors::{dmx::Patch, openhaunt::OpenHauntOutput, OutputPlugin};
    use pult_schema::types::fixture::{
        Fixture, FixtureAddress, FixtureType, ParameterDefinition,
        ParameterKind, ParameterValue,
    };

    let h = harness().await;
    let mut sim = openhaunt_node_sim::start(SimConfig::new(ModuleKind::DmxOut, "e2e-gate"))
        .await
        .unwrap();
    let sacn_port = sim.sacn_addr.expect("a gateway listens for sACN").port();
    h.resolve_at("e2e-gate", ModuleKind::DmxOut.type_id(), loopback(sim.http_addr)).await;
    h.devices.adopt("e2e-gate".into()).await.unwrap();

    // The plugin is pointed at the simulator's port rather than 5568, so tests can
    // run side by side.
    let mut output =
        OpenHauntOutput::new(h.directory.clone(), h.devices.clone(), sacn_port, None).await.unwrap();

    // One ordinary dimmer, patched to the universe the gateway was given.
    let dimmer_type = FixtureType {
        id: uuid::Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition::new(
            ParameterKind::Intensity,
            ParameterValue::Float(0.0),
        )],
        ..FixtureType::default()
    };
    let mut dimmer = Fixture {
        id: uuid::Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: dimmer_type.id,
        address: FixtureAddress::dmx(1, 1),
        position: None,
        sensed_values: Default::default(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    };
    crate::infra::connectors::dmx::holding(&mut dimmer, "Intensity", ParameterValue::Float(1.0));

    let mut fixtures = h.fixtures().await;
    fixtures.push(dimmer);
    let types: Vec<FixtureType> = h
        .engine
        .get(vec![PathSegment::Key("fixture_types".into())])
        .await
        .map(|v| serde_json::from_value(v).unwrap())
        .unwrap();

    let patch = Patch::new(
        fixtures,
        types.into_iter().chain(std::iter::once(dimmer_type)).collect(),
        vec![],
    );
    output.send(&patch, &[], 0).await.unwrap();

    let (universe, channels) =
        tokio::time::timeout(std::time::Duration::from_secs(2), sim.sacn_frames.recv())
            .await
            .expect("a frame within two seconds")
            .expect("the simulator is still listening");
    assert_eq!(universe, 1);
    assert_eq!(channels[0], 255, "the dimmer at full, as the node would see it");
    assert_eq!(channels.len(), 512);
}

#[tokio::test]
async fn a_button_on_a_node_advances_a_cue() {
    // The whole path, in one test: a node publishes an edge, the console maps the
    // port to a parameter, a flow sees the change, and the sequence moves.
    use pult_schema::{
        lifecycle::Lifecycle,
        types::{
            fixture::ParameterKind,
            flow::{
                Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition,
                TriggerSource,
            },
            sequence::Sequence,
        },
    };

    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-button").await;
    let fixture_id = h.devices.adopt("e2e-button".into()).await.unwrap();

    let sequence = Sequence {
        id: uuid::Uuid::new_v4(),
        name: "Act 1".into(),
        cue_ids: vec![uuid::Uuid::new_v4(), uuid::Uuid::new_v4()],
        active_cue_index: None,
        went_at: None,
        master: None,
    };
    let flow = Flow { id: uuid::Uuid::new_v4(), name: "Front door".into(), enabled: true };
    let node = |kind| FlowNode {
        id: uuid::Uuid::new_v4(),
        flow_id: flow.id,
        kind,
        x: 0.0,
        y: 0.0,
        active: false,
        last_fired_at: None,
    };
    let source = node(FlowNodeKind::Source(TriggerSource::Parameter {
        fixture_id,
        parameter: ParameterKind::Contact(0),
    }));
    let gate = node(FlowNodeKind::Condition(TriggerCondition::RisingEdge));
    let action = node(FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: sequence.id }));
    let wire = |from: &FlowNode, to: &FlowNode| FlowEdge {
        id: uuid::Uuid::new_v4(),
        flow_id: flow.id,
        from_node: from.id,
        from_port: 0,
        to_node: to.id,
        to_port: 0,
    };
    for (table, value) in [
        ("sequences", serde_json::to_value(&sequence).unwrap()),
        ("flows", serde_json::to_value(&flow).unwrap()),
        ("flow_nodes", serde_json::to_value(&source).unwrap()),
        ("flow_nodes", serde_json::to_value(&gate).unwrap()),
        ("flow_nodes", serde_json::to_value(&action).unwrap()),
        ("flow_edges", serde_json::to_value(wire(&source, &gate)).unwrap()),
        ("flow_edges", serde_json::to_value(wire(&gate, &action)).unwrap()),
    ] {
        h.engine
            .set(
                vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())],
                Lifecycle::Persisted,
                value,
            )
            .await
            .unwrap();
    }

    eventually("the cue to advance", || {
        let (inputs, h) = (sim.inputs.clone(), &h);
        async move {
            let _ = inputs.send(Input::Contact { port: 0, state: true }).await;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            h.engine
                .get(vec![PathSegment::Key("sequences".into()), PathSegment::Id(sequence.id)])
                .await
                .map(|s| s["active_cue_index"] == serde_json::json!(0))
                .unwrap_or(false)
        }
    })
    .await;
}

/// The whole offload path, against a node that shares no code with the console.
///
/// The console works out a shape, the plugin decides the node can trace it, one
/// message crosses the wire, and then the node moves on its own while nothing at all
/// is said to it. That last part is the claim worth testing end to end: the values
/// change between two reads, and no console is talking.
#[tokio::test]
async fn a_node_that_can_trace_a_shape_is_left_to_get_on_with_it() {
    use openhaunt_node_sim::motion;
    use pult_schema::types::{
        effect::{Curve, Direction, EffectSource, RunningEffect, Shape},
        fixture::{FixtureType, ParameterValue},
    };

    use crate::infra::connectors::{dmx::Patch, openhaunt::OpenHauntOutput, OutputPlugin};

    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::Ws2812, "e2e-strip").await;
    h.devices.adopt("e2e-strip".into()).await.unwrap();

    // What the node said about itself reached the directory, which is where the
    // plugin looks before it decides to send a shape rather than samples.
    let capable = h
        .directory
        .borrow()
        .entries
        .get("e2e-strip")
        .and_then(|e| e.effects.clone())
        .and_then(|c| c.port(1).cloned())
        .expect("the node advertised its brightness port");
    assert!(capable.has_shape("sine"));

    let mut output =
        OpenHauntOutput::new(h.directory.clone(), h.devices.clone(), 5568, None).await.unwrap();

    let mut fixtures = h.fixtures().await;
    let types: Vec<FixtureType> = h
        .engine
        .get(vec![PathSegment::Key("fixture_types".into())])
        .await
        .map(|v| serde_json::from_value(v).unwrap())
        .unwrap();

    // A one-second sine on the brightness port, as playback would have worked it out.
    let now = pult_schema::types::sequence::now_ms();
    fixtures[0].live_effects.insert(
        "Intensity".into(),
        RunningEffect {
            effect_id: uuid::Uuid::new_v4(),
            curve: Curve::Shape(Shape::Sine),
            rate_hz: 1.0,
            low: ParameterValue::Float(0.0),
            high: ParameterValue::Float(1.0),
            width: 0.5,
            direction: Direction::Forward,
            phase: 0.0,
            t0: now,
            source: EffectSource::Programmer,
        },
    );

    let patch = Patch::new(fixtures, types, vec![]);
    output.send(&patch, &[], 0).await.unwrap();

    // The node has been told, once.
    let watching = sim.snapshot.clone();
    eventually("the node to start tracing", || {
        let watching = watching.clone();
        async move { watching.borrow().effects.contains_key("1") }
    })
    .await;
    let summary = sim.snapshot.borrow().effects["1"]["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("sine"), "and it knows what it is tracing: {summary}");

    // And now it moves without being told again: `send` is not called in this window,
    // so every change the node shows is one it made itself. Waited for, never slept
    // for: a loaded machine runs the node's tick late, and a fixed window read two
    // `None`s before the first sample had been drawn.
    eventually("the node to draw its first sample", || {
        let watching = watching.clone();
        async move { watching.borrow().outputs.contains_key("1") }
    })
    .await;
    let first = sim.snapshot.borrow().outputs.get("1").cloned();
    eventually("the node to move on its own", || {
        let watching = watching.clone();
        let first = first.clone();
        async move { watching.borrow().outputs.get("1").cloned() != first }
    })
    .await;

    // Taking the shape away stops it, and the console follows with a value.
    let mut settled = patch.fixtures.clone();
    settled[0].live_effects.clear();
    crate::infra::connectors::dmx::holding(
        &mut settled[0],
        "Intensity",
        ParameterValue::Float(0.25),
    );
    let settled = Patch::new(settled, patch.fixture_types.values().cloned().collect(), vec![]);
    output.send(&settled, &[], 0).await.unwrap();

    let watching = sim.snapshot.clone();
    eventually("the node to stop and take the value", || {
        let watching = watching.clone();
        async move {
            let snapshot = watching.borrow();
            snapshot.effects.is_empty()
                && snapshot.outputs.get("1") == Some(&serde_json::json!({ "value": 0.25 }))
        }
    })
    .await;

    // Two constants, written separately in two crates that share no code, naming the
    // topic the shape above was timed against. A shape is only as good as the clock
    // under it, and a clock published where nobody is listening is no clock at all.
    assert_eq!(
        motion::CLOCK_TOPIC,
        crate::infra::devices::mqtt::CLOCK_TOPIC,
        "the node subscribes to the topic the console publishes",
    );
}

// ── A node's loudspeaker ──────────────────────────────────────────────────────

/// A second of a 440 Hz sine, mono, as the WAV an operator would have dropped on a
/// timeline.
fn a_scream() -> Vec<u8> {
    let samples: Vec<f32> =
        (0..48_000).map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5).collect();
    pult_audio::wav::encode(&samples, 1, 48_000, pult_audio::wav::Encoding::Pcm16).unwrap()
}

fn a_timeline_with(clip: pult_schema::types::timeline::AudioClip, anchor_ms: u64) -> pult_schema::types::timeline::Timeline {
    pult_schema::types::timeline::Timeline {
        id: uuid::Uuid::new_v4(),
        name: "Scare".into(),
        clips: vec![clip],
        detected: None,
        source: pult_schema::types::timeline::TimelineSource::Internal,
        grid: Vec::new(),
        markers: Vec::new(),
        events: Vec::new(),
        tracks: Vec::new(),
        flashes: None,
        pending: None,
        speed_master: None,
        node_id: None,
        end: None,
        running: true,
        anchor_ms,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
        armed_by: None,
    }
}

/// The whole of a clip's life on a node: made for it, uploaded and verified by sha,
/// played at the show time the anchor says, stopped with the timeline, and deleted
/// when nothing on any timeline wants it any more. Every step over the real HTTP and
/// the real broker, to a node that shares no code with the console.
#[tokio::test]
async fn a_clip_routed_to_a_node_is_uploaded_played_stopped_and_let_go_of() {
    use pult_schema::types::timeline::{AudioClip, AudioDestination, NodeAudioMode};

    let h = harness().await;
    let mut config = SimConfig::new(ModuleKind::DryContact, "e2e-audio");
    config.node.audio = Some(openhaunt_node_sim::audio::default_description());
    let sim = openhaunt_node_sim::start(config).await.expect("the simulator starts");
    h.resolve_at("e2e-audio", ModuleKind::DryContact.type_id(), loopback(sim.http_addr)).await;
    h.devices.adopt("e2e-audio".into()).await.unwrap();
    eventually("the node to describe its loudspeaker", || async {
        h.state().await.discovered.get("e2e-audio").is_some_and(|d| d.audio.is_some())
    })
    .await;

    // A station that leads, with somewhere to keep the file.
    let net = crate::infra::net::Network::unconfigured();
    net.set_standing(crate::infra::net::Standing { is_leader: true, sacn_slot: None });
    let dir = std::env::temp_dir().join(format!("pult-node-audio-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let assets = crate::infra::assets::AssetStore::new(Some(dir), pool);
    let asset = assets.put("audio/wav", &a_scream()).await.unwrap();
    let (manager, audio) = crate::infra::audio::AudioManager::new(
        NodeId(Uuid::new_v4()),
        h.engine.clone(),
        net,
        Default::default(),
        Some(assets),
    );
    tokio::spawn(manager.with_nodes(h.directory.clone(), h.devices.clone()).run());

    let anchor = pult_schema::clock::now_ms();
    let clip = AudioClip {
        id: uuid::Uuid::new_v4(),
        name: "Scream".into(),
        asset,
        peaks: None,
        start_ms: 0,
        offset_ms: 0,
        length_ms: None,
        gain_db: -6.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        // Looped, so it is still sounding when the upload has settled and landed.
        looped: true,
        destination: AudioDestination::Node { serial: "e2e-audio".into(), mode: NodeAudioMode::Clip },
    };
    let mut timeline = a_timeline_with(clip.clone(), anchor);
    audio.timelines(vec![timeline.clone()]);

    let held = sim.audio.clone();
    eventually("the clip to be on the node", || {
        let held = held.clone();
        async move { held.clip_count() == 1 }
    })
    .await;

    let mut commands = sim.audio_commands.clone();
    let voice = crate::infra::audio::nodes::voice_name(timeline.id, clip.id);
    eventually("a play at the anchor", || {
        let commands = commands.clone();
        let voice = voice.clone();
        async move {
            commands.borrow().iter().any(|(verb, body)| {
                verb == "play" && body["voice"] == serde_json::json!(voice) && body["at"] == serde_json::json!(anchor)
            })
        }
    })
    .await;
    let play = commands
        .borrow_and_update()
        .iter()
        .find(|(verb, _)| verb == "play")
        .map(|(_, body)| body.clone())
        .unwrap();
    assert_eq!(play["loop"], serde_json::json!(true));
    let gain = play["gain"].as_f64().unwrap();
    assert!((gain - 0.501).abs() < 0.01, "-6 dB travels as a gain on play, not in the file: {gain}");

    timeline.running = false;
    audio.timelines(vec![timeline.clone()]);
    eventually("a stop for the voice", || {
        let commands = commands.clone();
        let voice = voice.clone();
        async move {
            commands.borrow().iter().any(|(verb, body)| verb == "stop" && body["voice"] == serde_json::json!(voice))
        }
    })
    .await;

    timeline.clips.clear();
    audio.timelines(vec![timeline]);
    eventually("the clip to be deleted", || {
        let held = held.clone();
        async move { held.clip_count() == 0 }
    })
    .await;
}

/// A stream-mode clip is sent as it plays: the node is told where the session starts,
/// and RTP packets arrive on the port it named.
#[tokio::test]
async fn a_stream_mode_clip_reaches_the_node_as_rtp() {
    use pult_schema::types::timeline::{AudioClip, AudioDestination, NodeAudioMode};

    let h = harness().await;
    let mut config = SimConfig::new(ModuleKind::DryContact, "e2e-stream");
    config.node.audio = Some(openhaunt_node_sim::audio::default_description());
    let sim = openhaunt_node_sim::start(config).await.expect("the simulator starts");
    h.resolve_at("e2e-stream", ModuleKind::DryContact.type_id(), loopback(sim.http_addr)).await;
    h.devices.adopt("e2e-stream".into()).await.unwrap();
    eventually("the node on the broker", || async {
        h.directory.borrow().entries.get("e2e-stream").is_some_and(|e| e.on_broker && e.audio.is_some())
    })
    .await;

    let net = crate::infra::net::Network::unconfigured();
    net.set_standing(crate::infra::net::Standing { is_leader: true, sacn_slot: None });
    let dir = std::env::temp_dir().join(format!("pult-node-stream-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let assets = crate::infra::assets::AssetStore::new(Some(dir), pool);
    let asset = assets.put("audio/wav", &a_scream()).await.unwrap();
    let (manager, audio) = crate::infra::audio::AudioManager::new(
        NodeId(Uuid::new_v4()),
        h.engine.clone(),
        net,
        Default::default(),
        Some(assets),
    );
    tokio::spawn(manager.with_nodes(h.directory.clone(), h.devices.clone()).run());

    let clip = AudioClip {
        id: uuid::Uuid::new_v4(),
        name: "Bed".into(),
        asset,
        peaks: None,
        start_ms: 0,
        offset_ms: 0,
        length_ms: None,
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        looped: true,
        destination: AudioDestination::Node { serial: "e2e-stream".into(), mode: NodeAudioMode::Stream },
    };
    let mut timeline = a_timeline_with(clip, pult_schema::clock::now_ms());
    audio.timelines(vec![timeline.clone()]);

    let commands = sim.audio_commands.clone();
    eventually("a stream start", || {
        let commands = commands.clone();
        async move {
            commands.borrow().iter().any(|(verb, body)| verb == "stream" && body["op"] == serde_json::json!("start"))
        }
    })
    .await;
    let held = sim.audio.clone();
    eventually("packets on the stream port", || {
        let held = held.clone();
        async move { held.stream_packets() > 10 }
    })
    .await;
    // Nothing was uploaded: a streamed clip never goes to the store.
    assert_eq!(sim.audio.clip_count(), 0);

    timeline.running = false;
    audio.timelines(vec![timeline]);
    eventually("a stream stop", || {
        let commands = commands.clone();
        async move {
            commands.borrow().iter().any(|(verb, body)| verb == "stream" && body["op"] == serde_json::json!("stop"))
        }
    })
    .await;
}

// ── Heads and strips ──────────────────────────────────────────────────────────

async fn types(h: &Harness) -> Vec<pult_schema::types::fixture::FixtureType> {
    let value = h.engine.get(vec![PathSegment::Key("fixture_types".into())]).await.unwrap();
    serde_json::from_value(value).unwrap()
}

fn type_of<'a>(
    types: &'a [pult_schema::types::fixture::FixtureType],
    fixture: &pult_schema::types::fixture::Fixture,
) -> &'a pult_schema::types::fixture::FixtureType {
    types.iter().find(|t| t.id == fixture.fixture_type_id).expect("a fixture has its type")
}

#[tokio::test]
async fn a_v0_node_is_a_fixture_per_head_and_each_strip_has_its_own_color() {
    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::NodeV0, "e2e-v0").await;
    let node = h.devices.adopt("e2e-v0".into()).await.unwrap();

    let fixtures: Vec<_> =
        h.fixtures().await.into_iter().filter(|f| f.address.serial() == Some("e2e-v0")).collect();
    assert_eq!(fixtures.len(), 3, "the node, and a fixture per strip");
    let types = types(&h).await;

    let own = fixtures.iter().find(|f| f.id == node).unwrap();
    assert_eq!(own.address.head(), Some(0));
    assert_eq!(type_of(&types, own).parameters.len(), 6, "four relays and two sensors, no strip ports");

    for head in [1u8, 2] {
        let strip = fixtures.iter().find(|f| f.address.head() == Some(head)).unwrap();
        assert_eq!(strip.id, openhaunt::head_fixture_id("e2e-v0", head), "derived, so stable");
        let t = type_of(&types, strip);
        assert_eq!(t.parameters.len(), 9);
        assert!(t.parameters.iter().any(|p| p.kind == pult_schema::types::ParameterKind::ColorRgb));
        let effect = t.parameters.iter().find(|p| !p.slots.is_empty()).expect("the Effect port");
        assert!(effect.slots.iter().any(|s| s.name == "chase"));
        assert_eq!(strip.address.strip().unwrap().mode, pult_schema::types::pixels::StripMode::Effect);
    }
    let state = h.state().await;
    assert_eq!(state.discovered["e2e-v0"].head_fixtures.len(), 2);
}

#[tokio::test]
async fn a_strip_patched_in_sacn_is_one_fixture_with_a_cell_per_pixel_and_the_node_is_told() {
    use pult_schema::types::pixels::StripMode;

    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::NodeV0, "e2e-sacn").await;
    h.devices.adopt("e2e-sacn".into()).await.unwrap();
    let before = h.fixtures().await.len();

    let change = crate::infra::devices::StripChange {
        mode: Some(StripMode::Sacn),
        count: Some(300),
        ..Default::default()
    };
    let head = h
        .devices
        .patch_strip("e2e-sacn".into(), 2, change, pult_schema::types::user::User::DEFAULT_ID)
        .await
        .unwrap();

    // The node runs what the show says, in the universes the console picked.
    let running = sim.pixels.strips()[1].clone();
    assert_eq!(running.mode, "sacn");
    assert_eq!(running.count, 300);
    assert_eq!(running.universe, 1, "the first free run of two universes");

    // One fixture still: the strip is its pixels.
    let fixtures = h.fixtures().await;
    assert_eq!(fixtures.len(), before, "no fixture per pixel");
    let types = types(&h).await;
    let strip = fixtures.iter().find(|f| f.id == head).unwrap();
    let strip_type = type_of(&types, strip);
    assert_eq!(strip_type.cells, 300);
    let placement = strip.address.dmx_placement().expect("a sACN strip says where its bytes go").1;
    assert_eq!(placement.iter().map(|b| b.universe).collect::<Vec<_>>(), vec![1, 2], "spilled at 170");

    // The directory says the node reads both universes, which is what feeds it.
    let entry = h.directory.borrow().entries["e2e-sacn"].clone();
    assert_eq!(entry.universes, vec![1, 2]);

    // And back: the nine ports back on the strip's fixture.
    let back = crate::infra::devices::StripChange { mode: Some(StripMode::Effect), ..Default::default() };
    h.devices
        .patch_strip("e2e-sacn".into(), 2, back, pult_schema::types::user::User::DEFAULT_ID)
        .await
        .unwrap();
    assert_eq!(sim.pixels.strips()[1].mode, "effect");
    eventually("the strip to carry its node's nine ports again", || async {
        let fixtures = h.fixtures().await;
        let strip = fixtures.iter().find(|f| f.id == head).unwrap();
        let types = super::e2e_tests::types(&h).await;
        type_of(&types, strip).parameters.len() == 9 && type_of(&types, strip).cells == 0
    })
    .await;
}

#[tokio::test]
async fn a_node_running_another_mode_is_put_back_and_a_refusal_is_a_fault() {
    use pult_schema::types::pixels::StripMode;

    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::NodeV0, "e2e-back").await;
    h.devices.adopt("e2e-back".into()).await.unwrap();
    let change = crate::infra::devices::StripChange {
        mode: Some(StripMode::Sacn),
        count: Some(10),
        ..Default::default()
    };
    h.devices
        .patch_strip("e2e-back".into(), 1, change, pult_schema::types::user::User::DEFAULT_ID)
        .await
        .unwrap();

    // Somebody at the bench puts the strip back into effect mode.
    sim.pixels.put(0, &serde_json::json!({ "mode": "effect" })).unwrap();
    // The node announces itself again, as it does when its description changes.
    h.resolve_at("e2e-back", ModuleKind::NodeV0.type_id(), loopback(sim.http_addr)).await;
    let pixels = sim.pixels.clone();
    eventually("the strip to be put back", || {
        let pixels = pixels.clone();
        async move { pixels.strips()[0].mode == "sacn" }
    })
    .await;

    // A strip the node will not run is refused before anything is written.
    let too_long = crate::infra::devices::StripChange { count: Some(5000), ..Default::default() };
    let refused = h
        .devices
        .patch_strip("e2e-back".into(), 1, too_long, pult_schema::types::user::User::DEFAULT_ID)
        .await;
    assert!(refused.unwrap_err().contains("1 to 1024"));
}

#[tokio::test]
async fn patching_a_strip_is_one_ctrl_z() {
    use pult_schema::types::{pixels::StripMode, user::User};

    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::NodeV0, "e2e-undo").await;
    h.devices.adopt("e2e-undo".into()).await.unwrap();
    let id = openhaunt::head_fixture_id("e2e-undo", 1);
    let effect_type = h.fixtures().await.iter().find(|f| f.id == id).unwrap().fixture_type_id;
    let change = crate::infra::devices::StripChange {
        mode: Some(StripMode::Sacn),
        count: Some(40),
        ..Default::default()
    };
    h.devices.patch_strip("e2e-undo".into(), 1, change, User::DEFAULT_ID).await.unwrap();
    let strip = h.fixtures().await.into_iter().find(|f| f.id == id).unwrap();
    assert_eq!(type_of(&types(&h).await, &strip).cells, 40);

    h.engine.undo(User::DEFAULT_ID, false).await;
    let strip = h.fixtures().await.into_iter().find(|f| f.id == id).unwrap();
    assert_eq!(strip.address.strip().unwrap().mode, StripMode::Effect, "the mode");
    assert_eq!(strip.fixture_type_id, effect_type, "and its type, with it");
}
