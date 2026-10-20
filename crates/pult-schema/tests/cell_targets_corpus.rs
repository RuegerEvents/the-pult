//! `testdata/cell-targets.json`, from this side: where a value set on a selection lands.
//! `frontend/src/lib/programmer.test.ts` reads the same cases against `rowTargets`, so
//! the command line, which asks the station, and the Programmer, which works it out in
//! the browser, put a value on the same keys.

use std::collections::HashMap;

use pult_schema::types::fixture::{parameter_key, FixtureType, ParameterDefinition, ParameterKind};
use pult_schema::types::group::{targets, Element};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct Corpus {
    types: HashMap<String, TypeSpec>,
    fixtures: HashMap<String, String>,
    /// The keys a fixture's patched mode carries; absent is every key.
    #[serde(default)]
    reaches: HashMap<String, Vec<String>>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct TypeSpec {
    cells: u16,
    parameters: Vec<ParamSpec>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParamSpec {
    kind: ParameterKind,
    per_cell: bool,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    kind: ParameterKind,
    elements: Vec<ElementSpec>,
    expected: Vec<String>,
}

#[derive(Deserialize)]
struct ElementSpec {
    fixture: String,
    cell: Option<u16>,
}

fn type_from(spec: &TypeSpec) -> FixtureType {
    let parameters = spec
        .parameters
        .iter()
        .map(|p| {
            let mut definition: ParameterDefinition = serde_json::from_value(serde_json::json!({
                "kind": p.kind,
                "default_value": { "type": "Float", "value": 0.0 },
            }))
            .expect("a minimal parameter definition parses");
            definition.per_cell = p.per_cell;
            definition
        })
        .collect();
    FixtureType { cells: spec.cells, parameters, ..Default::default() }
}

#[test]
fn every_case_in_the_corpus() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/cell-targets.json");
    let corpus: Corpus =
        serde_json::from_str(&std::fs::read_to_string(path).expect("the corpus is there"))
            .expect("the corpus parses");
    let types: HashMap<String, FixtureType> =
        corpus.types.iter().map(|(name, spec)| (name.clone(), type_from(spec))).collect();
    // A uuid per fixture name, and back again for reading a failure.
    let ids: HashMap<String, Uuid> = corpus
        .fixtures
        .keys()
        .map(|name| (name.clone(), Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())))
        .collect();
    let names: HashMap<Uuid, String> = ids.iter().map(|(n, id)| (*id, n.clone())).collect();
    let type_of = |id: &Uuid| types.get(&corpus.fixtures[&names[id]]);

    for case in &corpus.cases {
        let elements: Vec<Element> = case
            .elements
            .iter()
            .map(|e| Element { fixture: ids[&e.fixture], cell: e.cell })
            .collect();
        let reaches = |id: &Uuid, key: &str| {
            corpus.reaches.get(&names[id]).is_none_or(|keys| keys.iter().any(|k| k == key))
        };
        let got: Vec<String> = targets(&case.kind, &elements, type_of, reaches)
            .iter()
            .map(|t| format!("{}/{}", names[&t.fixture_id], parameter_key(&t.parameter_kind)))
            .collect();
        assert_eq!(got, case.expected, "corpus case {:?}", case.name);
    }
}
