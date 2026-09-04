//! The mounts corpus, from this side.
//!
//! `testdata/mounts.json` is read by this test and by
//! `frontend/src/lib/mount.test.ts`. A mount is resolved on a station when a demo
//! hangs a rig and in a browser on every frame of a drag, and the browser is the one
//! that *writes* it — so the corpus is the only thing keeping the station's own
//! arithmetic equal to the arithmetic that produced the numbers in the show.

use std::collections::HashMap;

use pult_schema::types::fixture::Vec3;
use pult_schema::types::mount::{Chord, Mount};
use pult_schema::types::scene::Transform;
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    #[serde(rename = "chordSets")]
    chord_sets: HashMap<String, Vec<Chord>>,
    transforms: Vec<PlacedCase>,
    nearest: Vec<NearestCase>,
}

#[derive(Deserialize)]
struct PlacedCase {
    name: String,
    chords: String,
    mount: Mount,
    transform: Transform,
}

#[derive(Deserialize)]
struct NearestCase {
    name: String,
    chords: String,
    point: Vec3,
    mount: Mount,
    /// `null` for a piece with nothing to clamp to, which is not a distance at all.
    distance: Option<f32>,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/mounts.json");
    let text = std::fs::read_to_string(path).expect("the corpus is where both suites look");
    serde_json::from_str(&text).expect("the corpus parses")
}

fn close(got: Vec3, want: Vec3, what: &str, case: &str) {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(
        near(got.x, want.x) && near(got.y, want.y) && near(got.z, want.z),
        "{case}: {what} is {got:?}, expected {want:?}",
    );
}

#[test]
fn every_mount_resolves_the_way_the_corpus_says() {
    let corpus = corpus();
    for case in &corpus.transforms {
        let chords = corpus.chord_sets.get(&case.chords).expect("a named chord set");
        let got = case.mount.transform(chords);
        close(got.position, case.transform.position, "position", &case.name);
        close(got.rotation, case.transform.rotation, "rotation", &case.name);
    }
}

#[test]
fn every_point_finds_the_clamp_the_corpus_says() {
    let corpus = corpus();
    for case in &corpus.nearest {
        let chords = corpus.chord_sets.get(&case.chords).expect("a named chord set");
        let (mount, distance) = Mount::nearest(case.point, chords);
        match case.distance {
            None => assert!(
                !distance.is_finite(),
                "{}: a piece with no chords answered a distance of {distance}",
                case.name,
            ),
            Some(wanted) => {
                assert_eq!(mount.chord, case.mount.chord, "{}: the wrong chord", case.name);
                assert!(
                    (mount.along - case.mount.along).abs() < 1e-3,
                    "{}: along is {}",
                    case.name,
                    mount.along,
                );
                assert!(
                    (mount.roll - case.mount.roll).abs() < 1e-3,
                    "{}: roll is {}",
                    case.name,
                    mount.roll,
                );
                assert!(
                    (distance - wanted).abs() < 1e-3,
                    "{}: {distance} away, expected {wanted}",
                    case.name,
                );
            }
        }
    }
}
