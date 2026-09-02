use std::collections::HashMap;

use pult_schema::types::fixture::{
    Fixture, FixtureAddress, FixtureType, ParameterBinding, ParameterDefinition,
    ParameterKind, ParameterValue,
};
use uuid::Uuid;

use super::*;
use crate::infra::connectors::dmx::holding;

fn a_packet(universe: u16, channels: &[u8; UNIVERSE_SIZE]) -> Vec<u8> {
    e131_data_packet(&[0xab; 16], "the-pult", universe, 1, 100, channels)
}

// ── Packet format ─────────────────────────────────────────────────────────────
//
// Checked by byte offset against E1.31-2018. A receiver drops a packet with a bad
// length field without saying so, which is exactly the failure these prevent.

#[test]
fn a_packet_is_the_length_the_standard_says() {
    assert_eq!(a_packet(1, &[0; UNIVERSE_SIZE]).len(), 638);
}

#[test]
fn the_root_layer_identifies_itself_as_acn() {
    let packet = a_packet(1, &[0; UNIVERSE_SIZE]);

    assert_eq!(&packet[0..2], &[0x00, 0x10], "preamble size");
    assert_eq!(&packet[2..4], &[0x00, 0x00], "postamble size");
    assert_eq!(&packet[4..16], b"ASC-E1.17\0\0\0");
    assert_eq!(&packet[18..22], &[0, 0, 0, 4], "root vector is VECTOR_ROOT_E131_DATA");
}

#[test]
fn each_pdu_length_counts_from_its_own_start_to_the_end() {
    let packet = a_packet(1, &[0; UNIVERSE_SIZE]);

    let pdu_length = |at: usize| u16::from_be_bytes([packet[at], packet[at + 1]]) & 0x0fff;
    assert_eq!(packet[16] >> 4, 0x7, "the top nibble is the flags");
    assert_eq!(pdu_length(16), 622, "root PDU: 638 - 16");
    assert_eq!(pdu_length(38), 600, "framing PDU: 638 - 38");
    assert_eq!(pdu_length(115), 523, "DMP PDU: 638 - 115");
}

#[test]
fn the_cid_goes_in_the_root_layer_unchanged() {
    let packet = a_packet(1, &[0; UNIVERSE_SIZE]);
    assert_eq!(&packet[22..38], &[0xab; 16]);
}

#[test]
fn the_source_name_is_a_padded_fixed_width_field() {
    let packet = a_packet(1, &[0; UNIVERSE_SIZE]);

    assert_eq!(&packet[44..52], b"the-pult");
    assert!(packet[52..108].iter().all(|b| *b == 0), "the rest of the 64 bytes is zero");
}

#[test]
fn a_long_source_name_is_truncated_rather_than_running_over_the_next_field() {
    let long = "x".repeat(200);
    let packet = e131_data_packet(&[0; 16], &long, 1, 1, 100, &[0; UNIVERSE_SIZE]);

    assert_eq!(packet.len(), 638);
    assert_eq!(packet[107], 0, "the last byte of the name field stays a terminator");
}

#[test]
fn priority_sequence_and_universe_sit_where_a_receiver_looks_for_them() {
    let packet = e131_data_packet(&[0; 16], "s", 0x0105, 42, 77, &[0; UNIVERSE_SIZE]);

    assert_eq!(packet[108], 77, "priority");
    assert_eq!(&packet[109..111], &[0, 0], "no synchronization address");
    assert_eq!(packet[111], 42, "sequence");
    assert_eq!(packet[112], 0, "options");
    assert_eq!(&packet[113..115], &[0x01, 0x05], "universe, big-endian");
}

#[test]
fn the_dmp_layer_describes_513_property_values_starting_with_the_start_code() {
    let packet = a_packet(1, &[0; UNIVERSE_SIZE]);

    assert_eq!(packet[117], 0x02, "VECTOR_DMP_SET_PROPERTY");
    assert_eq!(packet[118], 0xa1, "address and data type");
    assert_eq!(&packet[119..121], &[0x00, 0x00], "first property address");
    assert_eq!(&packet[121..123], &[0x00, 0x01], "address increment");
    assert_eq!(&packet[123..125], &[0x02, 0x01], "513: the start code plus 512 channels");
    assert_eq!(packet[125], 0x00, "DMX512-A start code");
}

