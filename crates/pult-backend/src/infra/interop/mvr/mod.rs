//! MVR, read into the show and written back out of it.
//!
//! `pult-mvr` knows the format and nothing about this console; this is the
//! translation. It goes through [`super::apply`] like the GDTF path does, so an
//! import is one gesture, leaves nothing behind if it is refused, and takes itself
//! back if a write fails halfway.

pub mod export;
pub mod plan;

pub use export::{plan_export, Export, Rig};
pub use plan::{plan_import, Existing};

use pult_mvr::transform::Placement;
use pult_schema::types::fixture::Vec3;
use pult_schema::types::scene::Transform;

/// A placement in the console's space, as the schema holds one.
///
/// Two representations of the same three vectors, and the conversion is here rather
/// than in either crate: `pult-mvr` may not know what a `Transform` is, and
/// `pult-schema` may not know what an MVR file is.
pub fn placement_as_transform(placement: &Placement) -> Transform {
    Transform {
        position: vec3(placement.position),
        rotation: vec3(placement.rotation),
        scale: vec3(placement.scale),
    }
}

/// And back, for export.
pub fn transform_as_placement(transform: &Transform) -> Placement {
    Placement {
        position: array(transform.position),
        rotation: array(transform.rotation),
        scale: array(transform.scale),
    }
}

fn vec3([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3 { x, y, z }
}

fn array(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

// ── The whole rig, in and out ─────────────────────────────────────────────────
//
// Both of these were the bodies of the two REST handlers, and they moved here when
// MVR-xchange arrived and needed exactly the same two acts over a socket rather than
// over HTTP. Reading the show, planning, finding the files a plan wants and applying
// the writes is the *interop*, not the transport, and two copies of it would be two
// answers to what an export contains.

use std::collections::{BTreeMap, BTreeSet};

use pult_schema::path::PathSegment;
use uuid::Uuid;

use crate::engine::EngineHandle;
use crate::infra::assets::AssetStore;

async fn read<T: serde::de::DeserializeOwned>(engine: &EngineHandle, table: &str) -> Vec<T> {
    engine
        .get(vec![PathSegment::Key(table.into())])
        .await
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Write the rig as an `.mvr`. Empty `only` is the whole show.
///
/// The plan says which files belong beside the scene; this is where they are found. A
/// fixture type that arrived as a file exports as that file, byte for byte; one the
/// console made for itself exports as a generated one, which is the rule
/// `/api/export/gdtf` already follows.
pub async fn write_rig(
    engine: &EngineHandle,
    assets: &AssetStore,
    only: &BTreeSet<Uuid>,
) -> Result<Vec<u8>, String> {
    let fixture_types: Vec<pult_schema::types::fixture::FixtureType> =
        read(engine, "fixture_types").await;
    let fixtures = read(engine, "fixtures").await;
    let scene_objects = read(engine, "scene_objects").await;
    let layers = read(engine, "layers").await;
    let symbols = read(engine, "symbols").await;
    let classes = read(engine, "classes").await;
    let named_assets = read(engine, "named_assets").await;
    let rig = Rig {
        fixture_types: &fixture_types,
        fixtures: &fixtures,
        scene_objects: &scene_objects,
        layers: &layers,
        symbols: &symbols,
        classes: &classes,
        named_assets: &named_assets,
    };

    let export = plan_export(&rig, only);

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for want in &export.wanted {
        if let Some(sha) = &want.asset {
            if let Ok(Some(stored)) = assets.get(sha).await {
                files.insert(want.name.clone(), stored.bytes);
                continue;
            }
        }
        if let Some(id) = want.fixture_type {
            if let Some(fixture_type) = fixture_types.iter().find(|t| t.id == id) {
                if let Ok((bytes, _)) = super::gdtf::export(assets, fixture_type).await {
                    files.insert(want.name.clone(), bytes);
                }
            }
        }
    }

    export::write(&export, files).map_err(|error| error.to_string())
}

/// Read an `.mvr` into the show, as `user_id`, in one gesture.
///
/// The show is read first — every collection an import matches against — so the plan
/// is built against what is there rather than against what it hopes is.
pub async fn read_rig(
    engine: &EngineHandle,
    assets: &AssetStore,
    bytes: &[u8],
    user_id: Uuid,
) -> Result<super::apply::ImportReport, String> {
    let fixture_types = read(engine, "fixture_types").await;
    let fixtures = read(engine, "fixtures").await;
    let scene_objects = read(engine, "scene_objects").await;
    let layers = read(engine, "layers").await;
    let symbols = read(engine, "symbols").await;
    let classes = read(engine, "classes").await;
    let named_assets = read(engine, "named_assets").await;
    let existing = Existing {
        fixture_types: &fixture_types,
        fixtures: &fixtures,
        scene_objects: &scene_objects,
        layers: &layers,
        symbols: &symbols,
        classes: &classes,
        named_assets: &named_assets,
    };

    let plan = plan_import(bytes, &existing).map_err(|error| error.to_string())?;
    super::apply::apply(plan, assets, engine, user_id).await.map_err(|error| error.to_string())
}
