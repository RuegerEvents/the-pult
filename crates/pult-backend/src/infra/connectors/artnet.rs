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
    dmx::{render, Patch, Universe, UNIVERSE_SIZE},
    OutputPlugin,
};

/// The port Art-Net is specified to use.
pub const ARTNET_PORT: u16 = 6454;

const HEADER: &[u8; 8] = b"Art-Net\0";
const OP_DMX: u16 = 0x5000;
const PROTOCOL_VERSION: u16 = 14;

/// Art-Net expects a node to hear from a controller regularly. Re-sending every
/// universe about once a second keeps a receiver from deciding the controller is
/// gone, without putting an idle rig's full output on the wire 40 times a second.
const REFRESH_AFTER: std::time::Duration = std::time::Duration::from_millis(800);

pub struct ArtNetOutput {
    socket: UdpSocket,
    target: SocketAddr,
    /// Last universe sent, per universe number, so unchanged data is not resent.
    sent: Vec<(u16, [u8; UNIVERSE_SIZE], std::time::Instant)>,
    /// Art-Net sequence counter, per universe. 0 means "not implemented", so it
    /// wraps 1..=255 rather than through zero.
    sequence: Vec<(u16, u8)>,
}

impl ArtNetOutput {
    pub async fn bind(target: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        if target.ip().is_multicast() || is_broadcast(&target) {
            socket.set_broadcast(true)?;
        }
        Ok(Self { socket, target, sent: Vec::new(), sequence: Vec::new() })
    }

    fn next_sequence(&mut self, universe: u16) -> u8 {
        match self.sequence.iter_mut().find(|(u, _)| *u == universe) {
            Some((_, seq)) => {
                *seq = if *seq >= 255 { 1 } else { *seq + 1 };
                *seq
            }
            None => {
                self.sequence.push((universe, 1));
                1
            }
        }
    }

    /// True if this universe has changed, or has gone long enough without a refresh.
    fn needs_send(&mut self, universe: &Universe, now: std::time::Instant) -> bool {
        match self.sent.iter_mut().find(|(n, _, _)| *n == universe.number) {
            Some((_, channels, last)) => {
                let changed = *channels != universe.channels;
                if changed || now.duration_since(*last) >= REFRESH_AFTER {
                    *channels = universe.channels;
                    *last = now;
                    true
                } else {
                    false
                }
            }
            None => {
                self.sent.push((universe.number, universe.channels, now));
                true
            }
        }
    }
}

impl OutputPlugin for ArtNetOutput {
    fn name(&self) -> &'static str {
        "art-net"
    }

    async fn send(&mut self, patch: &Patch, _changed: &[Uuid]) -> Result<()> {
        let now = std::time::Instant::now();
        for universe in render(patch) {
            if !self.needs_send(&universe, now) {
                continue;
            }
            let sequence = self.next_sequence(universe.number);
            let packet = art_dmx(universe.number, sequence, &universe.channels);
            self.socket.send_to(&packet, self.target).await?;
        }
        Ok(())
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
