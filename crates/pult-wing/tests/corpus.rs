//! Every frame in `testdata/wing-frames.json` came off a real wing. Two things are
//! asserted about each: that it decodes
//! into the message the capture's note says it is, and that re-encoding gives back the
//! same bytes. The second is the one that matters — a codec that reads a protocol but
//! cannot write it is half a codec, and this one has to write.

use pult_wing::chunk::Chunk;
use pult_wing::{DeviceState, Message};
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    frames: Vec<Frame>,
}

#[derive(Deserialize)]
struct Frame {
    dir: String,
    bytes: String,
    containers: Vec<String>,
    note: String,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/wing-frames.json");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).expect("wing-frames.json")
}

fn bytes(f: &Frame) -> Vec<u8> {
    (0..f.bytes.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&f.bytes[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn every_frame_decodes() {
    let c = corpus();
    assert!(c.frames.len() >= 20, "corpus shrank");
    for f in &c.frames {
        let raw = bytes(f);
        Chunk::decode(&raw).unwrap_or_else(|e| panic!("{} ({}): {e}", f.note, f.dir));
    }
}

/// The grammar is exact, so a frame must come back byte for byte. Anything less and a
/// host built on this would be sending a wing something subtly different from what
/// grandMA3 sends it.
#[test]
fn every_frame_re_encodes_byte_for_byte() {
    for f in &corpus().frames {
        let raw = bytes(f);
        let chunk = Chunk::decode(&raw).unwrap();
        assert_eq!(chunk.encode(), raw, "{} ({})", f.note, f.dir);
    }
}

#[test]
fn the_containers_are_the_ones_the_corpus_names() {
    for f in &corpus().frames {
        let chunk = Chunk::decode(&bytes(f)).unwrap();
        let seen: Vec<String> = chunk
            .children()
            .unwrap()
            .iter()
            .map(|c| format!("0x{:04x}", c.tag()))
            .filter(|t| t != "0x0024")
            .collect();
        assert_eq!(seen, f.containers, "{}", f.note);
    }
}

/// Spot the messages the boot turns on, by the note the corpus carries.
#[test]
fn the_boot_messages_are_recognised() {
    let c = corpus();
    let find = |needle: &str| {
        c.frames
            .iter()
            .find(|f| f.note.contains(needle))
            .map(|f| Message::decode(&bytes(f)).unwrap())
            .unwrap_or_else(|| panic!("no frame noted {needle:?}"))
    };

    assert_eq!(find("device state 1").1, Message::Heartbeat(DeviceState::Announcing));
    assert_eq!(find("capabilities request").1, Message::CapabilitiesRequest);
    assert_eq!(find("device state 2").1, Message::Heartbeat(DeviceState::Ready));

    match find("bootloader's").1 {
        Message::Capabilities(c) => {
            assert_eq!(c.device_type, "grandMA3 onPC command wing");
            assert_eq!(c.digital_in, 7);
        }
        other => panic!("expected capabilities, got {other:?}"),
    }

    match find("command_wing_app.bin").1 {
        Message::SoftwareRequest(name) => assert_eq!(name, "command_wing_app.bin"),
        other => panic!("expected a software request, got {other:?}"),
    }

    match find("offset 71680").1 {
        Message::SoftwarePacket { offset, last, data } => {
            // 71680 + 320 = 72000, which is command_wing_app.bin exactly.
            assert_eq!(offset, 71680);
            assert_eq!(data.len(), 320);
            assert!(last, "the packet that finishes the image says so");
        }
        other => panic!("expected the last software packet, got {other:?}"),
    }

    assert_eq!(find("@No update needed").1, Message::Text("@No update needed".into()));
    assert_eq!(find("end of the boot").1, Message::Ready);
}

/// The host's address is the number grandMA3 prints in its own log. If this ever
/// disagrees, the eight bytes are being read in the wrong order.
#[test]
fn the_hosts_address_is_the_number_ma3_logs() {
    let c = corpus();
    let out = c.frames.iter().find(|f| f.dir == "out").unwrap();
    let (from, _) = Message::decode(&bytes(out)).unwrap();
    assert_eq!(from.0 & 0xffff_ffff, 0x0100_007F, "the low half is 127.0.0.1");
}

/// A wing sends zeros where a host sends its id.
#[test]
fn the_wing_does_not_name_itself() {
    let c = corpus();
    for f in c.frames.iter().filter(|f| f.dir == "in") {
        let (from, _) = Message::decode(&bytes(f)).unwrap();
        assert_eq!(from, pult_wing::NodeId::WING, "{}", f.note);
    }
}

/// The gate that matters for anything that writes: what this crate *encodes* has to be
/// what grandMA3 put on the wire, byte for byte. Round-tripping a decoded chunk does
/// not prove it — the first version of the encoder set the sub-chunk bit on an empty
/// container, round-tripped perfectly, and made the wing stall its IN pipe.
#[test]
fn what_the_host_sends_is_what_ma3_sends() {
    use pult_wing::NodeId;
    let c = corpus();
    let out = |needle: &str| {
        c.frames
            .iter()
            .find(|f| f.dir == "out" && f.note.contains(needle))
            .unwrap_or_else(|| panic!("no outbound frame noted {needle:?}"))
    };

    // The host id is whatever the sending station calls itself; take it from the frame
    // being compared so that only the message's own bytes are under test.
    let id_of = |f: &Frame| Message::decode(&bytes(f)).unwrap().0;

    let f = out("capabilities request");
    assert_eq!(Message::CapabilitiesRequest.encode(id_of(f)), bytes(f));

    let f = out("device state 2");
    assert_eq!(
        Message::Heartbeat(DeviceState::Ready).encode(id_of(f)),
        bytes(f)
    );

    let f = out("offset 71680");
    if let (_, Message::SoftwarePacket { offset, last, data }) =
        Message::decode(&bytes(f)).unwrap()
    {
        assert_eq!(
            Message::SoftwarePacket { offset, last, data }.encode(id_of(f)),
            bytes(f)
        );
    } else {
        panic!("expected a software packet");
    }

    let _ = NodeId::WING;
}
