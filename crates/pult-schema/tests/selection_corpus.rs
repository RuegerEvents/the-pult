//! The corpus, from this side.
//!
//! `testdata/selection-queries.json` is read by this test and by
//! `frontend/src/lib/selection.test.ts`. A selection query has to pick the same
//! fixtures in the same order on a station as in a browser — a group saved on one
//! console and resolved on another is the whole feature — and two evaluators is the
//! price of not putting a round trip inside a drag. This is how the price is paid.

use std::collections::HashMap;

use pult_schema::types::{evaluate, scene::SceneObject, Fixture, SelectionQuery};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct Corpus {
    rig: Vec<Fixture>,
    /// What a fixture may hang off. A light on a truss is where the truss put it, so
    /// a geometric term reads a world position rather than the numbers on the row.
    #[serde(default)]
    scene: Vec<SceneObject>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    query: SelectionQuery,
    /// Absent for a saved group, which has no store behind it; present — even empty —
    /// for a live selection handing over the order an operator dragged.
    #[serde(default)]
    previous: Option<Vec<Uuid>>,
    expected: Vec<Uuid>,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/selection-queries.json");
    let text = std::fs::read_to_string(path).expect("the corpus is where both suites look");
    serde_json::from_str(&text).expect("the corpus parses")
}

#[test]
fn every_case_in_the_corpus() {
    let corpus = corpus();
    let names: HashMap<Uuid, &str> =
        corpus.rig.iter().map(|f| (f.id, f.name.as_str())).collect();
    let say = |ids: &[Uuid]| -> Vec<String> {
        ids.iter()
            .map(|id| names.get(id).map(|n| (*n).to_string()).unwrap_or_else(|| id.to_string()))
            .collect()
    };

    for case in &corpus.cases {
        let got = evaluate(&case.query, &corpus.rig, case.previous.as_deref(), &corpus.scene);
        assert_eq!(
            say(&got),
            say(&case.expected),
            "corpus case {:?} picked the wrong fixtures",
            case.name
        );
    }
}

#[test]
fn the_corpus_is_worth_reading() {
    // A corpus that quietly emptied itself would pass the test above.
    let corpus = corpus();
    assert!(corpus.rig.len() >= 5, "the rig is too small to order meaningfully");
    assert!(corpus.cases.len() >= 15, "the corpus has stopped covering the terms");
    assert!(
        corpus.rig.iter().any(|f| f.position.is_none()),
        "an unplaced fixture is what the geometric terms are tested against"
    );
}
