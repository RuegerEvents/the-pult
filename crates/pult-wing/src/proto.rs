//! What the tags mean, and a frame with its container recognised.
//!
//! Every frame has exactly two children: the sender's address, and one container that
//! says what kind of frame it is. [`Message`] is that second thing named.

use crate::chunk::{Chunk, ChunkError, ADDRESS, ROOT};
use crate::input::{self, InputState};
use crate::output::OutputState;
use serde::{Deserialize, Serialize};

/// The containers a frame's payload can be.
pub mod tag {
    pub const HEARTBEAT: u16 = 0x0023;
    pub const CAPABILITIES_REQUEST: u16 = 0x0025;
    pub const CAPABILITIES: u16 = 0x0026;
    pub const IO: u16 = 0x0027;
    pub const SOFTWARE: u16 = 0x0028;
    /// The onPC licence dongle. Recognised so that it can be named in a log and
    /// skipped; deliberately not implemented.
    pub const CERTIFICATE: u16 = 0x0029;
    /// Empty, sent once by the wing at the end of the boot.
    pub const READY: u16 = 0x002a;
}

/// Leaf tags inside [`tag::HEARTBEAT`], [`tag::CAPABILITIES`] and [`tag::SOFTWARE`].
mod leaf {
    pub const STATE: u16 = 0x0001;
    pub const DEVICE_TYPE: u16 = 0x000f;
    pub const CERT_CAPS: u16 = 0x0010;
    pub const CERT_CAPS_OVERALL: u16 = 0x0011;
    pub const BUTTONS: u16 = 0x0001;
    pub const ENCODERS: u16 = 0x0003;
    pub const DIGITAL_IN: u16 = 0x000e;
    pub const LEDS: u16 = 0x0004;

    pub const SOFTWARE_NAME: u16 = 0x0004;
    pub const SOFTWARE_OFFSET: u16 = 0x0001;
    pub const SOFTWARE_DATA: u16 = 0x0002;
    pub const SOFTWARE_PROGRESS: u16 = 0x0003;
}

/// A station's `ID8` — eight bytes that the host fills in and the wing leaves zero.
///
/// grandMA3 prints it as a `u64`, so that is what this holds: the host's
/// `7f 00 00 01 00 02 31 03` on the wire is `0x033102000100007F` in a log line, and the
/// two being the same number is the only reason to believe the field was read right.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl NodeId {
    pub const WING: NodeId = NodeId(0);
    fn bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }
    fn from_slice(b: &[u8]) -> NodeId {
        let mut a = [0u8; 8];
        a[..b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
        NodeId(u64::from_le_bytes(a))
    }
}

/// Where the wing is in its boot.
///
/// The numbers are the wing's own, carried in every heartbeat. A host that stops
/// answering sends it back to [`Announcing`](DeviceState::Announcing) within a few
/// seconds, because the application it was given lives in RAM.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    /// 1 — the bootloader, repeating its announce until somebody answers.
    Announcing,
    /// 2 — capabilities exchanged.
    Ready,
    /// 3 — taking its application.
    Loading,
    /// 4 — running it, and sending input.
    Running,
    Other(u32),
}

impl DeviceState {
    pub fn from_u32(v: u32) -> DeviceState {
        match v {
            1 => DeviceState::Announcing,
            2 => DeviceState::Ready,
            3 => DeviceState::Loading,
            4 => DeviceState::Running,
            other => DeviceState::Other(other),
        }
    }
    pub fn as_u32(self) -> u32 {
        match self {
            DeviceState::Announcing => 1,
            DeviceState::Ready => 2,
            DeviceState::Loading => 3,
            DeviceState::Running => 4,
            DeviceState::Other(v) => v,
        }
    }
}

/// What the wing says it is and has.
///
/// The bootloader answers a short version of this and the application a longer one with
/// fields nobody has named yet; those are kept in `extra` rather than dropped, because
/// a field this code does not understand is still evidence.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub device_type: String,
    pub buttons: u32,
    pub encoders: u32,
    pub leds: u32,
    pub digital_in: u32,
    pub certificate: u32,
    pub certificate_overall: u32,
    /// Tags seen in the reply that this code has no name for, in order.
    pub extra: Vec<(u16, u32)>,
}

