//! Art-Net output.
//!
//! Art-Net carries whole universes, so there is nothing to gain from knowing which
//! fixtures moved. What it does avoid is sending a universe whose 512 bytes are
//! unchanged, which is what keeps an idle rig off the network.

use std::net::SocketAddr;

use anyhow::Result;
use tokio::net::UdpSocket;
use uuid::Uuid;

use super::{
    dmx::{render, Patch, SequenceCounter, UniverseCache, UNIVERSE_SIZE, REFRESH_AFTER},
    OutputPlugin, SendFuture,
};

/// The port Art-Net is specified to use.
pub const ARTNET_PORT: u16 = 6454;

const HEADER: &[u8; 8] = b"Art-Net\0";
const OP_DMX: u16 = 0x5000;
const PROTOCOL_VERSION: u16 = 14;

pub struct ArtNetOutput {
    socket: UdpSocket,
    target: SocketAddr,
    sent: UniverseCache,
    sequence: SequenceCounter,
}

impl ArtNetOutput {
    pub async fn bind(target: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        if target.ip().is_multicast() || is_broadcast(&target) {
            socket.set_broadcast(true)?;
        }
        Ok(Self { socket, target, sent: UniverseCache::default(), sequence: SequenceCounter::default() })
    }
}

impl OutputPlugin for ArtNetOutput {
    fn name(&self) -> &'static str {
        "art-net"
    }

    fn send<'a>(&'a mut self, patch: &'a Patch, _changed: &'a [Uuid]) -> SendFuture<'a> {
        Box::pin(async move {
            let now = std::time::Instant::now();
            for universe in render(patch) {
                if !self.sent.needs_send(&universe, now, REFRESH_AFTER) {
                    continue;
                }
                let sequence = self.sequence.next(universe.number);
                let packet = art_dmx(universe.number, sequence, &universe.channels);
                self.socket.send_to(&packet, self.target).await?;
            }
            Ok(())
        })
    }
}

/// Build an ArtDmx packet.
///
/// The 15-bit port address splits into Net in the high 7 bits and SubUni in the low
/// 8, and the two length bytes are the one big-endian field in a header that is
/// otherwise little-endian. Both are easy to get backwards.
pub fn art_dmx(universe: u16, sequence: u8, channels: &[u8; UNIVERSE_SIZE]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(18 + UNIVERSE_SIZE);
    packet.extend_from_slice(HEADER);
    packet.extend_from_slice(&OP_DMX.to_le_bytes());
    packet.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    packet.push(sequence);
    packet.push(0); // physical, informational only
    packet.push((universe & 0x00FF) as u8); // SubUni
    packet.push(((universe >> 8) & 0x007F) as u8); // Net
    packet.extend_from_slice(&(UNIVERSE_SIZE as u16).to_be_bytes());
    packet.extend_from_slice(channels);
    packet
}

fn is_broadcast(addr: &SocketAddr) -> bool {
    match addr {
        SocketAddr::V4(v4) => v4.ip().is_broadcast() || v4.ip().octets()[3] == 255,
        SocketAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests;
