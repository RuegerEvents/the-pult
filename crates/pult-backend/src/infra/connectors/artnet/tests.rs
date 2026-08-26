use std::collections::HashMap;

use pult_schema::types::fixture::{
    Fixture, FixtureType, ParameterDefinition, ParameterKind, ParameterValue,
};
use uuid::Uuid;

use super::*;

// ── Packet format ─────────────────────────────────────────────────────────────

#[test]
fn a_packet_starts_with_the_art_net_header() {
    let packet = art_dmx(0, 1, &[0; UNIVERSE_SIZE]);

    assert_eq!(&packet[0..8], b"Art-Net\0");
    assert_eq!(&packet[8..10], &[0x00, 0x50], "OpDmx is little-endian");
    assert_eq!(&packet[10..12], &[0x00, 0x0e], "protocol version is big-endian");
    assert_eq!(packet.len(), 18 + UNIVERSE_SIZE);
}

#[test]
fn the_universe_number_splits_into_subuni_and_net() {
    let packet = art_dmx(0x0105, 1, &[0; UNIVERSE_SIZE]);

    assert_eq!(packet[14], 0x05, "SubUni is the low byte");
    assert_eq!(packet[15], 0x01, "Net is the high 7 bits");
}

#[test]
fn the_length_field_is_big_endian() {
    let packet = art_dmx(0, 1, &[0; UNIVERSE_SIZE]);

    assert_eq!(&packet[16..18], &[0x02, 0x00], "512 as big-endian");
}

#[test]
fn channel_data_follows_the_header_unchanged() {
    let mut channels = [0u8; UNIVERSE_SIZE];
    channels[0] = 255;
    channels[41] = 128;
    channels[511] = 7;

    let packet = art_dmx(0, 1, &channels);

    assert_eq!(packet[18], 255);
    assert_eq!(packet[18 + 41], 128);
    assert_eq!(packet[18 + 511], 7);
}

// ── Sending ───────────────────────────────────────────────────────────────────

fn a_dimmer_patch(level: f32) -> Patch {
    let fixture_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition {
            kind: ParameterKind::Intensity,
            dmx_channel: 1,
            default_value: ParameterValue::Float(0.0),
        }],
    };
    let mut fixture = Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        universe: 3,
        dmx_address: 1,
        position: None,
        live_values: HashMap::new(),
        active_preset: None,
    };
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(level));

    Patch {
        fixtures: vec![fixture],
        fixture_types: [(fixture_type.id, fixture_type)].into_iter().collect(),
    }
}

/// A receiver bound to a free local port, standing in for a lighting node.
async fn a_node() -> (tokio::net::UdpSocket, std::net::SocketAddr) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    (socket, addr)
}

async fn recv(socket: &tokio::net::UdpSocket) -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    let (n, _) = tokio::time::timeout(std::time::Duration::from_secs(2), socket.recv_from(&mut buf))
        .await
        .expect("expected a packet within two seconds")
        .unwrap();
    buf.truncate(n);
    buf
}

#[tokio::test]
async fn fixture_levels_reach_the_wire() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    output.send(&a_dimmer_patch(1.0), &[]).await.unwrap();

    let packet = recv(&node).await;
    assert_eq!(&packet[0..8], b"Art-Net\0");
    assert_eq!(packet[14], 3, "patched to universe 3");
    assert_eq!(packet[18], 255, "address 1 at full");
}

#[tokio::test]
async fn an_unchanged_universe_is_not_resent() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();
    let patch = a_dimmer_patch(0.5);

    output.send(&patch, &[]).await.unwrap();
    let first = recv(&node).await;
    assert_eq!(first[18], 128);

    // Same values, so nothing should go out.
    output.send(&patch, &[]).await.unwrap();
    output.send(&patch, &[]).await.unwrap();

    // A changed level breaks the silence, and is the next thing the node hears.
    output.send(&a_dimmer_patch(1.0), &[]).await.unwrap();
    let next = recv(&node).await;
    assert_eq!(next[18], 255, "an idle rig must not fill the network with identical frames");
}