/// A frame, with its payload container recognised.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Heartbeat(DeviceState),
    CapabilitiesRequest,
    Capabilities(Capabilities),
    /// A real-time frame, kept whole.
    ///
    /// It is **not** decoded into events here, and that is the point. A block is a
    /// whole snapshot, so an event is the difference between this frame and the last
    /// one — which only the caller holds. Differencing against a fresh state instead
    /// reports every held bit as a new press and every release as nothing at all,
    /// which on a real desk is every key latching on and never going off.
    /// Feed it to [`InputState::absorb`].
    Io(Chunk),
    /// One real-time frame from the host: LEDs, faders, sync.
    Output(OutputState),
    /// The wing asking for its application by name.
    SoftwareRequest(String),
    /// A slice of that application: byte offset, whether it is the last one, and the
    /// bytes. The offset chunk is eight bytes, two `u32`: the offset, and a flag that
    /// grandMA3 sets to 1 exactly once, on the packet that finishes the image. Without
    /// it the wing is never told the download ended.
    SoftwarePacket { offset: u32, last: bool, data: Vec<u8> },
    /// The wing's acknowledgement of a packet.
    SoftwareProgress(Vec<u8>),
    /// A line of text from the wing, e.g. `@No update needed`.
    Text(String),
    Ready,
    /// The licence exchange, named and not implemented.
    Certificate,
    /// A container this build has never heard of, kept whole rather than dropped —
    /// the rule the layout tree already follows for a panel id it does not know.
    Unknown(Chunk),
}

impl Message {
    /// Decode one frame.
    pub fn decode(buf: &[u8]) -> Result<(NodeId, Message), ChunkError> {
        let frame = Chunk::decode(buf)?;
        let from = frame
            .child(ADDRESS)
            .and_then(|c| c.as_bytes().ok())
            .map(NodeId::from_slice)
            .unwrap_or_default();
        let body = frame
            .children()?
            .iter()
            .find(|c| c.tag() != ADDRESS)
            .ok_or(ChunkError::NotAFrame(ROOT))?;
        Ok((from, Message::from_container(body)?))
    }

    fn from_container(c: &Chunk) -> Result<Message, ChunkError> {
        Ok(match c.tag() {
            tag::HEARTBEAT => Message::Heartbeat(DeviceState::from_u32(
                c.child_u32(leaf::STATE).unwrap_or(0),
            )),
            tag::CAPABILITIES_REQUEST => Message::CapabilitiesRequest,
            tag::CAPABILITIES => Message::Capabilities(caps(c)),
            // A real-time frame is handed over whole. Which direction it came from is
            // the caller's to know — the wing's frames carry keys, faders, encoders and
            // digital inputs, the host's carry LEDs, faders and sync, and `0x0003` is
            // faders in both — so guessing here would be guessing at something the
            // transport already knows for certain.
            tag::IO => match c.child_str(input::tag::TEXT) {
                Some(text) => Message::Text(text),
                None => Message::Io(c.clone()),
            },
            tag::SOFTWARE => software(c),
            tag::CERTIFICATE => Message::Certificate,
            tag::READY => Message::Ready,
            _ => Message::Unknown(c.clone()),
        })
    }

    /// The container this message is, ready to be wrapped in a frame.
    fn container(&self) -> Chunk {
        match self {
            Message::Heartbeat(s) => {
                Chunk::group(tag::HEARTBEAT, vec![Chunk::u32(leaf::STATE, s.as_u32())])
            }
            Message::CapabilitiesRequest => Chunk::group(tag::CAPABILITIES_REQUEST, vec![]),
            Message::Capabilities(c) => Chunk::group(
                tag::CAPABILITIES,
                vec![
                    Chunk::u32(leaf::CERT_CAPS_OVERALL, c.certificate_overall),
                    Chunk::u32(leaf::CERT_CAPS, c.certificate),
                    Chunk::bytes(leaf::DEVICE_TYPE, with_nul(&c.device_type)),
                    Chunk::u32(leaf::BUTTONS, c.buttons),
                    Chunk::u32(leaf::ENCODERS, c.encoders),
                    Chunk::u32(leaf::DIGITAL_IN, c.digital_in),
                    Chunk::u32(leaf::LEDS, c.leds),
                ],
            ),
            Message::Output(o) => o.container(),
            Message::SoftwareRequest(name) => Chunk::group(
                tag::SOFTWARE,
                vec![Chunk::bytes(leaf::SOFTWARE_NAME, with_nul(name))],
            ),
            Message::SoftwarePacket { offset, last, data } => {
                let mut off = [0u8; 8];
                off[..4].copy_from_slice(&offset.to_le_bytes());
                off[4..].copy_from_slice(&u32::from(*last).to_le_bytes());
                Chunk::group(
                    tag::SOFTWARE,
                    vec![
                        Chunk::bytes(leaf::SOFTWARE_OFFSET, off.to_vec()),
                        Chunk::bytes(leaf::SOFTWARE_DATA, data.clone()),
                    ],
                )
            }
            Message::SoftwareProgress(b) => Chunk::group(
                tag::SOFTWARE,
                vec![Chunk::bytes(leaf::SOFTWARE_PROGRESS, b.clone())],
            ),
            Message::Text(t) => {
                Chunk::group(tag::IO, vec![Chunk::bytes(input::tag::TEXT, with_nul(t))])
            }
            Message::Ready => Chunk::group(tag::READY, vec![]),
            Message::Certificate => Chunk::group(tag::CERTIFICATE, vec![]),
            Message::Io(c) => c.clone(),
            Message::Unknown(c) => c.clone(),
        }
    }

