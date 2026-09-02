use std::collections::HashMap;

use pult_schema::types::{
    fixture::{
        FixtureType, ParameterDefinition, ParameterKind,
    },
    openhaunt as modules,
    openhaunt::{EffectCapability, PortEffectCapability},
};

use super::*;
use crate::infra::connectors::dmx::holding;
use crate::infra::devices::{DeviceCommand, DeviceEntry};

// ── Fixtures ──────────────────────────────────────────────────────────────────

const DMX_OUT: u16 = 0x0001;
const DIGITAL_IN: u16 = 0x0002;
const WS2812: u16 = 0x0003;
const MAINS_RELAY: u16 = 0x0004;
const OLED: u16 = 0x0005;

/// The fixture type a module becomes, built the way adoption builds one: from the
/// description the node itself served. The descriptions are written here as the
/// device's words — there is no table in the console to look one up in.
fn a_module(module_type: u16) -> FixtureType {
    let (name, description) = match module_type {
        DMX_OUT => (
            "DMX Gateway",
            serde_json::json!({ "ports": [], "dmx": { "protocols": ["sacn"], "universes": 1 } }),
        ),
        DIGITAL_IN => (
            "Digital Inputs",
            serde_json::json!({ "ports": (0..8).map(|n| serde_json::json!({
                "port": n, "name": format!("Input {}", n + 1),
                "access": "readonly", "dataType": "boolean", "class": "contact",
            })).collect::<Vec<_>>() }),
        ),
        WS2812 => (
            "LED Strip",
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Strip colour", "access": "readwrite",
                  "dataType": "color", "class": "color" },
                { "port": 1, "name": "Brightness", "access": "readwrite", "dataType": "number",
                  "unit": "percent", "minimum": 0, "maximum": 1, "default": 0,
                  "class": "intensity" },
            ]}),
        ),
        MAINS_RELAY => (
            "Mains Relay",
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Relay", "access": "readwrite",
                  "dataType": "boolean", "default": 0, "class": "switch" },
            ]}),
        ),
        OLED => (
            "Display",
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Line", "access": "readwrite",
                  "dataType": "string", "class": "text" },
            ]}),
        ),
        other => panic!("no description written for module {other:#06x}"),
    };
    modules::fixture_type_from(
        module_type,
        name,
        &serde_json::from_value(description).expect("a description parses"),
    )
}

/// A device manager that records what it is told to send, rather than sending it.
fn a_recording_device_handle() -> (DeviceHandle, tokio::sync::mpsc::Receiver<DeviceCommand>) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    (DeviceHandle(tx), rx)
}

fn directory(entries: Vec<(&str, DeviceEntry)>) -> watch::Receiver<DeviceDirectory> {
    let (tx, rx) = watch::channel(DeviceDirectory {
        entries: entries.into_iter().map(|(s, e)| (s.to_string(), e)).collect(),
    });
    // Held for the life of the test, so the receiver never sees a closed channel.
    std::mem::forget(tx);
    rx
}

fn an_entry(ip: &str, module_type: u16, universe: Option<u16>, online: bool) -> DeviceEntry {
    DeviceEntry { ip: ip.to_string(), port: 80, module_type, universe, online, effects: None }
}

/// The same node, with one port that says what it can trace for itself.
fn a_capable_entry(
    ip: &str,
    module_type: u16,
    port: u8,
    shapes: &[&str],
    transitions: bool,
) -> DeviceEntry {
    DeviceEntry {
        effects: Some(EffectCapability {
            ports: vec![PortEffectCapability {
                port,
                shapes: shapes.iter().map(|s| s.to_string()).collect(),
                steps: true,
                transitions,
            }],
        }),
        ..an_entry(ip, module_type, None, true)
    }
}

fn a_node_fixture(fixture_type: &FixtureType, serial: &str, universe: Option<u16>) -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: serial.into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::OpenHaunt { serial: serial.into(), universe },
        position: None,
        sensed_values: HashMap::new(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    }
}

fn a_dmx_dimmer(universe: u16, level: f32) -> (Fixture, FixtureType) {
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
        address: FixtureAddress::dmx(universe, 1),
        position: None,
        sensed_values: HashMap::new(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    };
    holding(&mut fixture, "Intensity", ParameterValue::Float(level));
    (fixture, fixture_type)
}