#[test]
fn channel_data_follows_the_start_code_unchanged() {
    let mut channels = [0u8; UNIVERSE_SIZE];
    channels[0] = 255;
    channels[41] = 128;
    channels[511] = 7;

    let packet = a_packet(1, &channels);

    assert_eq!(packet[126], 255);
    assert_eq!(packet[126 + 41], 128);
    assert_eq!(packet[126 + 511], 7);
}

// ── Multicast groups ──────────────────────────────────────────────────────────

#[test]
fn a_universe_maps_to_its_own_multicast_group() {
    assert_eq!(multicast_group(1), std::net::Ipv4Addr::new(239, 255, 0, 1));
    assert_eq!(multicast_group(255), std::net::Ipv4Addr::new(239, 255, 0, 255));
    assert_eq!(multicast_group(256), std::net::Ipv4Addr::new(239, 255, 1, 0));
    assert_eq!(multicast_group(0x0105), std::net::Ipv4Addr::new(239, 255, 1, 5));
}

// ── Sending ───────────────────────────────────────────────────────────────────

fn a_dimmer_patch(universe: u16, level: f32) -> Patch {
    let fixture_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition {
            binding: Some(ParameterBinding::Dmx { channel: 1 }),
            ..ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))
        }],
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
    };
    holding(&mut fixture, "Intensity", ParameterValue::Float(level));

    Patch::new(vec![fixture], vec![fixture_type], vec![])
}

/// A receiver on an ephemeral port, so the tests never touch 5568 or multicast.
async fn a_receiver() -> (tokio::net::UdpSocket, SocketAddr) {
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

#[tokio::test]
async fn fixture_levels_reach_the_wire() {
    let (receiver, addr) = a_receiver().await;
    let mut output = SacnOutput::bind(Some(addr)).await.unwrap();

    output.send(&a_dimmer_patch(3, 1.0), &[], 0).await.unwrap();

    let packet = recv(&receiver).await;
    assert_eq!(&packet[113..115], &[0x00, 0x03], "universe 3");
    assert_eq!(packet[126], 255, "address 1 at full");
}

#[tokio::test]
async fn an_unchanged_universe_is_not_resent() {
    let (receiver, addr) = a_receiver().await;
    let mut output = SacnOutput::bind(Some(addr)).await.unwrap();

    output.send(&a_dimmer_patch(1, 1.0), &[], 0).await.unwrap();
    let _ = recv(&receiver).await;
    output.send(&a_dimmer_patch(1, 1.0), &[], 0).await.unwrap();

    let again = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        recv(&receiver),
    )
    .await;
    assert!(again.is_err(), "an idle rig must stay off the network");
}

#[tokio::test]
async fn the_sequence_counter_advances_and_skips_zero() {
    let (receiver, addr) = a_receiver().await;
    let mut output = SacnOutput::bind(Some(addr)).await.unwrap();

    output.send(&a_dimmer_patch(1, 0.5), &[], 0).await.unwrap();
    let first = recv(&receiver).await;
    output.send(&a_dimmer_patch(1, 0.6), &[], 0).await.unwrap();
    let second = recv(&receiver).await;

    assert_ne!(first[111], 0, "zero means 'sequence not implemented'");
    assert_eq!(second[111], first[111] + 1);
}

#[tokio::test]
async fn a_fixture_on_a_node_puts_nothing_on_a_universe() {
    let (receiver, addr) = a_receiver().await;
    let mut output = SacnOutput::bind(Some(addr)).await.unwrap();

    // Addressed to the node before the patch is built, because where a fixture's
    // channels land is resolved once, when the patch arrives — the same as when it
    // settles. Moving a fixture after the fact moves nothing.
    let mut patch = a_dimmer_patch(1, 1.0);
    let mut fixture = patch.fixtures.remove(0);
    fixture.address = FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: Some(1) };
    let patch = Patch::new(vec![fixture], patch.fixture_types.into_values().collect(), vec![]);
    output.send(&patch, &[], 0).await.unwrap();

    let anything = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        recv(&receiver),
    )
    .await;
    assert!(anything.is_err());
}
