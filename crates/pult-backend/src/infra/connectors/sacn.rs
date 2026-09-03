//! sACN (E1.31) output.
//!
//! Where Art-Net broadcasts to a configured address, E1.31 has a multicast group
//! per universe — `239.255.<high>.<low>` — so a receiver joins only the universes
//! it cares about and the rest of the network never sees them. That is the whole
//! reason to prefer it, and the reason the OpenHaunt DMX gateway asks for it.
//!
//! The same packet is what [`super::openhaunt`] unicasts to a gateway, so the
//! builder here takes a target and the plugin is a thin thing around it.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use tokio::net::UdpSocket;
use uuid::Uuid;

use super::{
    dmx::{render, Patch, SequenceCounter, UniverseCache, REFRESH_AFTER, UNIVERSE_SIZE},
    Frame, Frames, OutputPlugin, SendFuture,
};

/// The port E1.31 is specified to use.
pub const SACN_PORT: u16 = 5568;

/// Total length of a data packet: 126 bytes of header plus 512 channels.
pub const PACKET_SIZE: usize = 638;

const ACN_IDENTIFIER: &[u8; 12] = b"ASC-E1.17\0\0\0";
const VECTOR_ROOT_DATA: u32 = 0x0000_0004;
const VECTOR_FRAMING_DATA: u32 = 0x0000_0002;
const VECTOR_DMP_SET_PROPERTY: u8 = 0x02;
const DEFAULT_PRIORITY: u8 = 100;

/// The multicast group a universe is carried on.
pub fn multicast_group(universe: u16) -> Ipv4Addr {
    let [high, low] = universe.to_be_bytes();
    Ipv4Addr::new(239, 255, high, low)
}

/// Build an E1.31 data packet.
///
/// Three nested PDUs, each opening with a flags-and-length field: the top nibble is
/// `0x7`, the remaining twelve bits are the length of that PDU *from its own first
/// byte to the end of the packet*. Getting one of the three lengths wrong produces a
/// packet most receivers silently drop, which is why they are computed rather than
/// written out.
pub fn e131_data_packet(
    cid: &[u8; 16],
    source_name: &str,
    universe: u16,
    sequence: u8,
    priority: u8,
    channels: &[u8; UNIVERSE_SIZE],
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(PACKET_SIZE);

    // ── Root layer ──
    packet.extend_from_slice(&0x0010u16.to_be_bytes()); // preamble size
    packet.extend_from_slice(&0x0000u16.to_be_bytes()); // postamble size
    packet.extend_from_slice(ACN_IDENTIFIER);
    // Root PDU runs from byte 16 to the end.
    packet.extend_from_slice(&flags_and_length(PACKET_SIZE - 16));
    packet.extend_from_slice(&VECTOR_ROOT_DATA.to_be_bytes());
    packet.extend_from_slice(cid);

    // ── Framing layer ── from byte 38 to the end.
    packet.extend_from_slice(&flags_and_length(PACKET_SIZE - 38));
    packet.extend_from_slice(&VECTOR_FRAMING_DATA.to_be_bytes());
    let mut name = [0u8; 64];
    let source = source_name.as_bytes();
    let n = source.len().min(63);
    name[..n].copy_from_slice(&source[..n]);
    packet.extend_from_slice(&name);
    packet.push(priority);
    packet.extend_from_slice(&0u16.to_be_bytes()); // synchronization address: none
    packet.push(sequence);
    packet.push(0); // options: no preview, no terminate
    packet.extend_from_slice(&universe.to_be_bytes());

    // ── DMP layer ── from byte 115 to the end.
    packet.extend_from_slice(&flags_and_length(PACKET_SIZE - 115));
    packet.push(VECTOR_DMP_SET_PROPERTY);
    packet.push(0xa1); // address type and data type
    packet.extend_from_slice(&0x0000u16.to_be_bytes()); // first property address
    packet.extend_from_slice(&0x0001u16.to_be_bytes()); // address increment
    // The count includes the start code, so 512 channels are 513 property values.
    packet.extend_from_slice(&((UNIVERSE_SIZE + 1) as u16).to_be_bytes());
    packet.push(0x00); // DMX512-A start code
    packet.extend_from_slice(channels);

    debug_assert_eq!(packet.len(), PACKET_SIZE);
    packet
}

fn flags_and_length(length: usize) -> [u8; 2] {
    ((0x7000 | (length as u16 & 0x0fff)) as u16).to_be_bytes()
}

// ── The plugin ────────────────────────────────────────────────────────────────

pub struct SacnOutput {
    socket: UdpSocket,
    /// Fixed for the life of the process. A receiver uses it to tell two sources
    /// apart when both claim a universe, so it must not change between packets.
    cid: [u8; 16],
    source_name: String,
    /// Where to send. None means the multicast group for each universe, which is
    /// what E1.31 is for; a concrete address is a receiver that cannot be reached
    /// by multicast.
    target: Option<SocketAddr>,
    sent: UniverseCache,
    sequence: SequenceCounter,
}

impl SacnOutput {
    pub async fn bind(target: Option<SocketAddr>) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.set_multicast_loop_v4(true)?;
        Ok(Self {
            socket,
            cid: *Uuid::new_v4().as_bytes(),
            source_name: "the-pult".to_string(),
            target,
            sent: UniverseCache::default(),
            sequence: SequenceCounter::default(),
        })
    }

    fn destination(&self, universe: u16) -> SocketAddr {
        self.target
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(multicast_group(universe)), SACN_PORT))
    }
}

impl OutputPlugin for SacnOutput {
    fn frames(&self) -> Frames {
        Frames::DMX
    }

    fn name(&self) -> &'static str {
        "sacn"
    }

    fn send<'a>(
        &'a mut self,
        patch: &'a Patch,
        _changed: &'a [Uuid],
        now_ms: u64,
    ) -> SendFuture<'a> {
        Box::pin(async move {
            let now = std::time::Instant::now();
            // Timed on its own: rendering is where every parameter of every patched
            // fixture is worked out, and putting the bytes on the wire is the rest.
            let universes = render(patch, now_ms);
            let mut frame = Frame::evaluated(now.elapsed());
            for universe in universes {
                if !self.sent.needs_send(&universe, now, REFRESH_AFTER) {
                    continue;
                }
                let sequence = self.sequence.next(universe.number);
                let packet = e131_data_packet(
                    &self.cid,
                    &self.source_name,
                    universe.number,
                    sequence,
                    DEFAULT_PRIORITY,
                    &universe.channels,
                );
                self.socket.send_to(&packet, self.destination(universe.number)).await?;
                // After the send: a universe the dedup skipped never reached the wire.
                frame.sent(packet.len());
            }
            Ok(frame)
        })
    }
}

#[cfg(test)]
mod tests;
