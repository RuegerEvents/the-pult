//! Importing other people's MVR files.
//!
//! `#[ignore]`d, because the corpus is gitignored — see `pult-mvr`'s own corpus test.
//! This is the half that needs a schema: what the reader produced, turned into rows
//! this console would actually store.
//!
//! Planning is a pure function, so this needs no station: bytes and an empty show go
//! in, and every write the import would make comes out. Which is also the point — a
//! file that would fail leaves nothing behind because nothing is stored until the
//! whole plan exists.
//!
//! ```text
//! PULT_MVR_SAMPLES=~/mvr-corpus scripts/fetch-interop-corpus.sh
//! cargo test -p pult-backend --test mvr_corpus -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use pult_backend::infra::interop::mvr::{plan_import, Existing};
use pult_schema::types::fixture::Fixture;
use pult_schema::types::scene::{Layer, SceneObject};

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/corpus/mvr");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("mvr"))
        })
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            std::fs::read(entry.path()).ok().map(|bytes| (name, bytes))
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no MVR files in testdata/corpus/mvr — run scripts/fetch-interop-corpus.sh \
         with PULT_MVR_SAMPLES set to a directory of .mvr files",
    );
    files
}

/// The rows a plan would write, by collection.
fn rows(plan: &pult_backend::infra::interop::apply::ImportPlan) -> BTreeMap<String, Vec<&serde_json::Value>> {
    let mut by_table: BTreeMap<String, Vec<&serde_json::Value>> = BTreeMap::new();
    for (path, _, value) in &plan.writes {
        if let Some(pult_schema::path::PathSegment::Key(table)) = path.first() {
            by_table.entry(table.clone()).or_default().push(value);
        }
    }
    by_table
}

#[test]
#[ignore = "needs testdata/corpus/mvr"]
fn every_real_file_becomes_a_rig_this_console_could_store() {
    for (name, bytes) in corpus() {
        let plan = plan_import(&bytes, &Existing::default())
            .unwrap_or_else(|e| panic!("{name} does not plan: {e}"));
        let by_table = rows(&plan);

        println!("\n{name}");
        for (table, values) in &by_table {
            println!("  {table}: {}", values.len());
        }
        println!("  assets: {}", plan.assets.len());
        for warning in &plan.report.warnings {
            println!("    ! {warning}");
        }

        // Every fixture is patched to a type the same plan writes, in a mode that
        // type has, at an address somebody could read off a plot.
        let types: Vec<uuid::Uuid> = by_table
            .get("fixture_types")
            .map(|values| {
                values
                    .iter()
                    .map(|v| serde_json::from_value::<pult_schema::types::fixture::FixtureType>((*v).clone()).unwrap().id)
                    .collect()
            })
            .unwrap_or_default();

        let fixtures: Vec<Fixture> = by_table
            .get("fixtures")
            .map(|values| {
                values.iter().map(|v| serde_json::from_value((*v).clone()).unwrap()).collect()
            })
            .unwrap_or_default();
        assert!(!fixtures.is_empty(), "{name} has no fixtures");

        for fixture in &fixtures {
            assert!(
                types.contains(&fixture.fixture_type_id),
                "{name}: {} is patched to a type this import does not write",
                fixture.name,
            );
            assert!(fixture.position.is_some(), "{name}: {} has no place", fixture.name);
            for span in fixture.address.breaks() {
                assert!(
                    span.universe >= 1 && (1..=512).contains(&span.address),
                    "{name}: {} is at {}/{}",
                    fixture.name,
                    span.universe,
                    span.address,
                );
            }
        }

        // And every parent named is an object the same plan writes, or the object is
        // at the top of its layer. A dangling parent would put lights at the origin.
        let objects: Vec<SceneObject> = by_table
            .get("scene_objects")
            .map(|values| {
                values.iter().map(|v| serde_json::from_value((*v).clone()).unwrap()).collect()
            })
            .unwrap_or_default();
        let object_ids: Vec<uuid::Uuid> = objects.iter().map(|o| o.id).collect();
        let layers: Vec<Layer> = by_table
            .get("layers")
            .map(|values| {
                values.iter().map(|v| serde_json::from_value((*v).clone()).unwrap()).collect()
            })
            .unwrap_or_default();
        let layer_ids: Vec<uuid::Uuid> = layers.iter().map(|l| l.id).collect();

        for object in &objects {
            if let Some(parent) = object.parent {
                assert!(object_ids.contains(&parent), "{name}: {} hangs off nothing", object.name);
            }
            assert!(object.layer.is_some_and(|l| layer_ids.contains(&l)), "{name}: {} is in no layer", object.name);
        }
        for fixture in &fixtures {
            if let Some(parent) = fixture.parent {
                assert!(
                    object_ids.contains(&parent),
                    "{name}: {} hangs off nothing",
                    fixture.name,
                );
            }
        }
    }
}

