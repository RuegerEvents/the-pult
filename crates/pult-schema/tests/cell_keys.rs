//! The corpus, from this side.
//!
//! `testdata/cell-keys.json` is read by this test and by `frontend/src/lib/patch.test.ts`.
//! A cell of a multi-cell fixture is part of its kind and spelled into its key, and
//! every map in the console — the programmer, a running fade, a take, a mode's channel
//! layout — is keyed by that string. A browser spelling `ColorRgb@7` one way and the
//! station another is a pixel that one of them never finds.

use pult_schema::types::fixture::{parameter_key, split_cell, ParameterKind};
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    kinds: Vec<KindCase>,
    keys: Vec<KeyCase>,
}

#[derive(Deserialize)]
struct KindCase {
    name: String,
    kind: ParameterKind,
    key: String,
}

#[derive(Deserialize)]
struct KeyCase {
    name: String,
    key: String,
    base: String,
    cell: Option<u16>,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/cell-keys.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the corpus")).expect("it parses")
}

#[test]
fn a_kind_is_spelled_as_the_corpus_says() {
    for case in corpus().kinds {
        assert_eq!(parameter_key(&case.kind), case.key, "{}", case.name);
    }
}

#[test]
fn a_key_is_read_back_as_the_corpus_says() {
    for case in corpus().keys {
        let (base, cell) = split_cell(&case.key);
        assert_eq!((base, cell), (case.base.as_str(), case.cell), "{}", case.name);
    }
}
