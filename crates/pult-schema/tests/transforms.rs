//! The transforms corpus, from this side.
//!
//! `testdata/transforms.json` is read by this test and by
//! `frontend/src/lib/scene.test.ts`. Composing a parent chain happens on a station
//! when a group is resolved and in a browser on every frame of a drag, so there are
//! two of it; this is how the two are held to each other.
//!
//! The corpus also has a `matrices` half, which starts from a matrix as an MVR file
//! writes one. That is a seam between `pult-mvr` and this crate rather than between
//! this crate and the browser, so it is read by `pult-backend/tests/transforms.rs` —
//! where the two already meet — and not here.

use std::collections::HashMap;

use pult_schema::types::fixture::Vec3;
use pult_schema::types::scene::{by_id, world_transform, SceneObject, SceneObjectKind, Transform};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct Corpus {
    chains: Vec<ChainCase>,
}

#[derive(Deserialize)]
struct ChainCase {
    name: String,
    objects: Vec<Placed>,
    local: Transform,
    parent: Option<Uuid>,
    world: Transform,
}

/// Only the three fields a chain is composed from; a corpus case should not have to
/// spell a whole `SceneObject` to say where a truss is.
#[derive(Deserialize)]
struct Placed {
    id: Uuid,
    parent: Option<Uuid>,
    transform: Transform,
}

impl Placed {
    fn object(&self) -> SceneObject {
        SceneObject {
            id: self.id,
            name: String::new(),
            kind: SceneObjectKind::Truss,
            transform: self.transform,
            parent: self.parent,
            layer: None,
            class: None,
            geometry: Vec::new(),
            symbol: None,
        }
    }
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/transforms.json");
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

fn same(got: &Transform, want: &Transform, case: &str) {
    close(got.position, want.position, "position", case);
    close(got.rotation, want.rotation, "rotation", case);
    close(got.scale, want.scale, "scale", case);
}

#[test]
fn every_chain_composes_the_way_the_corpus_says() {
    for case in corpus().chains {
        let objects: Vec<SceneObject> = case.objects.iter().map(Placed::object).collect();
        let by = by_id(&objects);
        let got = world_transform(&case.local, case.parent, &by);
        same(&got, &case.world, &case.name);
    }
}

/// A chain with no parents is the placement itself, which is worth asserting rather
/// than assuming: it is the case every unplaced-in-a-drawing fixture takes.
#[test]
fn an_orphan_is_its_own_world() {
    let objects: HashMap<Uuid, &SceneObject> = HashMap::new();
    let local = Transform {
        position: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
        rotation: Vec3 { x: 10.0, y: 20.0, z: 30.0 },
        scale: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
    };

    same(&world_transform(&local, None, &objects), &local, "an orphan");
}

/// A cycle is not supposed to be possible, and a rig view that hangs is worse than a
/// truss drawn in the wrong place.
#[test]
fn a_parent_chain_that_loops_stops_rather_than_hangs() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let placed = |id, parent| SceneObject {
        id,
        name: String::new(),
        kind: SceneObjectKind::Truss,
        transform: Transform::at(Vec3 { x: 1.0, y: 0.0, z: 0.0 }),
        parent: Some(parent),
        layer: None,
        class: None,
        geometry: Vec::new(),
        symbol: None,
    };
    let objects = vec![placed(a, b), placed(b, a)];

    let got = world_transform(&Transform::default(), Some(a), &by_id(&objects));

    // Sixty-four steps of one metre each, and then it gives up.
    assert!(got.position.x > 0.0, "it walked and stopped: {got:?}");
}