/// Planning the same file twice against the show the first one made writes no new
/// rows: everything matches by the uuid the file gave it.
#[test]
#[ignore = "needs testdata/corpus/mvr"]
fn a_second_import_of_a_real_file_creates_nothing() {
    for (name, bytes) in corpus() {
        let first = plan_import(&bytes, &Existing::default()).expect("plans");
        let by_table = rows(&first);
        let of = |table: &str| -> Vec<serde_json::Value> {
            by_table.get(table).map(|v| v.iter().map(|v| (*v).clone()).collect()).unwrap_or_default()
        };
        fn parse<T: serde::de::DeserializeOwned>(values: Vec<serde_json::Value>) -> Vec<T> {
            values.into_iter().map(|v| serde_json::from_value(v).unwrap()).collect()
        }

        let fixture_types = parse(of("fixture_types"));
        let fixtures = parse(of("fixtures"));
        let scene_objects = parse(of("scene_objects"));
        let layers = parse(of("layers"));
        let symbols = parse(of("symbols"));
        let classes = parse(of("classes"));
        let named_assets = parse(of("named_assets"));
        let existing = Existing {
            fixture_types: &fixture_types,
            fixtures: &fixtures,
            scene_objects: &scene_objects,
            layers: &layers,
            symbols: &symbols,
            classes: &classes,
            named_assets: &named_assets,
        };

        let second = plan_import(&bytes, &existing).expect("plans again");

        println!(
            "{name}: {} created and {} updated the second time",
            second.report.created, second.report.updated,
        );
        assert_eq!(second.report.created, 0, "{name} would double its rig on re-import");
        assert_eq!(second.report.updated, first.report.created, "{name} lost rows on re-import");
        assert!(second.report.missing.is_empty(), "{name}: {:?}", second.report.missing);
    }
}

/// A real drawing, written back out and read again, is the same rig.
///
/// The corpus version of the round trip: the small checked-in file proves the rule,
/// and this proves it against files with ninety-five symbols, twenty-eight layers and
/// a fixture definition whose name does not survive a zip's central directory.
#[test]
#[ignore = "needs testdata/corpus/mvr"]
fn every_real_file_survives_being_written_back_out() {
    use pult_backend::infra::interop::mvr::{plan_export, Rig};
    use std::collections::{BTreeMap, BTreeSet};

    for (name, bytes) in corpus() {
        let plan = plan_import(&bytes, &Existing::default()).expect("plans");
        let by_table = rows(&plan);
        let of = |table: &str| -> Vec<serde_json::Value> {
            by_table.get(table).map(|v| v.iter().map(|v| (*v).clone()).collect()).unwrap_or_default()
        };
        fn parse<T: serde::de::DeserializeOwned>(values: Vec<serde_json::Value>) -> Vec<T> {
            values.into_iter().map(|v| serde_json::from_value(v).unwrap()).collect()
        }

        let fixture_types = parse(of("fixture_types"));
        let fixtures: Vec<Fixture> = parse(of("fixtures"));
        let scene_objects: Vec<SceneObject> = parse(of("scene_objects"));
        let layers: Vec<Layer> = parse(of("layers"));
        let symbols = parse(of("symbols"));
        let classes = parse(of("classes"));
        let named_assets = parse(of("named_assets"));
        let rig = Rig {
            fixture_types: &fixture_types,
            fixtures: &fixtures,
            scene_objects: &scene_objects,
            layers: &layers,
            symbols: &symbols,
            classes: &classes,
            named_assets: &named_assets,
        };

        // The assets the import would have stored, by their hash, so the export can
        // be given the files it asks for without a station in the way.
        let stored: BTreeMap<String, Vec<u8>> = plan
            .assets
            .iter()
            .map(|(_, bytes)| (pult_backend::infra::assets::digest(bytes), bytes.clone()))
            .collect();

        let export = plan_export(&rig, &BTreeSet::new());
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for want in &export.wanted {
            if let Some(sha) = &want.asset {
                if let Some(bytes) = stored.get(sha) {
                    files.insert(want.name.clone(), bytes.clone());
                }
            }
        }
        let written = pult_backend::infra::interop::mvr::export::write(&export, files)
            .unwrap_or_else(|e| panic!("{name} does not export: {e}"));

        let again = plan_import(&written, &Existing::default())
            .unwrap_or_else(|e| panic!("{name} does not re-import: {e}"));
        let back = rows(&again);

        println!(
            "{name}: {} fixtures out, {} back",
            fixtures.len(),
            back.get("fixtures").map_or(0, |v| v.len()),
        );
        for (table, values) in &by_table {
            // `named_assets` is the one that legitimately shrinks: a name whose bytes
            // this station holds is written back, and a texture nothing references is
            // not carried into the export at all.
            if table == "named_assets" {
                continue;
            }
            assert_eq!(
                back.get(table).map_or(0, |v| v.len()),
                values.len(),
                "{name}: {table} changed on the way out and back",
            );
        }

        // And the fixtures come back at the same addresses, in the same modes.
        let back_fixtures: Vec<Fixture> = parse(
            back.get("fixtures").map(|v| v.iter().map(|v| (*v).clone()).collect()).unwrap_or_default(),
        );
        let key = |f: &Fixture| (f.id, f.address.clone(), f.fixture_type_id);
        let mut before: Vec<_> = fixtures.iter().map(key).collect();
        let mut after: Vec<_> = back_fixtures.iter().map(key).collect();
        before.sort_by_key(|(id, _, _)| *id);
        after.sort_by_key(|(id, _, _)| *id);
        assert_eq!(before, after, "{name}: the patch changed on the way out and back");
    }
}