fn patch(fixtures: Vec<Fixture>, types: Vec<FixtureType>) -> Patch {
    Patch::new(fixtures, types, vec![])
}

async fn a_gateway_socket() -> (UdpSocket, u16) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, port)
}

/// Everything the plugin sent to the device manager, drained without blocking.
fn sent(rx: &mut tokio::sync::mpsc::Receiver<DeviceCommand>) -> Vec<(String, u8, serde_json::Value)> {
    let mut out = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let DeviceCommand::SetOutput { serial, port, value } = cmd {
            out.push((serial, port, value));
        }
    }
    out
}

// ── Ports ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_relay_is_commanded_when_it_changes_and_not_before() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("relay1", an_entry("127.0.0.1", MAINS_RELAY, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(MAINS_RELAY);
    let mut fixture = a_node_fixture(&ft, "relay1", None);
    holding(&mut fixture, "Switch:0", ParameterValue::Bool(true));

    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    assert_eq!(
        sent(&mut received),
        vec![("relay1".to_string(), 0, serde_json::json!({ "state": true }))],
    );

    // The same value again is not news.
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    assert!(sent(&mut received).is_empty(), "a relay must not be commanded 40 times a second");

    holding(&mut fixture, "Switch:0", ParameterValue::Bool(false));
    output.send(&patch(vec![fixture], vec![ft]), &[], 0).await.unwrap();
    assert_eq!(
        sent(&mut received),
        vec![("relay1".to_string(), 0, serde_json::json!({ "state": false }))],
    );
}

#[tokio::test]
async fn a_strip_sends_its_colour_as_bytes_and_its_brightness_as_a_number() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("led1", an_entry("127.0.0.1", WS2812, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "led1", None);
    holding(&mut fixture, "ColorRgb", ParameterValue::rgb(1.0, 0.5, 0.0));
    holding(&mut fixture, "Intensity", ParameterValue::Float(0.5));

    output.send(&patch(vec![fixture], vec![ft]), &[], 0).await.unwrap();

    let messages = sent(&mut received);
    assert_eq!(messages[0], ("led1".into(), 0, serde_json::json!({ "r": 255, "g": 128, "b": 0 })));
    assert_eq!(messages[1], ("led1".into(), 1, serde_json::json!({ "value": 0.5 })));
}

