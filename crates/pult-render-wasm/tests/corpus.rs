//! The corpus, evaluated natively.
//!
//! `testdata/driven-values.json` is read by this test and by
//! `frontend/src/lib/evaluator.test.ts`, which asks the *wasm* build of this crate the
//! same questions. Between them they are the guard that `values-as-functions` put in
//! place of a TypeScript twin: there is only one implementation of the arithmetic, so
//! what has to be checked is not two implementations agreeing but two compilations of
//! one — and a wasm build that rounds a float differently, or a boundary that packs a
//! colour wrong, fails here rather than on stage.
//!
//! The file lives outside both, because neither owns it.

use std::collections::HashMap;

use pult_render::{
    effect::{RunningEffect, RunningFade},
    value::ParameterValue,
    Driving,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    driving: DrivenBy,
    at: u64,
    expect: Option<ParameterValue>,
}

#[derive(Default, Deserialize)]
struct DrivenBy {
    #[serde(default)]
    programmer: Option<ParameterValue>,
    #[serde(default)]
    effect: Option<RunningEffect>,
    #[serde(default)]
    fade: Option<RunningFade>,
    #[serde(default)]
    home: Option<ParameterValue>,
}

/// Close enough that a difference is a bug rather than a rounding.
///
/// An `f32` carries about seven digits, and the corpus states values to three; the
/// slack is for the last bit of a sine, not for a shape that has changed.
const TOLERANCE: f32 = 1e-3;

fn agree(a: &ParameterValue, b: &ParameterValue) -> bool {
    use ParameterValue::*;
    match (a, b) {
        (Float(x), Float(y)) => (x - y).abs() < TOLERANCE,
        (Color { r: r0, g: g0, b: b0 }, Color { r: r1, g: g1, b: b1 }) => {
            (r0 - r1).abs() < TOLERANCE && (g0 - g1).abs() < TOLERANCE && (b0 - b1).abs() < TOLERANCE
        }
        _ => a == b,
    }
}

#[test]
fn every_case_in_the_corpus_evaluates_to_what_it_says() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/driven-values.json");
    let text = std::fs::read_to_string(path).expect("the corpus is where it says it is");
    let corpus: Corpus = serde_json::from_str(&text).expect("the corpus parses");
    assert!(corpus.cases.len() > 30, "a corpus this small is not guarding anything");

    let mut wrong: Vec<String> = Vec::new();
    let mut seen: HashMap<&str, usize> = HashMap::new();

    for case in &corpus.cases {
        *seen.entry(case.name.as_str()).or_default() += 1;
        let driving = Driving {
            programmer: case.driving.programmer.as_ref(),
            effect: case.driving.effect.as_ref(),
            fade: case.driving.fade.as_ref(),
            home: case.driving.home.as_ref(),
        };
        let got = pult_render::value_at(&driving, case.at);
        let ok = match (&got, &case.expect) {
            (None, None) => true,
            (Some(a), Some(b)) => agree(a, b),
            _ => false,
        };
        if !ok {
            wrong.push(format!("{}: expected {:?}, got {:?}", case.name, case.expect, got));
        }
    }

    assert!(wrong.is_empty(), "{} cases disagree:\n  {}", wrong.len(), wrong.join("\n  "));
    let duplicated: Vec<&&str> = seen.iter().filter(|(_, n)| **n > 1).map(|(k, _)| k).collect();
    assert!(duplicated.is_empty(), "two cases share a name, so a failure names neither: {duplicated:?}");
}
