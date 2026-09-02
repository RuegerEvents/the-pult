//! Two readers of the same files, compared.
//!
//! `gdtf` 0.3 on crates.io reads GDTF and cannot write it, which is why this crate
//! exists — but it is an independent implementation of the same spec, and where the
//! two disagree about a mode's name or a channel's offsets, one of them has a bug.
//! Cheaper evidence than reading the XSD again.
//!
//! Ignored by default: it needs the downloaded corpus, and a real Share file is the
//! only interesting input. The hand-authored fixtures agree by construction.

mod common;

use std::collections::BTreeSet;
use std::io::Cursor;

#[test]
#[ignore = "needs scripts/fetch-interop-corpus.sh"]
fn both_readers_see_the_same_modes_and_the_same_channels() {
    let files = common::corpus_files();
    assert!(
        !files.is_empty(),
        "no files in {} — run scripts/fetch-interop-corpus.sh first",
        common::corpus().display()
    );

    let mut disagreements = Vec::new();
    for (name, bytes) in files {
        let ours = match pult_gdtf::GdtfFile::parse(&bytes) {
            Ok(file) => file,
            Err(error) => {
                disagreements.push(format!("{name}: we cannot read it: {error}"));
                continue;
            }
        };
        let theirs = match gdtf::GdtfFile::new(Cursor::new(bytes.clone())) {
            Ok(file) => file,
            // Their reader failing is not evidence against ours: it is stricter in
            // places and the point of the comparison is where *both* succeed.
            Err(_) => continue,
        };

        let ours_modes: BTreeSet<String> = ours
            .description
            .fixture_type
            .dmx_modes
            .items
            .iter()
            .map(|mode| mode.name.clone())
            .collect();
        let theirs_modes: BTreeSet<String> = theirs
            .description
            .fixture_types
            .iter()
            .flat_map(|fixture| fixture.dmx_modes.iter())
            .filter_map(|mode| mode.name.as_ref().map(ToString::to_string))
            .collect();

        if ours_modes != theirs_modes {
            disagreements.push(format!(
                "{name}: modes differ — ours {ours_modes:?}, theirs {theirs_modes:?}"
            ));
            continue;
        }

        // The channel offsets a mode's own `<DMXChannels>` lists, before any
        // geometry-reference expansion: that is the part both readers model the same
        // way, and the part where a parse bug shows up.
        for (mine, other) in ours.description.fixture_type.dmx_modes.items.iter().zip(
            theirs
                .description
                .fixture_types
                .first()
                .into_iter()
                .flat_map(|f| f.dmx_modes.iter()),
        ) {
            let mine_offsets: Vec<Vec<u16>> = mine
                .dmx_channels
                .items
                .iter()
                .map(|channel| channel.offsets())
                .collect();
            let other_offsets: Vec<Vec<u16>> = other
                .dmx_channels
                .iter()
                .map(|channel| {
                    channel
                        .offset
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|offset| offset as u16)
                        .collect()
                })
                .collect();
            if mine_offsets != other_offsets {
                disagreements.push(format!(
                    "{name}, mode {:?}: offsets differ — ours {mine_offsets:?}, theirs {other_offsets:?}",
                    mine.name
                ));
            }
        }
    }

    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}