#[tokio::test]
async fn a_display_sends_text() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("oled1", an_entry("127.0.0.1", OLED, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(OLED);
    let mut fixture = a_node_fixture(&ft, "oled1", None);
    holding(&mut fixture, "Text", ParameterValue::Text("ACT ONE".into()));

    output.send(&patch(vec![fixture], vec![ft]), &[], 0).await.unwrap();

    assert_eq!(
        sent(&mut received),
        vec![("oled1".to_string(), 0, serde_json::json!({ "text": "ACT ONE" }))],
    );
}

#[tokio::test]
async fn a_device_that_is_offline_is_not_commanded() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("relay1", an_entry("127.0.0.1", MAINS_RELAY, None, false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(MAINS_RELAY);
    let mut fixture = a_node_fixture(&ft, "relay1", None);
    holding(&mut fixture, "Switch:0", ParameterValue::Bool(true));

    output.send(&patch(vec![fixture], vec![ft]), &[], 0).await.unwrap();

    assert!(sent(&mut received).is_empty());
}

#[tokio::test]
async fn an_input_parameter_is_never_sent_back_to_the_device_that_reported_it() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("in1", an_entry("127.0.0.1", DIGITAL_IN, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(DIGITAL_IN);
    let mut fixture = a_node_fixture(&ft, "in1", None);
    holding(&mut fixture, "Contact:0", ParameterValue::Bool(true));

    output.send(&patch(vec![fixture], vec![ft]), &[], 0).await.unwrap();

    assert!(sent(&mut received).is_empty(), "an input module has nothing to drive");
}

#[tokio::test]
async fn unpatching_a_device_forgets_what_was_last_sent_to_it() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("relay1", an_entry("127.0.0.1", MAINS_RELAY, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(MAINS_RELAY);
    let mut fixture = a_node_fixture(&ft, "relay1", None);
    holding(&mut fixture, "Switch:0", ParameterValue::Bool(true));
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    let _ = sent(&mut received);

    // Unpatched, then patched again with the same value.
    let other = a_module(OLED);
    let placeholder = a_node_fixture(&other, "oled1", None);
    output.send(&patch(vec![placeholder], vec![other]), &[], 0).await.unwrap();
    let _ = sent(&mut received);

    output.send(&patch(vec![fixture], vec![ft]), &[], 0).await.unwrap();

    assert_eq!(
        sent(&mut received).len(),
        1,
        "a re-adopted relay has to be told where it stands, not compared against a memory",
    );
}

// ── Gateways ──────────────────────────────────────────────────────────────────

async fn recv(socket: &UdpSocket) -> Vec<u8> {
    let mut buffer = vec![0u8; 2048];
    let n = tokio::time::timeout(std::time::Duration::from_secs(1), socket.recv(&mut buffer))
        .await
        .expect("a packet within a second")
        .unwrap();
    buffer.truncate(n);
    buffer
}

#[tokio::test]
async fn a_gateway_receives_the_universe_it_was_adopted_onto() {
    let (socket, port) = a_gateway_socket().await;
    let (devices, _received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![(
            "gate1",
            an_entry("127.0.0.1", DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    let gateway_type = a_module(DMX_OUT);
    let gateway = a_node_fixture(&gateway_type, "gate1", Some(5));
    let (dimmer, dimmer_type) = a_dmx_dimmer(5, 1.0);

    output
        .send(&patch(vec![gateway, dimmer], vec![gateway_type, dimmer_type]), &[], 0)
        .await
        .unwrap();

    let packet = recv(&socket).await;
    assert_eq!(packet.len(), 638, "an E1.31 data packet");
    assert_eq!(&packet[113..115], &[0x00, 0x05], "universe 5");
    assert_eq!(packet[126], 255, "the dimmer at full");
}

#[tokio::test]
async fn a_gateway_hears_nothing_about_a_universe_it_is_not_on() {
    let (socket, port) = a_gateway_socket().await;
    let (devices, _received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![(
            "gate1",
            an_entry("127.0.0.1", DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    let gateway_type = a_module(DMX_OUT);
    let gateway = a_node_fixture(&gateway_type, "gate1", Some(5));
    let (dimmer, dimmer_type) = a_dmx_dimmer(9, 1.0);

    output
        .send(&patch(vec![gateway, dimmer], vec![gateway_type, dimmer_type]), &[], 0)
        .await
        .unwrap();

    let anything =
        tokio::time::timeout(std::time::Duration::from_millis(100), recv(&socket)).await;
    assert!(anything.is_err(), "universe 9 is not this gateway's business");
}

#[tokio::test]
async fn an_unchanged_universe_is_not_resent_to_a_gateway() {
    let (socket, port) = a_gateway_socket().await;
    let (devices, _received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![(
            "gate1",
            an_entry("127.0.0.1", DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    let gateway_type = a_module(DMX_OUT);
    let gateway = a_node_fixture(&gateway_type, "gate1", Some(5));
    let (dimmer, dimmer_type) = a_dmx_dimmer(5, 1.0);
    let frame = || patch(vec![gateway.clone(), dimmer.clone()], vec![gateway_type.clone(), dimmer_type.clone()]);

    output.send(&frame(), &[], 0).await.unwrap();
    let _ = recv(&socket).await;
    output.send(&frame(), &[], 0).await.unwrap();

    let again = tokio::time::timeout(std::time::Duration::from_millis(100), recv(&socket)).await;
    assert!(again.is_err());
}

#[tokio::test]
async fn a_console_with_no_node_fixtures_sends_nothing_at_all() {
    let (socket, port) = a_gateway_socket().await;
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![(
            "gate1",
            an_entry("127.0.0.1", DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    // A discovered gateway that nobody adopted, and an ordinary DMX rig.
    let (dimmer, dimmer_type) = a_dmx_dimmer(5, 1.0);
    output.send(&patch(vec![dimmer], vec![dimmer_type]), &[], 0).await.unwrap();

    assert!(sent(&mut received).is_empty());
    let anything =
        tokio::time::timeout(std::time::Duration::from_millis(100), recv(&socket)).await;
    assert!(anything.is_err(), "a discovered device is not an adopted one");
}

// ── Offload ───────────────────────────────────────────────────────────────────
//
// The point of all of this is a message count. A three second fade at 40 Hz is a
// hundred and twenty messages to a node that could have been told "go there over
// three seconds" once, and a chase never stops producing them at all. So most of
// these tests assert what was *not* sent.

use pult_schema::types::effect::{
    Curve, Direction, Easing, EffectSource, RunningEffect, RunningFade, Shape,
};

/// Everything the plugin sent, of both kinds, in order.
///
/// One drain, because the channel is consumed by reading it: asking for the values
/// and then for the shapes throws the values away and reports that none were sent,
/// which is a convincing-looking green as often as it is a convincing-looking red.
fn everything(
    rx: &mut tokio::sync::mpsc::Receiver<DeviceCommand>,
) -> Vec<(&'static str, String, u8, Option<serde_json::Value>)> {
    let mut out = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            DeviceCommand::SetOutput { serial, port, value } => {
                out.push(("set", serial, port, Some(value)))
            }
            DeviceCommand::SetEffect { serial, port, payload } => {
                out.push(("effect", serial, port, payload))
            }
            _ => {}
        }
    }
    out
}

type Message = (&'static str, String, u8, Option<serde_json::Value>);

fn of_kind(messages: &[Message], kind: &str, port: u8) -> Vec<Message> {
    messages.iter().filter(|m| m.0 == kind && m.2 == port).cloned().collect()
}

fn a_running_sine(t0: u64) -> RunningEffect {
    RunningEffect {
        effect_id: Uuid::nil(),
        curve: Curve::Shape(Shape::Sine),
        rate_hz: 0.5,
        low: ParameterValue::Float(0.0),
        high: ParameterValue::Float(1.0),
        width: 0.5,
        direction: Direction::Forward,
        phase: 0.25,
        t0,
        source: EffectSource::Programmer,
    }
}

/// The whole bargain: the value moves on every pass and the node hears about it once.
#[tokio::test]
async fn a_capable_port_is_handed_the_shape_once_and_then_left_alone() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 1, &["sine"], false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_effects.insert("Intensity".into(), a_running_sine(1_000));

    // Three passes, with the rendered value somewhere different on each.
    for level in [0.5f32, 0.9, 0.1] {
        holding(&mut fixture, "Intensity", ParameterValue::Float(level));
        output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    }

    let messages = everything(&mut received);
    let effects = of_kind(&messages, "effect", 1);
    assert_eq!(effects.len(), 1, "described once, not once a pass: {effects:?}");
    assert_eq!(effects[0].1, "strip1");
    let payload = effects[0].3.as_ref().expect("a description, not a clear");
    assert_eq!(payload["curve"], serde_json::json!({ "shape": "sine" }));
    assert_eq!(payload["rate"], 0.5);
    assert_eq!(payload["phase"], 0.25);
    assert_eq!(payload["t0"], 1_000);
    assert_eq!(payload["low"], serde_json::json!({ "value": 0.0 }));

    assert!(
        of_kind(&messages, "set", 1).is_empty(),
        "and the port is sent no values at all while it is tracing",
    );
}

/// A port that never said it could trace a sine is not sent one. Nothing is
/// negotiated: silence in `/info` means the old behaviour, exactly as before.
#[tokio::test]
async fn a_port_that_says_nothing_gets_values_as_it_always_did() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", an_entry("127.0.0.1", WS2812, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    let sine = a_running_sine(1_000);
    fixture.live_effects.insert("Intensity".into(), sine.clone());

    // Half a hertz with a quarter-cycle phase, so half a second past its anchor is
    // half a cycle in from the top: the bottom of the range.
    let now_ms = 2_000;
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], now_ms).await.unwrap();

    let expected = match pult_render::effect_value_at(&sine, now_ms) {
        ParameterValue::Float(level) => level,
        other => panic!("expected a level, got {other:?}"),
    };
    let messages = everything(&mut received);
    assert!(of_kind(&messages, "effect", 1).is_empty(), "no description");
    let sent = of_kind(&messages, "set", 1);
    let value = sent[0].3.as_ref().unwrap()["value"].as_f64().unwrap() as f32;
    assert!(
        (value - expected).abs() < 1e-6,
        "the value the console worked out for this moment, as ever: {value} vs {expected}",
    );
}

/// A port that can chop a square wave still cannot trace a sine, and saying so per
/// shape is what stops the console handing it one it would ignore in silence.
#[tokio::test]
async fn a_shape_the_port_did_not_list_falls_back_to_values() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 1, &["square"], false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_effects.insert("Intensity".into(), a_running_sine(1_000));
    holding(&mut fixture, "Intensity", ParameterValue::Float(0.5));

    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();

    let messages = everything(&mut received);
    assert!(of_kind(&messages, "effect", 1).is_empty(), "a sine is not a square");
    assert!(!of_kind(&messages, "set", 1).is_empty(), "so it gets values");
}

/// Taking a light out of a chase has to reach the node as two messages in order:
/// stop tracing, then here is a value. Either one alone leaves it wrong.
#[tokio::test]
async fn clearing_an_effect_stops_the_node_and_then_gives_it_a_value() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 1, &["sine"], false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_effects.insert("Intensity".into(), a_running_sine(1_000));
    holding(&mut fixture, "Intensity", ParameterValue::Float(0.5));
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    let _ = everything(&mut received);

    // The operator grabs the fader: nothing periodic any more, and the value it
    // happens to land on is the one it was already rendering.
    fixture.live_effects.clear();
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();

    let all = everything(&mut received);
    let messages: Vec<_> = all.iter().filter(|m| m.2 == 1).collect();
    assert_eq!(messages.len(), 2, "exactly two: {messages:?}");
    assert_eq!(messages[0].0, "effect");
    assert_eq!(messages[0].3, None, "stop tracing");
    assert_eq!(messages[1].0, "set");
    assert_eq!(
        messages[1].3.as_ref().unwrap()["value"],
        0.5,
        "and the value goes out even though it is the same one the plugin last recorded",
    );
}

/// Tapping a speed master changes the rate and the anchor, and both are in the
/// description, so the same dedup that kept the node quiet now speaks up.
#[tokio::test]
async fn a_change_of_tempo_sends_one_more_description() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 1, &["sine"], false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_effects.insert("Intensity".into(), a_running_sine(1_000));
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    assert_eq!(of_kind(&everything(&mut received), "effect", 1).len(), 1);

    fixture
        .live_effects
        .insert("Intensity".into(), RunningEffect { rate_hz: 2.0, t0: 9_000, ..a_running_sine(1_000) });
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();

    let again = of_kind(&everything(&mut received), "effect", 1);
    assert_eq!(again.len(), 1, "one more, not one a pass");
    let payload = again[0].3.as_ref().unwrap();
    assert_eq!(payload["rate"], 2.0);
    assert_eq!(payload["t0"], 9_000);
}

/// A step chase goes out as its keyframes, in the port's own payload shape, so a
/// node parses each value with the code it already has for a `set`.
#[tokio::test]
async fn a_step_chase_goes_out_as_its_keyframes() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 0, &[], false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_effects.insert(
        "ColorRgb".into(),
        RunningEffect {
            curve: Curve::Steps(vec![
                pult_schema::types::effect::Step {
                    at: 0.0,
                    value: ParameterValue::rgb(1.0, 0.0, 0.0),
                    easing: Easing::Step,
                },
                pult_schema::types::effect::Step {
                    at: 0.5,
                    value: ParameterValue::rgb(0.0, 1.0, 0.0),
                    easing: Easing::Linear,
                },
            ]),
            ..a_running_sine(0)
        },
    );

    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();

    let effects = of_kind(&everything(&mut received), "effect", 0);
    assert_eq!(effects.len(), 1);
    let steps = &effects[0].3.as_ref().unwrap()["curve"]["steps"];
    assert_eq!(steps[0]["value"], serde_json::json!({ "r": 255, "g": 0, "b": 0 }));
    assert_eq!(steps[0]["easing"], "step");
    assert_eq!(steps[1]["at"], 0.5);
    assert_eq!(steps[1]["easing"], "linear");
}

// ── Transitions ───────────────────────────────────────────────────────────────

/// A three second fade was a hundred and twenty messages. It is one.
#[tokio::test]
async fn a_fade_on_a_capable_port_is_one_timed_set_and_nothing_after_it() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 1, &["sine"], true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_fades.insert(
        "Intensity".into(),
        RunningFade {
            from: ParameterValue::Float(0.0),
            to: ParameterValue::Float(1.0),
            t0: 5_000,
            duration_ms: 3_000,
            easing: Easing::EaseInOut,
            cue_id: Uuid::nil(),
        },
    );

    // Three frames part way through the fade, at three different moments — so the
    // value the console would work out has plainly moved between them.
    for now_ms in [5_500u64, 6_500, 7_500] {
        output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], now_ms).await.unwrap();
    }

    let messages = of_kind(&everything(&mut received), "set", 1);
    assert_eq!(messages.len(), 1, "one timed set, no samples: {messages:?}");
    let payload = messages[0].3.as_ref().unwrap();
    assert_eq!(payload["value"], 1.0, "the destination, not where it is now");
    assert_eq!(payload["fade_ms"], 3_000);
    assert_eq!(payload["t0"], 5_000);
    assert_eq!(payload["curve"], "ease-in-out");

    // The fade lands, and stays: it is the record of where the parameter got to. Its
    // description has not changed, and where it is going is where the node already is,
    // so there is nothing left to say.
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 9_000).await.unwrap();

    assert!(
        of_kind(&everything(&mut received), "set", 1).is_empty(),
        "and nothing at all when it arrives",
    );
}

/// A port that did not advertise `transitions` gets the fade interpolated for it,
/// sample by sample, exactly as before.
#[tokio::test]
async fn a_fade_on_a_port_that_cannot_time_one_is_still_interpolated_here() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("strip1", a_capable_entry("127.0.0.1", WS2812, 1, &["sine"], false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_fades.insert(
        "Intensity".into(),
        RunningFade {
            from: ParameterValue::Float(0.0),
            to: ParameterValue::Float(1.0),
            t0: 5_000,
            duration_ms: 3_000,
            easing: Easing::Linear,
            cue_id: Uuid::nil(),
        },
    );

    // Three frames at three moments. The console works the fade out for each of them
    // and sends what it worked out, which is what a port that cannot time one needs.
    for now_ms in [5_500u64, 6_500, 7_500] {
        output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], now_ms).await.unwrap();
    }

    let messages = of_kind(&everything(&mut received), "set", 1);
    assert_eq!(messages.len(), 3, "one per frame, as ever");
    assert!(
        messages.iter().all(|m| m.3.as_ref().unwrap().get("fade_ms").is_none()),
        "and untimed",
    );
}

/// A node that reboots is at its defaults and has forgotten the shape it was
/// tracing. The console has to say it again, and the existing offline-to-online
/// rule is what notices.
#[tokio::test]
async fn a_node_that_reboots_is_told_the_shape_again() {
    let (devices, mut received) = a_recording_device_handle();
    let (tx, directory_rx) = watch::channel(DeviceDirectory {
        entries: [("strip1".to_string(), a_capable_entry("127.0.0.1", WS2812, 1, &["sine"], false))]
            .into_iter()
            .collect(),
    });
    let mut output = OpenHauntOutput::new(directory_rx, devices, 5568).await.unwrap();

    let ft = a_module(WS2812);
    let mut fixture = a_node_fixture(&ft, "strip1", None);
    fixture.live_effects.insert("Intensity".into(), a_running_sine(1_000));
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    assert_eq!(of_kind(&everything(&mut received), "effect", 1).len(), 1, "said once");

    // Away and back.
    tx.send_modify(|d| d.entries.get_mut("strip1").unwrap().online = false);
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();
    tx.send_modify(|d| d.entries.get_mut("strip1").unwrap().online = true);
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[], 0).await.unwrap();

    assert_eq!(
        of_kind(&everything(&mut received), "effect", 1).len(),
        1,
        "and said again after the reboot",
    );
}
