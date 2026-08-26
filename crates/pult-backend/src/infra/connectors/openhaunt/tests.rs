use std::collections::HashMap;

use pult_schema::types::{
    fixture::{
        FixtureType, ParameterBinding, ParameterDefinition, ParameterKind,
    },
    openhaunt as modules,
};

use super::*;
use crate::infra::devices::{DeviceCommand, DeviceEntry};

// ── Fixtures ──────────────────────────────────────────────────────────────────

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
    DeviceEntry { ip: ip.to_string(), port: 80, module_type, universe, online }
}

fn a_node_fixture(fixture_type: &FixtureType, serial: &str, universe: Option<u16>) -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: serial.into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::OpenHaunt { serial: serial.into(), universe },
        position: None,
        live_values: HashMap::new(),
        active_preset: None,
    }
}

fn a_dmx_dimmer(universe: u16, level: f32) -> (Fixture, FixtureType) {
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
        address: FixtureAddress::Dmx { universe, address: 1 },
        position: None,
        live_values: HashMap::new(),
        active_preset: None,
    };
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(level));
    (fixture, fixture_type)
}

fn patch(fixtures: Vec<Fixture>, types: Vec<FixtureType>) -> Patch {
    Patch { fixtures, fixture_types: types.into_iter().map(|t| (t.id, t)).collect() }
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
        directory(vec![("relay1", an_entry("127.0.0.1", modules::MODULE_TYPE_MAINS_RELAY, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = modules::builtin_fixture_type(modules::MODULE_TYPE_MAINS_RELAY).unwrap();
    let mut fixture = a_node_fixture(&ft, "relay1", None);
    fixture.live_values.insert("Switch:0".into(), ParameterValue::Bool(true));

    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[]).await.unwrap();
    assert_eq!(
        sent(&mut received),
        vec![("relay1".to_string(), 0, serde_json::json!({ "state": true }))],
    );

    // The same value again is not news.
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[]).await.unwrap();
    assert!(sent(&mut received).is_empty(), "a relay must not be commanded 40 times a second");

    fixture.live_values.insert("Switch:0".into(), ParameterValue::Bool(false));
    output.send(&patch(vec![fixture], vec![ft]), &[]).await.unwrap();
    assert_eq!(
        sent(&mut received),
        vec![("relay1".to_string(), 0, serde_json::json!({ "state": false }))],
    );
}

#[tokio::test]
async fn a_strip_sends_its_colour_as_bytes_and_its_brightness_as_a_level() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("led1", an_entry("127.0.0.1", modules::MODULE_TYPE_WS2812, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = modules::builtin_fixture_type(modules::MODULE_TYPE_WS2812).unwrap();
    let mut fixture = a_node_fixture(&ft, "led1", None);
    fixture.live_values.insert("ColorRgb".into(), ParameterValue::Color { r: 1.0, g: 0.5, b: 0.0 });
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(0.5));

    output.send(&patch(vec![fixture], vec![ft]), &[]).await.unwrap();

    let messages = sent(&mut received);
    assert_eq!(messages[0], ("led1".into(), 0, serde_json::json!({ "r": 255, "g": 128, "b": 0 })));
    assert_eq!(messages[1], ("led1".into(), 1, serde_json::json!({ "brightness": 0.5 })));
}

#[tokio::test]
async fn a_display_sends_text() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("oled1", an_entry("127.0.0.1", modules::MODULE_TYPE_OLED, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = modules::builtin_fixture_type(modules::MODULE_TYPE_OLED).unwrap();
    let mut fixture = a_node_fixture(&ft, "oled1", None);
    fixture.live_values.insert("Text".into(), ParameterValue::Text("ACT ONE".into()));

    output.send(&patch(vec![fixture], vec![ft]), &[]).await.unwrap();

    assert_eq!(
        sent(&mut received),
        vec![("oled1".to_string(), 0, serde_json::json!({ "text": "ACT ONE" }))],
    );
}

#[tokio::test]
async fn a_device_that_is_offline_is_not_commanded() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("relay1", an_entry("127.0.0.1", modules::MODULE_TYPE_MAINS_RELAY, None, false))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = modules::builtin_fixture_type(modules::MODULE_TYPE_MAINS_RELAY).unwrap();
    let mut fixture = a_node_fixture(&ft, "relay1", None);
    fixture.live_values.insert("Switch:0".into(), ParameterValue::Bool(true));

    output.send(&patch(vec![fixture], vec![ft]), &[]).await.unwrap();

    assert!(sent(&mut received).is_empty());
}

#[tokio::test]
async fn an_input_parameter_is_never_sent_back_to_the_device_that_reported_it() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("in1", an_entry("127.0.0.1", modules::MODULE_TYPE_DIGITAL_IN, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = modules::builtin_fixture_type(modules::MODULE_TYPE_DIGITAL_IN).unwrap();
    let mut fixture = a_node_fixture(&ft, "in1", None);
    fixture.live_values.insert("Contact:0".into(), ParameterValue::Bool(true));

    output.send(&patch(vec![fixture], vec![ft]), &[]).await.unwrap();

    assert!(sent(&mut received).is_empty(), "an input module has nothing to drive");
}

#[tokio::test]
async fn unpatching_a_device_forgets_what_was_last_sent_to_it() {
    let (devices, mut received) = a_recording_device_handle();
    let mut output = OpenHauntOutput::new(
        directory(vec![("relay1", an_entry("127.0.0.1", modules::MODULE_TYPE_MAINS_RELAY, None, true))]),
        devices,
        5568,
    )
    .await
    .unwrap();

    let ft = modules::builtin_fixture_type(modules::MODULE_TYPE_MAINS_RELAY).unwrap();
    let mut fixture = a_node_fixture(&ft, "relay1", None);
    fixture.live_values.insert("Switch:0".into(), ParameterValue::Bool(true));
    output.send(&patch(vec![fixture.clone()], vec![ft.clone()]), &[]).await.unwrap();
    let _ = sent(&mut received);

    // Unpatched, then patched again with the same value.
    let other = modules::builtin_fixture_type(modules::MODULE_TYPE_OLED).unwrap();
    let placeholder = a_node_fixture(&other, "oled1", None);
    output.send(&patch(vec![placeholder], vec![other]), &[]).await.unwrap();
    let _ = sent(&mut received);

    output.send(&patch(vec![fixture], vec![ft]), &[]).await.unwrap();

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
            an_entry("127.0.0.1", modules::MODULE_TYPE_DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    let gateway_type = modules::builtin_fixture_type(modules::MODULE_TYPE_DMX_OUT).unwrap();
    let gateway = a_node_fixture(&gateway_type, "gate1", Some(5));
    let (dimmer, dimmer_type) = a_dmx_dimmer(5, 1.0);

    output
        .send(&patch(vec![gateway, dimmer], vec![gateway_type, dimmer_type]), &[])
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
            an_entry("127.0.0.1", modules::MODULE_TYPE_DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    let gateway_type = modules::builtin_fixture_type(modules::MODULE_TYPE_DMX_OUT).unwrap();
    let gateway = a_node_fixture(&gateway_type, "gate1", Some(5));
    let (dimmer, dimmer_type) = a_dmx_dimmer(9, 1.0);

    output
        .send(&patch(vec![gateway, dimmer], vec![gateway_type, dimmer_type]), &[])
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
            an_entry("127.0.0.1", modules::MODULE_TYPE_DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    let gateway_type = modules::builtin_fixture_type(modules::MODULE_TYPE_DMX_OUT).unwrap();
    let gateway = a_node_fixture(&gateway_type, "gate1", Some(5));
    let (dimmer, dimmer_type) = a_dmx_dimmer(5, 1.0);
    let frame = || patch(vec![gateway.clone(), dimmer.clone()], vec![gateway_type.clone(), dimmer_type.clone()]);

    output.send(&frame(), &[]).await.unwrap();
    let _ = recv(&socket).await;
    output.send(&frame(), &[]).await.unwrap();

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
            an_entry("127.0.0.1", modules::MODULE_TYPE_DMX_OUT, Some(5), true),
        )]),
        devices,
        port,
    )
    .await
    .unwrap();

    // A discovered gateway that nobody adopted, and an ordinary DMX rig.
    let (dimmer, dimmer_type) = a_dmx_dimmer(5, 1.0);
    output.send(&patch(vec![dimmer], vec![dimmer_type]), &[]).await.unwrap();

    assert!(sent(&mut received).is_empty());
    let anything =
        tokio::time::timeout(std::time::Duration::from_millis(100), recv(&socket)).await;
    assert!(anything.is_err(), "a discovered device is not an adopted one");
}