    /// The bytes of a frame carrying this message, from `from`.
    pub fn encode(&self, from: NodeId) -> Vec<u8> {
        Chunk::group(
            ROOT,
            vec![
                Chunk::bytes(ADDRESS, from.bytes().to_vec()),
                self.container(),
            ],
        )
        .encode()
    }
}

fn with_nul(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

fn caps(c: &Chunk) -> Capabilities {
    let known = [
        leaf::DEVICE_TYPE,
        leaf::CERT_CAPS,
        leaf::CERT_CAPS_OVERALL,
        leaf::BUTTONS,
        leaf::ENCODERS,
        leaf::DIGITAL_IN,
        leaf::LEDS,
    ];
    let mut out = Capabilities {
        device_type: c.child_str(leaf::DEVICE_TYPE).unwrap_or_default(),
        buttons: c.child_u32(leaf::BUTTONS).unwrap_or(0),
        encoders: c.child_u32(leaf::ENCODERS).unwrap_or(0),
        leds: c.child_u32(leaf::LEDS).unwrap_or(0),
        digital_in: c.child_u32(leaf::DIGITAL_IN).unwrap_or(0),
        certificate: c.child_u32(leaf::CERT_CAPS).unwrap_or(0),
        certificate_overall: c.child_u32(leaf::CERT_CAPS_OVERALL).unwrap_or(0),
        extra: Vec::new(),
    };
    // `0x0001` and `0x0003` are buttons and encoders here; the device-type string is
    // not a u32. Everything else the reply carries is recorded as it stands.
    for ch in c.children().unwrap_or(&[]) {
        if known.contains(&ch.tag()) {
            continue;
        }
        if let Some(v) = c.child_u32(ch.tag()) {
            out.extra.push((ch.tag(), v));
        }
    }
    out
}

fn software(c: &Chunk) -> Message {
    if let Some(name) = c.child_str(leaf::SOFTWARE_NAME) {
        return Message::SoftwareRequest(name);
    }
    if let Some(data) = c.child(leaf::SOFTWARE_DATA).and_then(|d| d.as_bytes().ok()) {
        let off = c
            .child(leaf::SOFTWARE_OFFSET)
            .and_then(|o| o.as_bytes().ok())
            .unwrap_or(&[]);
        let word = |i: usize| {
            off.get(i * 4..i * 4 + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .unwrap_or(0)
        };
        return Message::SoftwarePacket {
            offset: word(0),
            last: word(1) != 0,
            data: data.to_vec(),
        };
    }
    let progress = c
        .child(leaf::SOFTWARE_PROGRESS)
        .and_then(|p| p.as_bytes().ok())
        .unwrap_or(&[])
        .to_vec();
    Message::SoftwareProgress(progress)
}

/// What a host has to keep doing to hold a wing up.
///
/// It is here rather than in the transport because it is a fact about the protocol: the
/// wing drops back to the bootloader if the host goes quiet, so "send something" is a
/// rule, not a tuning constant somebody picked.
pub const KEEPALIVE_MS: u64 = 500;

/// How the boot runs, as a host sees it. Feed it what arrives and do what it says.
#[derive(Clone, Debug, Default)]
pub struct Boot {
    pub state: Option<DeviceState>,
    pub caps: Option<Capabilities>,
    pub wanted: Option<String>,
    pub sent: u32,
    pub input: InputState,
    /// Set once the wing has taken its application and restarted. Before that a
    /// `Loading` heartbeat means it is busy taking the image and must be left alone;
    /// afterwards it means the application is up and waiting to be told to run.
    pub restarted: bool,
    /// Set once the host has said `Running`. It says it **once**: the wing repeats its
    /// `Loading` heartbeat about once a second while it waits, and answering every one
    /// of them sends two within a few hundred microseconds, which the wing answers by
    /// dropping off the bus.
    pub declared_running: bool,
}

/// What the host should do next.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Send this and wait for more.
    Send(Message),
    /// Nothing to do; keep the heartbeat going.
    Wait,
    /// The wing is up and sending input.
    Running,
    /// The download finished and the wing is restarting into what it was given. The
    /// host must now say **nothing** and let go: it drops off the bus, comes back a
    /// few seconds later, and the boot runs again — that second round is the one that
    /// reaches [`DeviceState::Running`].
    Restarting,
}

impl Boot {
    /// Advance on one received message. `image` is the application to serve when the
    /// wing asks for it — see `docs/WING-PROTOCOL.md` on why a host must carry it and
    /// must not ship it.
    pub fn on(&mut self, msg: &Message, image: Option<&[u8]>) -> Step {
        match msg {
            Message::Heartbeat(s) => {
                self.state = Some(*s);
                match s {
                    DeviceState::Announcing => Step::Send(Message::CapabilitiesRequest),
                    DeviceState::Running => Step::Running,
                    // **The host is what starts it.** After the restart the wing holds
                    // at `Loading` indefinitely — grandMA3's capture shows it sitting
                    // there for two seconds while the licence exchange runs — and what
                    // actually moves it on is the host sending a heartbeat that says
                    // `Running`. The wing then echoes state 4 and input begins.
                    DeviceState::Loading if self.restarted && !self.declared_running => {
                        self.declared_running = true;
                        Step::Send(Message::Heartbeat(DeviceState::Running))
                    }
                    _ => Step::Wait,
                }
            }
            Message::Capabilities(c) => {
                self.caps = Some(c.clone());
                Step::Send(Message::Heartbeat(DeviceState::Ready))
            }
            Message::SoftwareRequest(name) => {
                self.wanted = Some(name.clone());
                self.sent = 0;
                self.packet(image)
            }
            Message::SoftwareProgress(_) => self.packet(image),
            // The wing restarts into what it was just given, and the boot runs a second
            // time — which is why grandMA3's capture carries two capabilities replies,
            // a short one from the bootloader and a longer one from the application.
            //
            // The host sends **nothing** here. grandMA3 says not one word between this
            // and the device coming back three seconds later, and a capabilities
            // request put into that gap reaches a wing in the middle of handing over:
            // it restarts into the bootloader instead, asks for the image again, and
            // the whole thing loops for ever. That was a real defect, found by diffing
            // against grandMA3's own timing rather than by reading the bytes.
            Message::Ready => {
                self.sent = 0;
                self.restarted = true;
                self.declared_running = false;
                Step::Restarting
            }
            Message::Io(c) => {
                // The boot keeps its own picture of the desk so that a caller that only
                // wants to follow along does not have to.
                let _ = self.input.absorb(c);
                Step::Running
            }
            _ => Step::Wait,
        }
    }

    /// The application goes out in 512-byte packets; the last one is short.
    fn packet(&mut self, image: Option<&[u8]>) -> Step {
        const PACKET: usize = 512;
        let Some(image) = image else { return Step::Wait };
        let offset = self.sent as usize;
        if offset >= image.len() {
            return Step::Wait;
        }
        let end = (offset + PACKET).min(image.len());
        self.sent = end as u32;
        Step::Send(Message::SoftwarePacket {
            offset: offset as u32,
            last: end == image.len(),
            data: image[offset..end].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Vec<u8> {
        (0..1200u32).map(|i| i as u8).collect()
    }

    /// The boot, driven entirely by what the wing says.
    #[test]
    fn the_host_only_ever_answers() {
        let img = image();
        let mut b = Boot::default();

        assert_eq!(
            b.on(&Message::Heartbeat(DeviceState::Announcing), Some(&img)),
            Step::Send(Message::CapabilitiesRequest)
        );
        assert_eq!(
            b.on(&Message::Capabilities(Capabilities::default()), Some(&img)),
            Step::Send(Message::Heartbeat(DeviceState::Ready))
        );

        // 1200 bytes is three packets, and only the third says so.
        let mut offsets = Vec::new();
        let mut step = b.on(&Message::SoftwareRequest("app.bin".into()), Some(&img));
        while let Step::Send(Message::SoftwarePacket { offset, last, ref data }) = step {
            offsets.push((offset, last, data.len()));
            step = b.on(&Message::SoftwareProgress(vec![]), Some(&img));
        }
        assert_eq!(
            offsets,
            vec![(0, false, 512), (512, false, 512), (1024, true, 176)]
        );

        // And then the host lets go. Saying anything here breaks the handover.
        assert_eq!(b.on(&Message::Ready, Some(&img)), Step::Restarting);

        // On the far side the application waits at `Loading`, and the host says
        // `Running` exactly once however many times it is asked.
        let loading = Message::Heartbeat(DeviceState::Loading);
        assert_eq!(
            b.on(&loading, Some(&img)),
            Step::Send(Message::Heartbeat(DeviceState::Running))
        );
        assert_eq!(b.on(&loading, Some(&img)), Step::Wait);
        assert_eq!(b.on(&loading, Some(&img)), Step::Wait);
        assert_eq!(
            b.on(&Message::Heartbeat(DeviceState::Running), Some(&img)),
            Step::Running
        );
    }

    /// A host with no image answers what it can and asks for nothing it cannot send.
    #[test]
    fn without_an_image_the_boot_simply_waits() {
        let mut b = Boot::default();
        assert_eq!(b.on(&Message::SoftwareRequest("app.bin".into()), None), Step::Wait);
    }
}
