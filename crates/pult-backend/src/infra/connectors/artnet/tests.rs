use std::collections::HashMap;

use pult_schema::types::fixture::{
    Fixture, FixtureAddress, FixtureType, ParameterDefinition,
    ParameterKind, ParameterValue,
};
use uuid::Uuid;

use super::*;
use crate::infra::connectors::dmx::holding;

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
        sensed_values: HashMap::new(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    };
    holding(&mut fixture, "Intensity", ParameterValue::Float(level));

    Patch::new(vec![fixture], vec![fixture_type], vec![])
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

    output.send(&a_dimmer_patch(1.0), &[], 0).await.unwrap();

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

    output.send(&patch, &[], 0).await.unwrap();
    let first = recv(&node).await;
    assert_eq!(first[18], 128);

    // Same values, so nothing should go out.
    output.send(&patch, &[], 0).await.unwrap();
    output.send(&patch, &[], 0).await.unwrap();

    // A changed level breaks the silence, and is the next thing the node hears.
    output.send(&a_dimmer_patch(1.0), &[], 0).await.unwrap();
    let next = recv(&node).await;
    assert_eq!(next[18], 255, "an idle rig must not fill the network with identical frames");
}

#[tokio::test]
async fn the_sequence_counter_advances_and_skips_zero() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    output.send(&a_dimmer_patch(0.1), &[], 0).await.unwrap();
    let first = recv(&node).await;
    output.send(&a_dimmer_patch(0.2), &[], 0).await.unwrap();
    let second = recv(&node).await;

    assert_eq!(first[12], 1);
    assert_eq!(second[12], 2);
    assert_ne!(first[12], 0, "zero means 'sequence not implemented'");
}

#[tokio::test]
async fn each_universe_gets_its_own_packet() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    // Both fixtures before the patch is built: where a fixture's channels land is
    // resolved once, when the patch arrives, the same as when it settles. A fixture
    // pushed in afterwards occupies nothing.
    let built = a_dimmer_patch(1.0);
    let first = built.fixtures[0].clone();
    let mut second = first.clone();
    second.id = Uuid::new_v4();
    second.address = FixtureAddress::dmx(9, 1);
    let patch = Patch::new(
        vec![first, second],
        built.fixture_types.into_values().collect(),
        vec![],
    );

    output.send(&patch, &[], 0).await.unwrap();

    let mut universes = vec![recv(&node).await[14], recv(&node).await[14]];
    universes.sort();
    assert_eq!(universes, vec![3, 9]);
}

#[tokio::test]
async fn an_empty_patch_sends_nothing() {
    let (node, addr) = a_node().await;
    let mut output = ArtNetOutput::bind(addr).await.unwrap();

    let empty = Patch::new(vec![], vec![], vec![]);
    output.send(&empty, &[], 0).await.unwrap();

    let mut buf = [0u8; 64];
    let got = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        node.recv_from(&mut buf),
    )
    .await;
    assert!(got.is_err(), "nothing patched means nothing to send");
}
