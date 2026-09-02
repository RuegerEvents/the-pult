//! Parse, write, parse: what goes in comes out.
//!
//! The comparison is over canonical XML rather than raw bytes, because attribute
//! order and the spelling of a number are not information. Two claims, and they are
//! different strengths:
//!
//! - For the **hand-authored** fixtures, the original file and our rewrite of it are
//!   canonically equal. Those files use only what the object model covers, so
//!   anything dropped is a hole in the model.
//! - For the **corpus** — real files off the Share, which carry vendor attributes and
//!   spec corners nobody has modelled — the claim is that writing is *stable*: our
//!   rewrite of a file and our rewrite of that rewrite agree, and every value we did
//!   read survives. A stronger claim would fail on files that are not wrong.

mod common;

use pult_gdtf::{canonicalize, GdtfFile};

#[test]
fn a_hand_authored_file_survives_a_round_trip_exactly() {
    for (name, bytes) in common::hand_authored() {
        let original = common::description(&name);
        let parsed = GdtfFile::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let written = parsed.write().unwrap();
        let reparsed =
            GdtfFile::parse(&written).unwrap_or_else(|e| panic!("{name} rewritten: {e}"));

        assert_eq!(parsed, reparsed, "{name}: the model changed across a write");
        assert_eq!(
            canonicalize(&original).unwrap(),
            canonicalize(&pult_gdtf::xml::to_string(&parsed.description, "GDTF").unwrap()).unwrap(),
            "{name}: the rewrite is not the file we read"
        );
        assert_eq!(
            parsed.resources, reparsed.resources,
            "{name}: resources changed"
        );
    }
}

#[test]
#[ignore = "needs scripts/fetch-interop-corpus.sh"]
fn every_corpus_file_reads_and_rewrites_stably() {
    let files = common::corpus_files();
    assert!(
        !files.is_empty(),
        "no files in {} — run scripts/fetch-interop-corpus.sh first",
        common::corpus().display()
    );

    let mut failures = Vec::new();
    for (name, bytes) in files {
        let parsed = match GdtfFile::parse(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                failures.push(format!("{name}: {error}"));
                continue;
            }
        };
        let written = parsed.write().unwrap();
        let reparsed = match GdtfFile::parse(&written) {
            Ok(reparsed) => reparsed,
            Err(error) => {
                failures.push(format!("{name}: our own output does not parse: {error}"));
                continue;
            }
        };
        if parsed.description != reparsed.description {
            failures.push(format!("{name}: the model changed across a write"));
        }
        if parsed.resources != reparsed.resources {
            failures.push(format!("{name}: resources changed across a write"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} corpus files failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