#[tokio::test]
async fn the_sequence_counter_advances_and_skips_zero() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    output.send(&a_dimmer_patch(0.1), &[]).await.unwrap();
    let first = recv(&node).await;
    output.send(&a_dimmer_patch(0.2), &[]).await.unwrap();
    let second = recv(&node).await;

    assert_eq!(first[12], 1);
    assert_eq!(second[12], 2);
    assert_ne!(first[12], 0, "zero means 'sequence not implemented'");
}

#[tokio::test]
async fn each_universe_gets_its_own_packet() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    let mut patch = a_dimmer_patch(1.0);
    let mut second = patch.fixtures[0].clone();
    second.id = Uuid::new_v4();
    second.universe = 9;
    patch.fixtures.push(second);

    output.send(&patch, &[]).await.unwrap();

    let mut universes = vec![recv(&node).await[14], recv(&node).await[14]];
    universes.sort();
    assert_eq!(universes, vec![3, 9]);
}

#[tokio::test]
async fn an_empty_patch_sends_nothing() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    let empty = Patch { fixtures: vec![], fixture_types: HashMap::new() };
    output.send(&empty, &[]).await.unwrap();

    let mut buf = [0u8; 64];
    let got = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        node.recv_from(&mut buf),
    )
    .await;
    assert!(got.is_err(), "nothing patched means nothing to send");
}

// ── Several plugins at once ───────────────────────────────────────────────────

/// A plugin that counts calls and can be told to fail, standing in for a second
/// protocol without needing one implemented.
struct Recorder {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    fails: bool,
}

impl OutputPlugin for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }

    fn send<'a>(&'a mut self, _patch: &'a Patch, _changed: &'a [Uuid]) -> crate::infra::connectors::SendFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fails {
                anyhow::bail!("this output is unplugged");
            }
            Ok(())
        })
    }
}

fn counter() -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0))
}

fn count(c: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> usize {
    c.load(std::sync::atomic::Ordering::SeqCst)
}

async fn push_a_patch(handle: &crate::infra::connectors::OutputHandle, patch_source: Patch) {
    handle.push(
        patch_source.fixtures.clone(),
        patch_source.fixture_types.values().cloned().collect(),
        vec![],
    );
}

#[tokio::test]
async fn every_plugin_receives_the_patch() {
    let first = counter();
    let second = counter();
    let (manager, handle) = crate::infra::connectors::OutputManager::new(vec![
        Box::new(Recorder { calls: first.clone(), fails: false }),
        Box::new(Recorder { calls: second.clone(), fails: false }),
    ]);
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(count(&first), 1);
    assert_eq!(count(&second), 1);
}

#[tokio::test]
async fn one_failing_plugin_does_not_silence_the_others() {
    let broken = counter();
    let working = counter();
    let (manager, handle) = crate::infra::connectors::OutputManager::new(vec![
        Box::new(Recorder { calls: broken.clone(), fails: true }),
        Box::new(Recorder { calls: working.clone(), fails: false }),
    ]);
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    push_a_patch(&handle, a_dimmer_patch(0.5)).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(count(&broken), 2, "a failing plugin is still asked next time");
    assert_eq!(
        count(&working), 2,
        "an unplugged interface must not stop the output that is working",
    );
}

#[tokio::test]
async fn art_net_can_feed_two_nodes_at_once() {
    let (first_node, first_addr) = a_node().await;
    let (second_node, second_addr) = a_node().await;

    let (manager, handle) = crate::infra::connectors::OutputManager::new(vec![
        Box::new(ArtNetOutput::bind(first_addr).await.unwrap()),
        Box::new(ArtNetOutput::bind(second_addr).await.unwrap()),
    ]);
    tokio::spawn(manager.run());

    push_a_patch(&handle, a_dimmer_patch(1.0)).await;

    assert_eq!(recv(&first_node).await[18], 255);
    assert_eq!(recv(&second_node).await[18], 255);
}
