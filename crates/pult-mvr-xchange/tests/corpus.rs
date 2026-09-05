//! The corpus: the specification's own examples, and the byte layout as bytes.
//!
//! Two things are proved here that unit tests inside the crate cannot prove. The
//! messages are the document's, so a rename that breaks interoperability fails here
//! rather than passing quietly against a struct this crate also wrote. And the
//! packages are pinned as hex, so the magic, the field order and the endianness are
//! held to something other than this crate's own encoder — a round trip through
//! `encode` and `Decoder` agrees with itself however wrong both halves are.

use pult_mvr_xchange::{
    frame::{self, Kind},
    Decoder, Message, Payload,
};
use serde_json::Value;

const LIMIT: u64 = 16 * 1024 * 1024;

fn corpus() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/messages.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_slice(&bytes).expect("testdata/messages.json is not JSON")
}

fn unhex(s: &str) -> Vec<u8> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).expect("bad hex"))
        .collect()
}

#[test]
fn every_message_in_the_corpus_parses_as_what_it_says_it_is() {
    let corpus = corpus();
    let cases = corpus["messages"].as_array().expect("messages");
    assert!(cases.len() > 10, "the corpus has gone missing");

    for case in cases {
        let name = case["name"].as_str().unwrap_or("unnamed");
        let bytes = serde_json::to_vec(&case["json"]).unwrap();
        let message = Message::from_json(&bytes)
            .unwrap_or_else(|e| panic!("{name}: did not parse: {e}"));
        assert_eq!(
            message.type_name(),
            case["type"].as_str().unwrap(),
            "{name}: parsed as the wrong message"
        );
    }
}

/// What this console *writes* for each case, checked against the corpus where the
/// corpus says the two differ.
///
/// This is where the three lenient readings are held honest: we accept the example's
/// spelling and send the table's, and a case saying so has to keep saying so.
#[test]
fn what_is_written_is_the_specification_table_s_spelling() {
    let corpus = corpus();
    for case in corpus["messages"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("unnamed");
        let Some(strict) = case.get("strict_json") else { continue };
        let parsed = Message::from_json(&serde_json::to_vec(&case["json"]).unwrap()).unwrap();
        let written: Value = serde_json::from_slice(&parsed.to_json().unwrap()).unwrap();
        assert_eq!(&written, strict, "{name}: written differently from the corpus");
    }
}

#[test]
fn a_lenient_case_and_its_strict_form_are_the_same_message() {
    let corpus = corpus();
    for case in corpus["messages"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("unnamed");
        let Some(strict) = case.get("strict_json") else { continue };
        let lenient = Message::from_json(&serde_json::to_vec(&case["json"]).unwrap()).unwrap();
        let strict = Message::from_json(&serde_json::to_vec(strict).unwrap()).unwrap();
        assert_eq!(lenient, strict, "{name}: the two spellings mean different things");
    }
}

#[test]
fn every_message_survives_the_framing() {
    let corpus = corpus();
    for case in corpus["messages"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("unnamed");
        let parsed = Message::from_json(&serde_json::to_vec(&case["json"]).unwrap()).unwrap();
        // Unknown carries nothing, so there is nothing to round trip through.
        if matches!(parsed, Message::Unknown) {
            continue;
        }
        let framed = frame::encode(Kind::Json, &parsed.to_json().unwrap());
        let mut decoder = Decoder::new(LIMIT);
        decoder.feed(&framed);
        let Some(Payload::Json(bytes)) = decoder.next().unwrap() else {
            panic!("{name}: did not come back out of the framing");
        };
        assert_eq!(Message::from_json(&bytes).unwrap(), parsed, "{name}: changed on the way");
    }
}

#[test]
fn the_bytes_on_the_wire_are_the_bytes_in_the_corpus() {
    let corpus = corpus();
    let cases = corpus["packages"].as_array().expect("packages");
    assert!(!cases.is_empty());

    for case in cases {
        let name = case["name"].as_str().unwrap_or("unnamed");
        let kind = match case["kind"].as_str().unwrap() {
            "json" => Kind::Json,
            "file" => Kind::File,
            other => panic!("{name}: unknown kind {other}"),
        };
        let payload = match (case.get("payload_utf8"), case.get("payload_hex")) {
            (Some(text), _) => text.as_str().unwrap().as_bytes().to_vec(),
            (_, Some(hex)) => unhex(hex.as_str().unwrap()),
            _ => panic!("{name}: no payload"),
        };
        let expected = unhex(case["hex"].as_str().unwrap());

        assert_eq!(frame::encode(kind, &payload), expected, "{name}: written differently");

        let mut decoder = Decoder::new(LIMIT);
        decoder.feed(&expected);
        let got = decoder.next().unwrap().expect("a whole package did not decode");
        assert_eq!(got.bytes(), &payload[..], "{name}: read differently");
    }
}

/// A message split at every possible point still arrives exactly once.
///
/// TCP has no message boundaries in it, and this is the failure that only shows up
/// against a real network: a header that arrives in two reads, or two messages that
/// arrive in one.
#[test]
fn a_stream_cut_anywhere_still_yields_the_same_messages() {
    let corpus = corpus();
    let first = &corpus["messages"][0]["json"];
    let message = Message::from_json(&serde_json::to_vec(first).unwrap()).unwrap();
    let mut stream = frame::encode(Kind::Json, &message.to_json().unwrap());
    stream.extend(frame::encode(Kind::File, &[0x50, 0x4b, 0x03, 0x04]));

    for cut in 0..stream.len() {
        let mut decoder = Decoder::new(LIMIT);
        let mut out = Vec::new();
        decoder.feed(&stream[..cut]);
        while let Some(p) = decoder.next().unwrap() {
            out.push(p);
        }
        decoder.feed(&stream[cut..]);
        while let Some(p) = decoder.next().unwrap() {
            out.push(p);
        }
        assert_eq!(out.len(), 2, "cut at {cut} lost or duplicated a message");
        assert_eq!(out[1], Payload::File(vec![0x50, 0x4b, 0x03, 0x04]), "cut at {cut}");
    }
}
