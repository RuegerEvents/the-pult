//! Art-Net output.
//!
//! Art-Net carries whole universes, so there is nothing to gain from knowing which
//! fixtures moved. What it does avoid is sending a universe whose 512 bytes are
//! unchanged, which is what keeps an idle rig off the network.

use std::net::SocketAddr;

use anyhow::Result;
use tokio::net::UdpSocket;
use uuid::Uuid;

use pult_schema::types::output::{OutputSection, SectionBody};

use super::{
    dmx::{render_carried, Patch, SequenceCounter, UniverseCache, UNIVERSE_SIZE, REFRESH_AFTER},
    Frame, Frames, OutputPlugin, SendFuture,
};

/// The port Art-Net is specified to use.
pub const ARTNET_PORT: u16 = 6454;

const HEADER: &[u8; 8] = b"Art-Net\0";
const OP_DMX: u16 = 0x5000;
const PROTOCOL_VERSION: u16 = 14;

pub struct ArtNetOutput {
    socket: UdpSocket,
    target: SocketAddr,
    /// The universes this node is here for. Empty is every one in the patch, which
    /// is what a configuration that names none means and what `bind` leaves it at.
    carried: Vec<u16>,
    sent: UniverseCache,
    sequence: SequenceCounter,
}

impl ArtNetOutput {
    pub async fn bind(target: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        if target.ip().is_multicast() || is_broadcast(&target) {
            socket.set_broadcast(true)?;
        }
        Ok(Self {
            socket,
            target,
            carried: Vec::new(),
            sent: UniverseCache::default(),
            sequence: SequenceCounter::default(),
        })
    }

    /// Restrict this output to the universes its configuration names.
    ///
    /// Taken after construction rather than as an argument to `bind`, the way the
    /// output manager takes its viewers: what a socket is bound to and what goes
    /// through it are separate questions, and every test that is about the packet
    /// format has no opinion on the second.
    pub fn carrying(mut self, universes: Vec<u16>) -> Self {
        self.carried = universes;
        self
    }
}

impl OutputPlugin for ArtNetOutput {
    fn frames(&self) -> Frames {
        Frames::DMX
    }

    fn name(&self) -> &'static str {
        "art-net"
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
            let universes = render_carried(patch, now_ms, &self.carried);
            let mut frame = Frame::evaluated(now.elapsed());
            for universe in universes {
                if !self.sent.needs_send(&universe, now, REFRESH_AFTER) {
                    continue;
                }
                // Assembling and sending, timed apart. Both are per universe and
                // neither shrinks when the evaluator gets faster, so one figure over
                // the pair could not say which of them to work on.
                let building = std::time::Instant::now();
                let sequence = self.sequence.next(universe.number);
                let packet = art_dmx(universe.number, sequence, &universe.channels);
                frame.assembled(building.elapsed());
                self.socket.send_to(&packet, self.target).await?;
                // Counted after the send rather than before it: a universe skipped by
                // the dedup above never reached the wire, and the whole point of the
                // figure is that a settled rig costs less than a moving one.
                frame.sent(packet.len());
            }
            Ok(frame)
        })
    }

    /// The universes as they last went out, read off the dedup cache.
    ///
    /// Nothing is kept for a viewer's sake: the images are here because skipping an
    /// unchanged universe needs them. Which is why watching an Art-Net output costs
    /// nothing at all on the frame path — the reason a viewer can be offered on a rig
    /// that is already busy.
    fn observe(&mut self, focus: Option<&str>) -> Option<Vec<OutputSection>> {
        Some(vec![OutputSection {
            title: format!("Art-Net to {}", self.target),
            note: None,
            body: SectionBody::Universes(self.sent.observe(focus, std::time::Instant::now())),
        }])
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
