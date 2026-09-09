//! The corpus, from this side.
//!
//! `testdata/tracking.json` is read by this test and by
//! `frontend/src/lib/cues.test.ts`. "A cue is the stack up to it" decides what a Go
//! asserts, what a rendered viewport on a sheet draws, and — since the cue sheet
//! exists — what colour every cell of the fixture sheet is. The browser cannot ask
//! the station for that: a cue clicked in a list has to colour a sheet in the same
//! frame. So there are two implementations of one rule, and this is the price.

use std::collections::HashMap;

use pult_schema::types::cue::{tracked_through, Cue};
use pult_schema::types::fixture::{parameter_key, ParameterKind, ParameterValue};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct Corpus {
    cues: Vec<Cue>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    /// The cue ids a sequence lists, up to and including the one being looked at.
    through: Vec<Uuid>,
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    fixture: Uuid,
    key: String,
    /// Which cue the surviving capture came from, which is the half a sheet colours by.
    cue: Uuid,
    value: ParameterValue,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/tracking.json");
    let text = std::fs::read_to_string(path).expect("the corpus is where both suites look");
    serde_json::from_str(&text).expect("the corpus parses")
}

#[test]
fn every_case_in_the_corpus() {
    let corpus = corpus();
    let by_id: HashMap<Uuid, &Cue> = corpus.cues.iter().map(|c| (c.id, c)).collect();

    for case in &corpus.cases {
        let got = tracked_through(case.through.iter(), |id| by_id.get(id).copied());
        let said: Vec<(Uuid, String, Uuid, &ParameterValue)> = got
            .iter()
            .map(|(cue, capture)| {
                (
                    capture.fixture_id,
                    parameter_key(&capture.parameter_kind),
                    cue.id,
                    &capture.value,
                )
            })
            .collect();
        let want: Vec<(Uuid, String, Uuid, &ParameterValue)> = case
            .expected
            .iter()
            .map(|e| (e.fixture, e.key.clone(), e.cue, &e.value))
            .collect();
        assert_eq!(said, want, "corpus case {:?} tracked the wrong way", case.name);
    }
}

#[test]
fn the_corpus_is_worth_reading() {
    // A corpus that quietly emptied itself would pass the test above.
    let corpus = corpus();
    assert!(corpus.cases.len() >= 5, "the corpus has stopped covering the rule");
    assert!(
        corpus.cases.iter().any(|c| c.expected.is_empty()),
        "nothing before the first cue is one of the two edges"
    );
    assert!(
        corpus.cues.iter().any(|c| c
            .captures
            .iter()
            .any(|p| matches!(p.parameter_kind, ParameterKind::Pan))),
        "a second parameter of one fixture is what keys-not-fixtures means"
    );
}
