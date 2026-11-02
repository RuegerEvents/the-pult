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
        sensed_at: Default::default(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        live_htp: Default::default(),
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

    let sequence = Sequence { merge: None, wrap: None,
        folder: None,
        id: uuid::Uuid::new_v4(),
        name: "Act 1".into(),
        cue_ids: vec![uuid::Uuid::new_v4(), uuid::Uuid::new_v4()],
        active_cue_index: None,
        went_at: None,
        master: None,
    };
    let flow = Flow { folder: None, id: uuid::Uuid::new_v4(), name: "Front door".into(), enabled: true };
    let node = |kind| FlowNode {
        name: None,
        due_at: None,
        disabled: None,
        id: uuid::Uuid::new_v4(),
        flow_id: flow.id,
        kind,
        x: 0.0,
        y: 0.0,
        active: false,
        last_fired_at: None,
        pulsed_at: None,
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
        folder: None,
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
        repeats: 1_000,
        stored: false,
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
        repeats: 1_000,
        stored: false,
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

/// What is published to a node arrives in the order it was asked for. Each publish
/// was a task of its own, and on a runtime with several workers two of them reached the
/// client in whichever order they were scheduled: a stream's `stop` overtook the next
/// one's `start`. Several workers here for that reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_is_published_to_a_node_arrives_in_order() {
    let h = harness().await;
    let mut config = SimConfig::new(ModuleKind::DryContact, "e2e-order");
    config.node.audio = Some(openhaunt_node_sim::audio::default_description());
    let sim = openhaunt_node_sim::start(config).await.expect("the simulator starts");
    h.resolve_at("e2e-order", ModuleKind::DryContact.type_id(), loopback(sim.http_addr)).await;
    h.devices.adopt("e2e-order".into()).await.unwrap();
    eventually("the node on the broker", || async {
        h.directory.borrow().entries.get("e2e-order").is_some_and(|e| e.on_broker)
    })
    .await;

    // Fewer than the device manager's queue holds, so none is dropped for being late.
    for n in 0..40 {
        h.devices.audio("e2e-order".into(), "gain", serde_json::json!({ "voice": "order", "gain": n }));
    }
    let commands = sim.audio_commands.clone();
    let gains = || -> Vec<i64> {
        commands.borrow().iter().filter(|(verb, _)| verb == "gain").filter_map(|(_, body)| body["gain"].as_i64()).collect()
    };
    eventually("every gain at the node", || {
        let got = gains().len();
        async move { got == 40 }
    })
    .await;
    assert_eq!(gains(), (0..40).collect::<Vec<_>>());
}

/// A leader restarted with a stream playing, whose browse never finds the node again.
/// The node walks back onto the same broker and says where it is on `net`, and that is
/// all the stream needs: found on the floor as a node on the broker, a timeline
/// running, and no `start` for as long as mDNS stayed quiet.
#[tokio::test]
async fn a_stream_resumes_after_a_restart_that_mdns_never_sees() {
    use pult_schema::types::timeline::{AudioClip, AudioDestination, NodeAudioMode};

    let port = super::tests::free_port();
    let pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let (engine, engine_handle, _broadcast) = crate::engine::ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
    tokio::spawn(engine.run());
    let station = NodeId(Uuid::new_v4());
    let (manager, devices, directory) = super::DeviceManager::new(station, engine_handle.clone(), port);
    tokio::spawn(manager.run());
    let h = Harness { engine: engine_handle.clone(), devices, directory };

    let mut config = SimConfig::new(ModuleKind::DryContact, "e2e-unseen");
    config.node.audio = Some(openhaunt_node_sim::audio::default_description());
    let sim = openhaunt_node_sim::start(config).await.expect("the simulator starts");
    h.resolve_at("e2e-unseen", ModuleKind::DryContact.type_id(), loopback(sim.http_addr)).await;
    h.devices.adopt("e2e-unseen".into()).await.unwrap();
    eventually("the node on the broker", || async {
        h.directory.borrow().entries.get("e2e-unseen").is_some_and(|e| e.on_broker && e.audio.is_some())
    })
    .await;

    let dir = std::env::temp_dir().join(format!("pult-node-unseen-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let assets_pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let assets = crate::infra::assets::AssetStore::new(Some(dir), assets_pool);
    let asset = assets.put("audio/wav", &a_scream()).await.unwrap();
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
        repeats: 1_000,
        stored: false,
        destination: AudioDestination::Node { serial: "e2e-unseen".into(), mode: NodeAudioMode::Stream },
    };
    let timeline = a_timeline_with(clip, pult_schema::clock::now_ms());
    let start_audio = |h: &Harness| {
        let net = crate::infra::net::Network::unconfigured();
        net.set_standing(crate::infra::net::Standing { is_leader: true, sacn_slot: None });
        let (manager, audio) = crate::infra::audio::AudioManager::new(
            NodeId(Uuid::new_v4()),
            h.engine.clone(),
            net,
            Default::default(),
            Some(assets.clone()),
        );
        tokio::spawn(manager.with_nodes(h.directory.clone(), h.devices.clone()).run());
        audio
    };
    let audio = start_audio(&h);
    audio.timelines(vec![timeline.clone()]);
    let commands = sim.audio_commands.clone();
    let starts = |commands: &tokio::sync::watch::Receiver<Vec<(String, serde_json::Value)>>| {
        commands.borrow().iter().filter(|(verb, body)| verb == "stream" && body["op"] == serde_json::json!("start")).count()
    };
    eventually("a stream start", || {
        let commands = commands.clone();
        async move { starts(&commands) > 0 }
    })
    .await;

    // The station goes, and comes back on the same broker port with no browse result.
    let _ = audio.0.send(crate::infra::audio::AudioCommand::Stop).await;
    h.devices.0.send(super::DeviceCommand::Stop).await.unwrap();
    let before = starts(&commands);
    let (manager, devices, directory) = super::DeviceManager::new(station, engine_handle.clone(), port);
    tokio::spawn(manager.asking_unseen_nodes_on(sim.http_addr.port()).run());
    let h = Harness { engine: engine_handle, devices, directory };
    let _audio = start_audio(&h);
    _audio.timelines(vec![timeline]);

    // Back on the broker, and in the directory for it, with no address yet.
    eventually("the node on the broker again", || async {
        h.directory.borrow().entries.get("e2e-unseen").is_some_and(|e| e.on_broker && e.adopted)
    })
    .await;
    // What the firmware says on every connect, retained (OpenHaunt docs/08); the
    // simulator does not, so it is said for it.
    let mut options = rumqttc::MqttOptions::new("e2e-unseen-net", "127.0.0.1", port);
    options.set_keep_alive(std::time::Duration::from_secs(5));
    let (client, mut eventloop) = rumqttc::AsyncClient::new(options, 16);
    tokio::spawn(async move { while eventloop.poll().await.is_ok() {} });
    client
        .publish(
            "openhaunt/e2e-unseen/net",
            rumqttc::QoS::AtLeastOnce,
            true,
            r#"{"addresses":[{"transport":"ethernet","ip":"127.0.0.1"}]}"#,
        )
        .await
        .unwrap();

    for _ in 0..600 {
        if starts(&commands) > before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(starts(&commands) > before, "a stream start after the restart, with no browse");
    let packets = sim.audio.stream_packets();
    eventually("packets of the new session", || {
        let held = sim.audio.clone();
        async move { held.stream_packets() > packets + 10 }
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

#[tokio::test]
async fn stepping_down_hands_whether_a_node_is_online_back_to_the_browse() {
    // The audio MacBook led for a moment while the Mini restarted, and the nodes came to
    // its broker. When the Mini answered again they walked back to it — leaving this
    // broker *before* the MacBook stepped down, so each one's "offline" will arrived
    // while it still drove them, and was believed. mDNS had shown every node already
    // and said nothing again: five nodes offline on that station for the rest of the
    // night (HHG, 2026-10-30). What a broker this station no longer runs said stops
    // counting when it steps down; what its own browse shows is the answer again.
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-walk").await;
    h.devices.adopt("e2e-walk".into()).await.unwrap();
    eventually("the node to be on this station's broker", || async {
        h.directory.borrow().entries.get("e2e-walk").is_some_and(|e| e.on_broker)
    })
    .await;

    // The node leaves while this station still drives it: its will says offline, and
    // while driving that is believed.
    sim.stop.stop().await;
    eventually("the will to be heard", || async { !h.state().await.discovered["e2e-walk"].online }).await;

    h.become_follower().await;
    eventually("the browse's answer to be the node's", || async {
        let state = h.state().await;
        !state.active && state.discovered["e2e-walk"].online
    })
    .await;
}

// ── Names ─────────────────────────────────────────────────────────────────────

/// What the node calls itself and its ports now, from its own `/info`.
async fn node_names(sim: &SimHandle) -> (String, openhaunt::NodeLabels) {
    let info: serde_json::Value = reqwest::get(format!("http://{}/api/v1/info", loopback(sim.http_addr)))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (info["name"].as_str().unwrap_or_default().to_string(), openhaunt::NodeLabels::from_info(&info))
}

/// Name a parameter in the show, as the Devices panel does: the whole map, one write.
async fn label_in_show(h: &Harness, fixture: Uuid, key: &str, label: &str) {
    let mut labels = h.fixtures().await.into_iter().find(|f| f.id == fixture).unwrap().labels();
    labels.insert(key.to_string(), label.to_string());
    h.engine
        .set(
            vec![
                PathSegment::Key("fixtures".into()),
                PathSegment::Id(fixture),
                PathSegment::Key("parameter_labels".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            serde_json::json!(labels),
        )
        .await
        .unwrap();
}

/// Rename on the node itself, as its own web page would.
async fn rename_on_the_node(sim: &SimHandle, body: serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("http://{}/api/v1/config", loopback(sim.http_addr)))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

async fn show_label(h: &Harness, fixture: Uuid, key: &str) -> Option<String> {
    h.fixtures().await.into_iter().find(|f| f.id == fixture)?.labels().get(key).cloned()
}

#[tokio::test]
async fn a_port_named_in_the_show_is_named_on_the_node() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-label").await;
    let fixture = h.devices.adopt("e2e-label".into()).await.unwrap();

    label_in_show(&h, fixture, "Contact:2", "Ausgang PIR").await;
    eventually("the node to carry the label", || async {
        node_names(&sim).await.1.ports.get(&2).map(String::as_str) == Some("Ausgang PIR")
    })
    .await;
    eventually("the two to have agreed", || async {
        let f = h.fixtures().await.into_iter().find(|f| f.id == fixture).unwrap();
        f.agreed_names.is_some_and(|a| a.parameters.get("Contact:2").map(String::as_str) == Some("Ausgang PIR"))
    })
    .await;
    assert!(h.state().await.discovered["e2e-label"].name_conflicts.is_empty());
    // The label is not the parameter: the key every cue holds is the same one.
    let types = types(&h).await;
    let own = h.fixtures().await.into_iter().find(|f| f.id == fixture).unwrap();
    assert!(type_of(&types, &own).parameters.iter().any(|p| crate::model::playback::parameter_key(&p.kind) == "Contact:2"));
}

#[tokio::test]
async fn a_node_already_named_names_the_show_when_it_is_adopted() {
    let h = harness().await;
    let mut config = SimConfig::new(ModuleKind::DigitalIn, "e2e-named");
    config.node.ports[4].label = Some("Relay Horn".into());
    let sim = openhaunt_node_sim::start(config).await.unwrap();
    h.resolve_at("e2e-named", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;
    let fixture = h.devices.adopt("e2e-named".into()).await.unwrap();
    assert_eq!(show_label(&h, fixture, "Contact:4").await.as_deref(), Some("Relay Horn"));
    assert!(h.state().await.discovered["e2e-named"].name_conflicts.is_empty(), "the show had none, so nothing to ask");
}

#[tokio::test]
async fn every_port_named_on_the_node_is_taken_not_only_the_last() {
    let h = harness().await;
    let mut config = SimConfig::new(ModuleKind::DigitalIn, "e2e-many");
    for (port, label) in [(0, "Donkey"), (1, "Horn"), (4, "Spinnengang"), (5, "Donkey")] {
        config.node.ports[port].label = Some(label.into());
    }
    let sim = openhaunt_node_sim::start(config).await.unwrap();
    h.resolve_at("e2e-many", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;
    let fixture = h.devices.adopt("e2e-many".into()).await.unwrap();

    for (key, label) in [("Contact:0", "Donkey"), ("Contact:1", "Horn"), ("Contact:4", "Spinnengang"), ("Contact:5", "Donkey")] {
        assert_eq!(show_label(&h, fixture, key).await.as_deref(), Some(label), "{key}");
    }
    // And nothing was told to the node to take them back.
    h.resolve_at("e2e-many", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;
    assert_eq!(node_names(&sim).await.1.ports.len(), 4);
    assert!(h.state().await.discovered["e2e-many"].name_conflicts.is_empty());
}

#[tokio::test]
async fn a_name_changed_on_the_node_is_asked_about_and_settled_either_way() {
    use pult_schema::types::devices::{NameChoice, NameSide};

    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-ask").await;
    let fixture = h.devices.adopt("e2e-ask".into()).await.unwrap();
    label_in_show(&h, fixture, "Contact:1", "Tor").await;
    eventually("the node to carry the label", || async {
        node_names(&sim).await.1.ports.get(&1).map(String::as_str) == Some("Tor")
    })
    .await;

    // Somebody renames it on the node's own page; the node announces itself again.
    rename_on_the_node(&sim, serde_json::json!({ "labels": { "ports": { "1": "Gate" } } })).await;
    h.resolve_at("e2e-ask", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;
    let conflicts = h.state().await.discovered["e2e-ask"].name_conflicts.clone();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!((conflicts[0].port, conflicts[0].node.as_str(), conflicts[0].show.as_str()), (Some(1), "Gate", "Tor"));
    assert_eq!(node_names(&sim).await.1.ports[&1], "Gate", "nothing is written either way before somebody picks");
    assert_eq!(show_label(&h, fixture, "Contact:1").await.as_deref(), Some("Tor"));

    // The show's.
    h.devices
        .settle_names("e2e-ask".into(), vec![NameChoice { head: 0, port: Some(1), take: NameSide::Show }])
        .await
        .unwrap();
    assert_eq!(node_names(&sim).await.1.ports[&1], "Tor");
    assert!(h.state().await.discovered["e2e-ask"].name_conflicts.is_empty());

    // And again, the node's.
    rename_on_the_node(&sim, serde_json::json!({ "labels": { "ports": { "1": "Gate" } } })).await;
    h.resolve_at("e2e-ask", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;
    h.devices
        .settle_names("e2e-ask".into(), vec![NameChoice { head: 0, port: Some(1), take: NameSide::Node }])
        .await
        .unwrap();
    assert_eq!(show_label(&h, fixture, "Contact:1").await.as_deref(), Some("Gate"));
    assert!(h.state().await.discovered["e2e-ask"].name_conflicts.is_empty());
}

#[tokio::test]
async fn a_name_given_while_the_node_was_off_is_passed_on_without_asking() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-off").await;
    let fixture = h.devices.adopt("e2e-off".into()).await.unwrap();
    label_in_show(&h, fixture, "Contact:0", "Erst").await;
    eventually("the first label to arrive", || async {
        node_names(&sim).await.1.ports.get(&0).map(String::as_str) == Some("Erst")
    })
    .await;
    eventually("the two to have agreed", || async {
        let f = h.fixtures().await.into_iter().find(|f| f.id == fixture).unwrap();
        f.agreed_names.is_some_and(|a| a.parameters.get("Contact:0").map(String::as_str) == Some("Erst"))
    })
    .await;

    // Off: gone from the network, as mDNS says. Not left to its will alone, which a
    // loaded machine heard after the five seconds `eventually` waits.
    let port = sim.http_addr.port();
    sim.stop.stop().await;
    h.event(super::DeviceEvent::Removed { serial: "e2e-off".into() }).await;
    eventually("the node to be off", || async { !h.state().await.discovered["e2e-off"].online }).await;

    label_in_show(&h, fixture, "Contact:0", "Später").await;
    // And the node's own name, which follows the same rule.
    h.engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture), PathSegment::Key("name".into())],
            pult_schema::lifecycle::Lifecycle::Persisted,
            serde_json::json!("Hof"),
        )
        .await
        .unwrap();

    // Back, on the same address, saying what it said before it went.
    let mut config = SimConfig::new(ModuleKind::DigitalIn, "e2e-off");
    config.node.http_port = port;
    config.node.name = "Node e2e-off".into();
    config.node.ports[0].label = Some("Erst".into());
    let sim = openhaunt_node_sim::start(config).await.unwrap();
    h.resolve_at("e2e-off", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;
    let (name, labels) = node_names(&sim).await;
    assert_eq!(labels.ports[&0], "Später");
    assert_eq!(name, "Hof");
    assert!(h.state().await.discovered["e2e-off"].name_conflicts.is_empty());
}

#[tokio::test]
async fn a_node_that_was_reset_is_asked_about_rather_than_overwritten() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-reset").await;
    let fixture = h.devices.adopt("e2e-reset".into()).await.unwrap();
    label_in_show(&h, fixture, "Contact:6", "Nebel").await;
    eventually("the two to have agreed", || async {
        let f = h.fixtures().await.into_iter().find(|f| f.id == fixture).unwrap();
        f.agreed_names.is_some_and(|a| a.parameters.get("Contact:6").map(String::as_str) == Some("Nebel"))
    })
    .await;

    // Reset at the bench: it comes back knowing nothing.
    let port = sim.http_addr.port();
    sim.stop.stop().await;
    h.event(super::DeviceEvent::Removed { serial: "e2e-reset".into() }).await;
    eventually("the node to be off", || async { !h.state().await.discovered["e2e-reset"].online }).await;
    let mut config = SimConfig::new(ModuleKind::DigitalIn, "e2e-reset");
    config.node.http_port = port;
    let sim = openhaunt_node_sim::start(config).await.unwrap();
    h.resolve_at("e2e-reset", ModuleKind::DigitalIn.type_id(), loopback(sim.http_addr)).await;

    let conflicts = h.state().await.discovered["e2e-reset"].name_conflicts.clone();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!((conflicts[0].node.as_str(), conflicts[0].show.as_str()), ("", "Nebel"));
    assert!(!node_names(&sim).await.1.ports.contains_key(&6), "not written before somebody picks");
}

#[tokio::test]
async fn a_new_node_takes_the_place_of_one_that_is_gone() {
    let h = harness().await;
    let old = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-old").await;
    let fixture = h.devices.adopt("e2e-old".into()).await.unwrap();
    label_in_show(&h, fixture, "Contact:3", "Ausgang PIR").await;
    old.stop.stop().await;
    h.event(super::DeviceEvent::Removed { serial: "e2e-old".into() }).await;

    let new = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-new").await;
    let comparison = h.devices.compare("e2e-new".into(), "e2e-old".into()).await.unwrap();
    assert!(!comparison.module_differs && comparison.missing.is_empty(), "{comparison:?}");

    let names = crate::infra::devices::AdoptNames {
        name: Some("Hof links".into()),
        ports: [(3, "Ausgang PIR".to_string()), (5, "Relay Horn".to_string())].into(),
        ..Default::default()
    };
    let id = h.devices.adopt_named("e2e-new".into(), Some("e2e-old".into()), Some(names)).await.unwrap();
    assert_eq!(id, fixture, "the same fixture, so every cue that names it drives the new node");

    let own = h.fixtures().await.into_iter().find(|f| f.id == fixture).unwrap();
    assert_eq!(own.address.serial(), Some("e2e-new"));
    assert_eq!(own.name, "Hof links");
    assert_eq!(own.labels().get("Contact:5").map(String::as_str), Some("Relay Horn"));
    let state = h.state().await;
    assert!(!state.discovered.contains_key("e2e-old"), "the old node is not coming back as itself");
    assert!(state.discovered["e2e-new"].name_conflicts.is_empty());

    let (name, labels) = node_names(&new).await;
    assert_eq!(name, "Hof links");
    assert_eq!(labels.ports[&3], "Ausgang PIR");
    assert_eq!(labels.ports[&5], "Relay Horn");
}

#[tokio::test]
async fn a_replacement_with_other_ports_says_what_it_leaves_behind() {
    let h = harness().await;
    let old = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-was").await;
    let fixture = h.devices.adopt("e2e-was".into()).await.unwrap();
    label_in_show(&h, fixture, "Contact:7", "Klingel").await;
    old.stop.stop().await;
    h.event(super::DeviceEvent::Removed { serial: "e2e-was".into() }).await;

    let _new = a_simulated_node(&h, ModuleKind::Environment, "e2e-is").await;
    let comparison = h.devices.compare("e2e-is".into(), "e2e-was".into()).await.unwrap();
    assert!(comparison.module_differs);
    assert_eq!(comparison.missing.len(), 8, "every contact: {comparison:?}");
    assert!(comparison.missing.iter().any(|m| m.label == "Klingel"), "under the name it was given");
}

#[tokio::test]
async fn a_strip_named_on_the_node_is_called_that_in_the_show() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::NodeV0, "e2e-strip-name").await;
    rename_on_the_node(&sim, serde_json::json!({ "labels": { "heads": { "1": "LED Streifen Tor" } } })).await;
    h.resolve_at("e2e-strip-name", ModuleKind::NodeV0.type_id(), loopback(sim.http_addr)).await;
    h.devices.adopt("e2e-strip-name".into()).await.unwrap();
    let strip = h.fixtures().await.into_iter().find(|f| f.id == openhaunt::head_fixture_id("e2e-strip-name", 1)).unwrap();
    assert_eq!(strip.name, "LED Streifen Tor");

    // Renamed in the show, it is renamed on the node.
    h.engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Id(strip.id), PathSegment::Key("name".into())],
            pult_schema::lifecycle::Lifecycle::Persisted,
            serde_json::json!("Streifen Hof"),
        )
        .await
        .unwrap();
    eventually("the head's label to follow", || async {
        node_names(&sim).await.1.heads.get(&1).map(String::as_str) == Some("Streifen Hof")
    })
    .await;
}
