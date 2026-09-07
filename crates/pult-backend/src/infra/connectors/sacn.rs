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

use pult_schema::types::output::{OutputSection, SectionBody};

use super::{
    dmx::{render_carried, Patch, SequenceCounter, UniverseCache, REFRESH_AFTER, UNIVERSE_SIZE},
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

/// One E1.31 data packet, read.
///
/// Everything a merge needs and nothing else. The **CID** is the source's identity —
/// fixed for the life of a sender, and the field that lets two consoles on one universe
/// be told apart when their addresses are both behind the same switch. The **priority**
/// is what E1.31 has and Art-Net has not, and is why an sACN merge is defined rather
/// than a guess.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedE131 {
    pub cid: [u8; 16],
    pub priority: u8,
    pub sequence: u8,
    pub universe: u16,
    /// The 512 slots, start code stripped. Short packets are padded with zero, which
    /// is what a receiver does: a sender is allowed to send fewer slots and the rest
    /// are not "unchanged", they are unlit.
    pub channels: [u8; UNIVERSE_SIZE],
}

/// Read an E1.31 data packet, or answer `None` for anything else on the port.
///
/// Every check here is one a real network will exercise. A universe discovery packet
/// and a synchronization packet arrive on the same port and are not data; a preview
/// packet is a previz's rehearsal and must not reach a rig; and a *terminated* packet
/// is a sender saying it has stopped, which reads as a source going away rather than
/// as a frame of zeros. Refusing by returning `None` rather than by erroring, because
/// a UDP port on a show LAN is a place where things that are not for you arrive
/// constantly and a log line per packet would be the whole log.
pub fn parse_e131(packet: &[u8]) -> Option<ReceivedE131> {
    if packet.len() < 126 {
        return None;
    }
    if &packet[4..16] != ACN_IDENTIFIER {
        return None;
    }
    if u32::from_be_bytes(packet[18..22].try_into().ok()?) != VECTOR_ROOT_DATA {
        return None;
    }
    if u32::from_be_bytes(packet[40..44].try_into().ok()?) != VECTOR_FRAMING_DATA {
        return None;
    }
    let priority = packet[108];
    let sequence = packet[111];
    let options = packet[112];
    // Bit 6 is preview, bit 5 is stream-terminated. Both mean "not a frame for the
    // lamps", for different reasons.
    if options & 0b1100_0000 != 0 {
        return None;
    }
    let universe = u16::from_be_bytes([packet[113], packet[114]]);
    if packet[117] != VECTOR_DMP_SET_PROPERTY {
        return None;
    }
    // The count includes the start code, so 513 is a full universe. A sender may send
    // fewer; one claiming more than fits is malformed and the slice is what bounds it.
    let count = u16::from_be_bytes([packet[123], packet[124]]) as usize;
    if count == 0 || packet[125] != 0x00 {
        // Start code other than zero: RDM, or somebody's per-packet text. Not DMX.
        return None;
    }
    let slots = (count - 1).min(UNIVERSE_SIZE).min(packet.len().saturating_sub(126));
    let mut channels = [0u8; UNIVERSE_SIZE];
    channels[..slots].copy_from_slice(&packet[126..126 + slots]);

    let mut cid = [0u8; 16];
    cid.copy_from_slice(&packet[22..38]);
    Some(ReceivedE131 { cid, priority, sequence, universe, channels })
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
    /// The universes this output carries. Empty is every one in the patch.
    carried: Vec<u16>,
    /// The priority byte every packet carries.
    ///
    /// Worked out by the manager from the row's [`SacnPriority`], this station's slot
    /// and whether it leads — held as a plain number here because a connector must
    /// not have to know what a session is to fill in a field of a packet.
    ///
    /// This is what makes several stations sending one universe a defined thing
    /// rather than a race, and it is the whole reason sACN is the kind that may run
    /// on more than one station where Art-Net is not.
    priority: u8,
    sent: UniverseCache,
    sequence: SequenceCounter,
}

impl SacnOutput {
    /// `interface` is the address to leave by, and for multicast it is more than a
    /// source address: the outgoing interface for a group is a socket option, and
    /// nothing here set it before — so a station with a house LAN and a lighting one
    /// put its universes on whichever the route table preferred, which on a console
    /// is usually the cable that reaches the internet.
    pub async fn bind(
        target: Option<SocketAddr>,
        interface: Option<std::net::Ipv4Addr>,
    ) -> Result<Self> {
        let socket = crate::infra::net::udp(interface).await?;
        socket.set_multicast_loop_v4(true)?;
        if let Some(address) = interface {
            // `IP_MULTICAST_IF`, which neither `std` nor `tokio` exposes — and it is
            // the option that actually decides which cable a group leaves by, as
            // against the bind address, which only decides the source. `socket2` was
            // already in the tree as tokio's own dependency, so this is a borrow of
            // the socket that exists rather than a second one built to set a flag.
            socket2::SockRef::from(&socket).set_multicast_if_v4(&address)?;
        }
        Ok(Self {
            socket,
            cid: *Uuid::new_v4().as_bytes(),
            source_name: "the-pult".to_string(),
            target,
            carried: Vec::new(),
            priority: DEFAULT_PRIORITY,
            sent: UniverseCache::default(),
            sequence: SequenceCounter::default(),
        })
    }

    /// Restrict this output to the universes its configuration names, the way
    /// [`super::artnet::ArtNetOutput::carrying`] does — and it matters more here,
    /// since multicast means a receiver already hears only the groups it joined and
    /// the filter is about what this station spends rather than about what arrives.
    pub fn carrying(mut self, universes: Vec<u16>) -> Self {
        self.carried = universes;
        self
    }

    /// What priority to claim, taken after construction the way `carrying` is.
    pub fn at_priority(mut self, priority: u8) -> Self {
        self.priority = priority.min(pult_schema::types::network::SACN_PRIORITY_MAX);
        self
    }

    /// What this output is claiming, so the manager can tell whether a change of
    /// leadership or slot means this socket has to be rebuilt.
    pub fn priority(&self) -> u8 {
        self.priority
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
            let universes = render_carried(patch, now_ms, &self.carried);
            let mut frame = Frame::evaluated(now.elapsed());
            for universe in universes {
                if !self.sent.needs_send(&universe, now, REFRESH_AFTER) {
                    continue;
                }
                // Assembling and sending, timed apart, for the reason Art-Net does
                // it: both are per universe and one figure over the pair cannot say
                // which of them a bigger rig is actually spending its frame on.
                let building = std::time::Instant::now();
                let sequence = self.sequence.next(universe.number);
                let packet = e131_data_packet(
                    &self.cid,
                    &self.source_name,
                    universe.number,
                    sequence,
                    self.priority,
                    &universe.channels,
                );
                frame.assembled(building.elapsed());
                self.socket.send_to(&packet, self.destination(universe.number)).await?;
                // After the send: a universe the dedup skipped never reached the wire.
                frame.sent(packet.len());
            }
            Ok(frame)
        })
    }

    /// The same reading of the same cache Art-Net answers with, so a sheet looks the
    /// same whichever of the two carried the universe.
    fn observe(&mut self, focus: Option<&str>) -> Option<Vec<OutputSection>> {
        Some(vec![OutputSection {
            title: match self.target {
                Some(target) => format!("sACN to {target}"),
                None => "sACN, a multicast group per universe".to_string(),
            },
            note: None,
            body: SectionBody::Universes(self.sent.observe(focus, std::time::Instant::now())),
        }])
    }
}

#[cfg(test)]
mod tests;
