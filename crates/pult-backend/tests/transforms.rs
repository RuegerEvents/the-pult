//! The other half of `testdata/transforms.json`: a matrix as a file writes one.
//!
//! `pult-mvr` converts between MVR's millimetre Z-up space and the console's, and
//! `pult-schema` composes placements in the console's. Neither can depend on the
//! other — one is a format library and the other is the data model — so the seam
//! between them is a corpus, read here, where both already are.
//!
//! The `chains` half of the same file is read by `pult-schema` and by the browser.

use pult_schema::types::fixture::Vec3;
use pult_schema::types::scene::Transform;
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    matrices: Vec<MatrixCase>,
}

#[derive(Deserialize)]
struct MatrixCase {
    name: String,
    matrix: String,
    transform: Transform,
}

#[test]
fn every_matrix_becomes_the_placement_the_corpus_says() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/transforms.json");
    let text = std::fs::read_to_string(path).expect("the corpus is where both suites look");
    let corpus: Corpus = serde_json::from_str(&text).expect("the corpus parses");

    for case in corpus.matrices {
        let matrix: pult_mvr::values::MvrMatrix = case
            .matrix
            .parse()
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let got = pult_backend::infra::interop::mvr::placement_as_transform(
            &pult_mvr::transform::decompose(&matrix),
        );

        let near = |a: Vec3, b: Vec3, what: &str| {
            let ok = |x: f32, y: f32| (x - y).abs() < 1e-3;
            assert!(
                ok(a.x, b.x) && ok(a.y, b.y) && ok(a.z, b.z),
                "{}: {what} is {a:?}, expected {b:?}",
                case.name,
            );
        };
        near(got.position, case.transform.position, "position");
        near(got.rotation, case.transform.rotation, "rotation");
        near(got.scale, case.transform.scale, "scale");
    }
}
