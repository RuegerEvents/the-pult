//! The asset routes, driven over real HTTP.

use std::sync::Arc;

use pult_schema::events::operation::NodeId;
use uuid::Uuid;

use crate::{engine::ShowEngine, infra::showfile};

use super::*;

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89,
];

/// A console serving its asset routes, and the address to reach it on.
async fn a_console() -> (String, OwnAssets) {
    let (addr, assets, _engine) = a_console_and_its_engine().await;
    (addr, assets)
}

/// What the asset routes need, plus what the two show routes need beside them.
///
/// One router serves both, so a test standing it up has to supply both — even though
/// nothing in this module asks a show to travel.
#[derive(Clone)]
struct TestState {
    assets: AssetState,
    shows: ShowsState,
}

impl axum::extract::FromRef<TestState> for AssetState {
    fn from_ref(state: &TestState) -> AssetState {
        state.assets.clone()
    }
}

impl axum::extract::FromRef<TestState> for ShowsState {
    fn from_ref(state: &TestState) -> ShowsState {
        state.shows.clone()
    }
}

/// An asset store with a directory of its own, taken away when the test ends.
///
/// The bytes are files in a bundle now, so a test that stores one has to have
/// somewhere to put it — an in-memory database is only half a store.
struct OwnAssets {
    dir: std::path::PathBuf,
    store: crate::infra::assets::AssetStore,
}

impl std::ops::Deref for OwnAssets {
    type Target = crate::infra::assets::AssetStore;
    fn deref(&self) -> &crate::infra::assets::AssetStore {
        &self.store
    }
}

impl Drop for OwnAssets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The same, with a handle on the engine.
///
/// For the tests that have to put something in the show first. Writing to the pool
/// behind the engine's back would not do: the engine holds the show in memory and
/// answers reads from there, so a row inserted underneath it is a row the routes
/// cannot see — which is the same reason the rest of the console never does it either.
async fn a_console_and_its_engine() -> (String, OwnAssets, crate::engine::EngineHandle) {
    let pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let (engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool.clone(), None);
    tokio::spawn(engine.run());

    let dir = std::env::temp_dir().join(format!("pult-rest-assets-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let assets = OwnAssets {
        store: crate::infra::assets::AssetStore::new(Some(dir.clone()), pool),
        dir,
    };

    let state = TestState {
        shows: ShowsState {
            // Nothing here imports or exports a whole show; those routes have their
            // own tests against a real bundle, and a console with no show open is
            // the honest state for one that is only serving assets.
            shows: crate::ShowsHandle::detached(),
            assets: assets.store.clone(),
        },
        assets: AssetState {
        assets: assets.store.clone(),
        engine: handle.clone(),
        node_id: NodeId(Uuid::new_v4()),
        // Pointed nowhere and given no disk cache: nothing in this module asks the
        // Share anything, and a client that could reach the real one from a test
        // would be a test that goes to somebody else's server.
        share: infra::interop::share::ShareHandle::with_base("http://127.0.0.1:1", None),
        },
    };
    let app: Router = routes().with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr.to_string(), assets, handle)
}

#[tokio::test]
async fn an_uploaded_plan_comes_back_byte_for_byte() {
    let (addr, _assets) = a_console().await;
    let client = reqwest::Client::new();

    let posted = client
        .post(format!("http://{addr}/assets"))
        .header("content-type", "image/png")
        .body(PNG)
        .send()
        .await
        .unwrap();
    assert!(posted.status().is_success());

    let body: serde_json::Value = posted.json().await.unwrap();
    let sha = body["sha256"].as_str().unwrap().to_string();
    assert_eq!(body["byte_len"], PNG.len());

    let fetched = client.get(format!("http://{addr}/assets/{sha}")).send().await.unwrap();
    assert!(fetched.status().is_success());
    assert_eq!(fetched.headers()["content-type"], "image/png");
    assert!(
        fetched.headers()["cache-control"].to_str().unwrap().contains("immutable"),
        "the name is the contents, so the answer can never go stale",
    );
    assert_eq!(fetched.bytes().await.unwrap(), PNG);
}

#[tokio::test]
async fn a_content_type_with_a_charset_on_it_is_still_that_type() {
    let (addr, _assets) = a_console().await;

    let posted = reqwest::Client::new()
        .post(format!("http://{addr}/assets"))
        .header("content-type", "image/png; charset=binary")
        .body(PNG)
        .send()
        .await
        .unwrap();

    assert!(posted.status().is_success());
    let body: serde_json::Value = posted.json().await.unwrap();
    assert_eq!(body["mime"], "image/png");
}

#[tokio::test]
async fn a_type_the_console_will_not_serve_is_refused_with_a_reason() {
    let (addr, _assets) = a_console().await;

    let posted = reqwest::Client::new()
        .post(format!("http://{addr}/assets"))
        .header("content-type", "image/svg+xml")
        .body(PNG)
        .send()
        .await
        .unwrap();

    assert_eq!(posted.status(), 400);
    assert!(posted.text().await.unwrap().contains("image/svg+xml"));
}

#[tokio::test]
async fn a_bundle_is_handed_back_as_an_attachment_and_a_plan_is_not() {
    let (addr, _assets) = a_console().await;
    let client = reqwest::Client::new();

    // A zip does not execute in a browser, so serving one is inert — but it is
    // not a document either, and a link to one should only ever download it.
    let posted = client
        .post(format!("http://{addr}/assets"))
        .header("content-type", crate::infra::assets::BUNDLE_MIME)
        .body(b"PK\x03\x04 not really a zip, but bytes are bytes".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 200);
    let sha = posted.json::<serde_json::Value>().await.unwrap()["sha256"].as_str().unwrap().to_string();

    let got = client.get(format!("http://{addr}/assets/{sha}")).send().await.unwrap();
    assert_eq!(
        got.headers().get("content-disposition").and_then(|v| v.to_str().ok()),
        Some("attachment"),
    );

    let posted = client
        .post(format!("http://{addr}/assets"))
        .header("content-type", "image/png")
        .body(PNG)
        .send()
        .await
        .unwrap();
    let sha = posted.json::<serde_json::Value>().await.unwrap()["sha256"].as_str().unwrap().to_string();

    let got = client.get(format!("http://{addr}/assets/{sha}")).send().await.unwrap();
    assert!(
        got.headers().get("content-disposition").is_none(),
        "a plan is displayed, not downloaded",
    );
}

#[tokio::test]
async fn an_asset_nobody_has_is_a_not_found_rather_than_a_hang() {
    let (addr, _assets) = a_console().await;

    let fetched = reqwest::Client::new()
        .get(format!("http://{addr}/assets/{}", assets::digest(b"never uploaded")))
        .send()
        .await
        .unwrap();

    assert_eq!(fetched.status(), 404);
}

#[tokio::test]
async fn a_relayed_request_is_not_relayed_again() {
    // Without this, three stations that each think the other has an asset would
    // forward one request round between them until something timed out.
    let (addr, _assets) = a_console().await;

    let fetched = reqwest::Client::new()
        .get(format!("http://{addr}/assets/{}", assets::digest(PNG)))
        .header("x-pult-asset-relay", "1")
        .send()
        .await
        .unwrap();

    assert_eq!(fetched.status(), 404);
}

// ── Preferences ───────────────────────────────────────────────────────────────

/// A console serving the config and preferences routes.
async fn a_console_with_preferences() -> String {
    let state = ConfigState {
        node_id: NodeId(Uuid::new_v4()),
        http_port: 7700,
        sync_port: 7701,
        // A console with no show open, which is what one started with no arguments
        // is and what a page reads as "draw the welcome screen".
        show: None,
        shows_dir: None,
    };
    let app: Router = config_routes().with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr.to_string()
}

/// The console's own settings, over HTTP rather than the socket: they are not show
/// data and do not replicate.
#[tokio::test]
async fn preferences_come_back_and_can_be_changed() {
    let _own = crate::infra::preferences::testing::own_file();
    let addr = a_console_with_preferences().await;
    let client = reqwest::Client::new();

    let before: serde_json::Value = client
        .get(format!("http://{addr}/api/preferences"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["historyDepth"], 500, "the default, before anybody says otherwise");

    let answered: serde_json::Value = client
        .put(format!("http://{addr}/api/preferences"))
        .json(&serde_json::json!({ "historyDepth": 1200 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(answered["historyDepth"], 1200);

    let again: serde_json::Value = client
        .get(format!("http://{addr}/api/preferences"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(again["historyDepth"], 1200, "and it stuck");
}

/// Naming one setting must not reset the others: a panel that knows about the
/// history depth and not about the home time would otherwise undo the second every
/// time somebody changed the first.
#[tokio::test]
async fn a_setting_not_named_is_left_alone() {
    let _own = crate::infra::preferences::testing::own_file();
    let addr = a_console_with_preferences().await;
    let client = reqwest::Client::new();

    let with_fade: serde_json::Value = client
        .put(format!("http://{addr}/api/preferences"))
        .json(&serde_json::json!({ "homeFadeMs": 2000 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(with_fade["homeFadeMs"], 2000);
    assert_eq!(with_fade["historyDepth"], 500, "untouched by a write that did not name it");

    let then_depth: serde_json::Value = client
        .put(format!("http://{addr}/api/preferences"))
        .json(&serde_json::json!({ "historyDepth": 750 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(then_depth["historyDepth"], 750);
    assert_eq!(then_depth["homeFadeMs"], 2000, "and the home time survived it");
}

#[tokio::test]
async fn a_home_time_out_of_range_answers_with_what_was_stored() {
    let _own = crate::infra::preferences::testing::own_file();
    let addr = a_console_with_preferences().await;

    let answered: serde_json::Value = reqwest::Client::new()
        .put(format!("http://{addr}/api/preferences"))
        .json(&serde_json::json!({ "homeFadeMs": 10_000_000 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(answered["homeFadeMs"], answered["homeFadeMsMax"]);
}

/// A number outside what the console will do comes back at the nearest one that is,
/// rather than being refused or stored as asked. The answer is what was stored, so a
/// panel showing it is showing the truth.
#[tokio::test]
async fn a_depth_out_of_range_answers_with_what_was_stored() {
    let _own = crate::infra::preferences::testing::own_file();
    let addr = a_console_with_preferences().await;

    let answered: serde_json::Value = reqwest::Client::new()
        .put(format!("http://{addr}/api/preferences"))
        .json(&serde_json::json!({ "historyDepth": 1 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(answered["historyDepth"], answered["historyDepthMin"]);
}

#[tokio::test]
async fn a_preference_that_is_not_a_number_is_refused() {
    let _own = crate::infra::preferences::testing::own_file();
    let addr = a_console_with_preferences().await;

    let status = reqwest::Client::new()
        .put(format!("http://{addr}/api/preferences"))
        .json(&serde_json::json!({ "historyDepth": "lots" }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
}

// ── GDTF, in and out ──────────────────────────────────────────────────────────

/// One of the checked-in fixture definitions, zipped the way a browser would send it.
fn a_gdtf(name: &str) -> Vec<u8> {
    use std::io::Write;

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/gdtf")
        .join(name);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    for path in files {
        writer
            .start_file(path.file_name().unwrap().to_string_lossy().into_owned(), options)
            .unwrap();
        writer.write_all(&std::fs::read(&path).unwrap()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn fixture_types(pool: &sqlx::SqlitePool) -> Vec<pult_schema::types::fixture::FixtureType> {
    pult_schema::db::get_all(pool).await.unwrap()
}

#[tokio::test]
async fn importing_a_gdtf_patches_the_show_with_the_fixture_it_describes() {
    let (addr, assets) = a_console().await;
    let client = reqwest::Client::new();

    let posted = client
        .post(format!("http://{addr}/api/import/gdtf"))
        .header("content-type", assets::GDTF_MIME)
        .body(a_gdtf("rgbw-two-mode"))
        .send()
        .await
        .unwrap();
    assert!(posted.status().is_success(), "{:?}", posted.text().await);

    let body: serde_json::Value = posted.json().await.unwrap();
    assert_eq!(body["replaced"], false);
    assert_eq!(body["warnings"].as_array().unwrap().len(), 0);

    // The engine writes through a channel, so give it a moment to land.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let types = fixture_types(assets.pool()).await;
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].name, "Test RGBW Mover");
    assert_eq!(types[0].id.to_string(), body["fixture_type_id"].as_str().unwrap());
    assert_eq!(types[0].dmx_modes.len(), 2);
    assert_eq!(types[0].physical.weight_kg, Some(18.5));
}

#[tokio::test]
async fn the_file_itself_is_kept_so_the_row_is_a_reading_of_it_rather_than_a_replacement() {
    use pult_schema::types::fixture::FixtureTypeSource;

    let (addr, assets) = a_console().await;
    let bytes = a_gdtf("rgbw-two-mode");
    let client = reqwest::Client::new();
    client
        .post(format!("http://{addr}/api/import/gdtf"))
        .header("content-type", assets::GDTF_MIME)
        .body(bytes.clone())
        .send()
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let types = fixture_types(assets.pool()).await;
    let FixtureTypeSource::Gdtf { asset, .. } = &types[0].source else {
        panic!("an imported type says where it came from: {:?}", types[0].source);
    };
    let stored = assets.get(asset).await.unwrap().expect("the archive was kept");
    assert_eq!(stored.bytes, bytes, "byte for byte, so a later reader gets more out of it");
    assert_eq!(stored.mime, assets::GDTF_MIME);
}

#[tokio::test]
async fn importing_the_same_fixture_again_updates_the_row_rather_than_making_a_second() {
    let (addr, assets) = a_console().await;
    let client = reqwest::Client::new();
    for _ in 0..2 {
        let posted = client
            .post(format!("http://{addr}/api/import/gdtf"))
            .header("content-type", assets::GDTF_MIME)
            .body(a_gdtf("rgbw-two-mode"))
            .send()
            .await
            .unwrap();
        assert!(posted.status().is_success());
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    assert_eq!(
        fixture_types(assets.pool()).await.len(),
        1,
        "the file's own id is the row's, so every fixture patched to it follows the revision",
    );
}

#[tokio::test]
async fn a_body_that_is_not_a_gdtf_is_refused_and_leaves_nothing_behind() {
    let (addr, assets) = a_console().await;
    let client = reqwest::Client::new();

    let posted = client
        .post(format!("http://{addr}/api/import/gdtf"))
        .header("content-type", assets::GDTF_MIME)
        .body(PNG)
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 400);
    assert!(posted.text().await.unwrap().contains("not a GDTF"));

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(fixture_types(assets.pool()).await.is_empty(), "a refused import stores nothing");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets")
        .fetch_one(assets.pool())
        .await
        .unwrap();
    assert_eq!(count, 0, "and no asset either");
}

#[tokio::test]
async fn an_imported_type_exports_as_the_file_it_arrived_in() {
    let (addr, _assets) = a_console().await;
    let bytes = a_gdtf("rgbw-two-mode");
    let client = reqwest::Client::new();

    let body: serde_json::Value = client
        .post(format!("http://{addr}/api/import/gdtf"))
        .header("content-type", assets::GDTF_MIME)
        .body(bytes.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let id = body["fixture_type_id"].as_str().unwrap();

    let exported = client.get(format!("http://{addr}/api/export/gdtf/{id}")).send().await.unwrap();
    assert!(exported.status().is_success());
    assert_eq!(exported.headers()["content-type"], assets::GDTF_MIME);
    let disposition = exported.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(disposition.starts_with("attachment;"), "a zip is never a document: {disposition}");
    assert!(disposition.contains("Pult@Test RGBW Mover.gdtf"), "{disposition}");
    assert_eq!(
        exported.bytes().await.unwrap(),
        bytes,
        "the archive is the record; a re-derived approximation of it would be worse",
    );
}

#[tokio::test]
async fn a_type_this_console_made_for_itself_still_exports_as_something_openable() {
    use pult_schema::types::fixture::{
        FixtureType, ParameterDefinition, ParameterKind, ParameterValue,
    };

    let (addr, _assets, engine) = a_console_and_its_engine().await;
    let fixture_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Hand Made".into(),
        manufacturer: "Nobody".into(),
        channel_count: 4,
        parameters: vec![
            ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0)),
            ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0)),
        ],
        ..FixtureType::default()
    };
    engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("fixture_types".into()),
                pult_schema::path::PathSegment::Key("__create".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            serde_json::to_value(&fixture_type).unwrap(),
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let exported = client
        .get(format!("http://{addr}/api/export/gdtf/{}", fixture_type.id))
        .send()
        .await
        .unwrap();
    assert!(exported.status().is_success());
    let bytes = exported.bytes().await.unwrap();

    // And another console can open it — which is the whole point of generating one.
    let read = pult_gdtf::GdtfFile::parse(&bytes).expect("a real GDTF");
    let gdtf = &read.description.fixture_type;
    assert_eq!(gdtf.name, "Hand Made");
    assert_eq!(
        gdtf.fixture_type_id.to_lowercase(),
        fixture_type.id.to_string(),
        "the console's own id, so exporting and re-importing lands on this row",
    );
    let mode = &gdtf.dmx_modes.items[0];
    assert_eq!(
        pult_gdtf::resolve::footprint(gdtf, mode),
        vec![4],
        "a dimmer and three colour channels",
    );
    assert!(pult_gdtf::validate::check(gdtf).is_empty());
}

#[tokio::test]
async fn asking_for_a_fixture_type_nobody_has_is_a_not_found() {
    let (addr, _assets) = a_console().await;
    let missing = Uuid::new_v4();
    let answer = reqwest::Client::new()
        .get(format!("http://{addr}/api/export/gdtf/{missing}"))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 404);
}

// ── MVR ───────────────────────────────────────────────────────────────────────

/// An `.mvr` built from a checked-in scene, the fixture definitions it names, and
/// whatever meshes the test wants in it.
///
/// The GDTFs come from `testdata/gdtf/` under the names the scene calls them, which is
/// how a small file gets to be about real fixture definitions rather than about
/// placeholders.
fn an_mvr(scene: &str, gdtfs: &[(&str, &str)], resources: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;

    let scene_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/mvr").join(scene);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    writer.start_file("GeneralSceneDescription.xml", options).unwrap();
    writer
        .write_all(
            &std::fs::read(&scene_path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", scene_path.display())),
        )
        .unwrap();

    for (entry, from) in gdtfs {
        writer.start_file(*entry, options).unwrap();
        writer.write_all(&a_gdtf(from)).unwrap();
    }
    for (entry, bytes) in resources {
        writer.start_file(*entry, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// The small rig, with both its fixture definitions and both its meshes.
fn a_small_rig() -> Vec<u8> {
    an_mvr(
        "a-small-rig.xml",
        &[
            ("Acme@Test RGBW Mover.gdtf", "rgbw-two-mode"),
            ("Acme@Test Dimmer.gdtf", "minimal"),
        ],
        &[("truss-3m.glb", b"glTF-not-really"), ("rostrum.3ds", b"3ds-not-really")],
    )
}

async fn post_mvr(addr: &str, bytes: Vec<u8>) -> serde_json::Value {
    let posted = reqwest::Client::new()
        .post(format!("http://{addr}/api/import/mvr"))
        .header("content-type", assets::MVR_MIME)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(posted.status().is_success(), "{:?}", posted.text().await);
    let body = posted.json().await.unwrap();
    // The engine writes through a channel, so give it a moment to land.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    body
}

#[tokio::test]
async fn importing_an_mvr_patches_the_rig_it_draws() {
    use pult_schema::types::fixture::Fixture;
    use pult_schema::types::scene::{Layer, SceneClass, SceneObject, SceneObjectKind, Symbol};

    let (addr, assets) = a_console().await;

    let body = post_mvr(&addr, a_small_rig()).await;
    assert_eq!(body["warnings"].as_array().unwrap().len(), 0, "{:?}", body["warnings"]);
    assert!(body["missing"].as_array().unwrap().is_empty());

    let layers: Vec<Layer> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    assert_eq!(layers.len(), 2, "a layer per layer");

    let classes: Vec<SceneClass> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    assert_eq!(classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Overhead"]);

    let symbols: Vec<Symbol> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].geometry.len(), 1, "the symbol carries its mesh");
    assert_eq!(symbols[0].geometry[0].file_name, "truss-3m.glb");

    let mut objects: Vec<SceneObject> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    objects.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        objects.iter().map(|o| (o.name.as_str(), o.kind)).collect::<Vec<_>>(),
        vec![("LX1", SceneObjectKind::Truss), ("Rostrum", SceneObjectKind::Object)],
    );

    let truss = objects.iter().find(|o| o.name == "LX1").unwrap();
    assert_eq!(truss.transform.position.y, 6.0, "4 m upstage, 6 m up: Z-up becomes Y-up");
    assert_eq!(truss.transform.position.z, -4.0);
    assert_eq!(truss.symbol, Some(symbols[0].id), "and it instances the symbol");
    assert_eq!(truss.class, Some(classes[0].id));

    let mut fixtures: Vec<Fixture> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    fixtures.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        fixtures.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        vec!["Dimmer 1", "Mover 1"],
    );

    let mover = fixtures.iter().find(|f| f.name == "Mover 1").unwrap();
    assert_eq!(mover.parent, Some(truss.id), "it hangs off the truss");
    assert_eq!(mover.fixture_number, Some(101));
    assert_eq!(mover.unit_number, Some(1));
    assert_eq!(
        mover.address.breaks(),
        vec![pult_schema::types::dmx_mode::DmxBreak { universe: 3, address: 1 }],
        "absolute 1025 is universe 3, channel 1",
    );
    assert_eq!(
        mover.address.mode(),
        Some("Basic"),
        "and the mode it names is one the type really has",
    );

    // Its own numbers put it 2 m along the truss; the truss is what puts it in the air.
    assert_eq!(mover.position.unwrap().position.x, 2.0);
    assert_eq!(mover.position.unwrap().position.y, 0.0);
}

#[tokio::test]
async fn a_mesh_is_stored_under_its_hash_and_findable_by_the_name_the_file_used() {
    use pult_schema::types::scene::NamedAsset;

    let (addr, assets) = a_console().await;
    post_mvr(&addr, a_small_rig()).await;

    let mut named: Vec<NamedAsset> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    named.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        named.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        vec!["rostrum.3ds", "truss-3m.glb"],
        "a name per resource, so a mesh asking for a texture by name can find it",
    );
    assert_eq!(named[0].mime, assets::TDS_MIME);
    assert_eq!(named[1].mime, assets::GLB_MIME);

    // And the bytes are really there, under the hash the row names.
    let fetched = reqwest::get(format!("http://{addr}/assets/{}", named[1].asset)).await.unwrap();
    assert!(fetched.status().is_success());
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), b"glTF-not-really");
}

#[tokio::test]
async fn importing_the_same_drawing_again_updates_it_rather_than_doubling_it() {
    use pult_schema::types::fixture::Fixture;
    use pult_schema::types::scene::SceneObject;

    let (addr, assets) = a_console().await;

    let first = post_mvr(&addr, a_small_rig()).await;
    assert!(first["created"].as_u64().unwrap() > 0);
    assert_eq!(first["updated"], 0);

    let second = post_mvr(&addr, a_small_rig()).await;
    assert_eq!(second["created"], 0, "nothing new the second time");
    assert_eq!(second["updated"], first["created"], "everything matched by its own uuid");

    let fixtures: Vec<Fixture> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    let objects: Vec<SceneObject> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    assert_eq!(fixtures.len(), 2, "two fixtures, not four");
    assert_eq!(objects.len(), 2);
}

/// A fixture whose definition the archive does not carry still lands in the patch.
///
/// The alternative is dropping it, which loses the address, the mode and the place —
/// everything the drawing knew — over a file somebody can supply later.
#[tokio::test]
async fn a_fixture_whose_gdtf_is_missing_gets_a_placeholder_and_a_warning() {
    use pult_schema::types::fixture::{Fixture, FixtureType};

    let (addr, assets) = a_console().await;

    let body = post_mvr(
        &addr,
        an_mvr(
            "a-small-rig.xml",
            &[("Acme@Test Dimmer.gdtf", "minimal")],
            &[("truss-3m.glb", b"glTF"), ("rostrum.3ds", b"3ds")],
        ),
    )
    .await;

    let warnings = body["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("placeholder")),
        "{warnings:?}",
    );

    let types: Vec<FixtureType> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    let placeholder = types.iter().find(|t| t.name == "Test RGBW Mover").unwrap();
    assert_eq!(placeholder.manufacturer, "Acme");
    assert!(placeholder.parameters.is_empty(), "it does not invent what the file does not say");

    let fixtures: Vec<Fixture> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    let mover = fixtures.iter().find(|f| f.name == "Mover 1").unwrap();
    assert_eq!(mover.fixture_type_id, placeholder.id, "the fixture is still patched");
    assert_eq!(
        mover.address.breaks(),
        vec![pult_schema::types::dmx_mode::DmxBreak { universe: 3, address: 1 }],
        "and still at the address the drawing gave it",
    );
}

/// What an earlier import left that this one does not mention is reported, never
/// deleted: somebody may have taken that light out of the drawing on purpose, and
/// somebody else may be relying on it being in the show.
#[tokio::test]
async fn a_fixture_the_new_drawing_drops_is_listed_and_left_alone() {
    use pult_schema::types::fixture::Fixture;

    let (addr, assets) = a_console().await;
    post_mvr(&addr, a_small_rig()).await;

    // The same drawing with the floor light taken out of it.
    let without = {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/mvr/a-small-rig.xml");
        let text = std::fs::read_to_string(path).unwrap();
        let start = text.find("<Fixture uuid=\"c0000000-0000-4000-8000-000000000021\"").unwrap();
        let end = text[start..].find("</Fixture>").unwrap() + start + "</Fixture>".len();
        format!("{}{}", &text[..start], &text[end..])
    };
    let bytes = {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("GeneralSceneDescription.xml", options).unwrap();
        writer.write_all(without.as_bytes()).unwrap();
        for (entry, from) in
            [("Acme@Test RGBW Mover.gdtf", "rgbw-two-mode"), ("Acme@Test Dimmer.gdtf", "minimal")]
        {
            writer.start_file(entry, options).unwrap();
            writer.write_all(&a_gdtf(from)).unwrap();
        }
        for entry in ["truss-3m.glb", "rostrum.3ds"] {
            writer.start_file(entry, options).unwrap();
            writer.write_all(b"mesh").unwrap();
        }
        writer.finish().unwrap().into_inner()
    };

    let body = post_mvr(&addr, bytes).await;

    let missing = body["missing"].as_array().unwrap();
    assert!(
        missing.iter().any(|m| m.as_str().unwrap().contains("Dimmer 1")),
        "it says what went: {missing:?}",
    );

    let fixtures: Vec<Fixture> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    assert!(
        fixtures.iter().any(|f| f.name == "Dimmer 1"),
        "and leaves it exactly where it was",
    );
}

#[tokio::test]
async fn a_body_that_is_not_an_mvr_is_refused_and_leaves_nothing_behind() {
    use pult_schema::types::scene::Layer;

    let (addr, assets) = a_console().await;

    let posted = reqwest::Client::new()
        .post(format!("http://{addr}/api/import/mvr"))
        .header("content-type", assets::MVR_MIME)
        .body(b"not a zip at all".to_vec())
        .send()
        .await
        .unwrap();

    assert_eq!(posted.status(), reqwest::StatusCode::BAD_REQUEST);
    let layers: Vec<Layer> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    assert!(layers.is_empty(), "a refused file leaves no rows");
}

/// Import a drawing, export it, and import that into a fresh show: the same rig.
///
/// The strongest thing that can be said about a reader and a writer that are two
/// halves of one crate, and the reason the ids are the file's own: matched by uuid,
/// the second import of an export creates nothing and updates everything.
#[tokio::test]
async fn a_rig_exported_and_imported_again_is_the_same_rig() {
    use pult_schema::types::fixture::Fixture;
    use pult_schema::types::scene::{Layer, SceneObject};

    let (addr, assets) = a_console().await;
    post_mvr(&addr, a_small_rig()).await;

    let exported = reqwest::get(format!("http://{addr}/api/export/mvr")).await.unwrap();
    assert!(exported.status().is_success());
    let bytes = exported.bytes().await.unwrap().to_vec();
    assert!(!bytes.is_empty());

    // A fresh show, and the export read into it.
    let (second_addr, second_assets) = a_console().await;
    let report = post_mvr(&second_addr, bytes.clone()).await;
    assert_eq!(report["warnings"].as_array().unwrap().len(), 0, "{:?}", report["warnings"]);

    let same = |a: &serde_json::Value, b: &serde_json::Value, what: &str| {
        assert_eq!(a, b, "{what} differs between the two shows");
    };
    for (what, first, second) in [
        (
            "fixtures",
            sorted::<Fixture>(assets.pool()).await,
            sorted::<Fixture>(second_assets.pool()).await,
        ),
        (
            "scene objects",
            sorted::<SceneObject>(assets.pool()).await,
            sorted::<SceneObject>(second_assets.pool()).await,
        ),
        ("layers", sorted::<Layer>(assets.pool()).await, sorted::<Layer>(second_assets.pool()).await),
    ] {
        same(&first, &second, what);
    }

    // And a third pass over the same bytes into the second show makes nothing.
    let again = post_mvr(&second_addr, bytes).await;
    assert_eq!(again["created"], 0, "the export of an export is the same rig again");
}

/// Every row of a collection, as JSON sorted by id, for comparing two shows.
async fn sorted<T>(pool: &sqlx::SqlitePool) -> serde_json::Value
where
    T: pult_schema::sql::PultSqlRow + serde::Serialize + Send + Unpin,
{
    let mut rows: Vec<serde_json::Value> = pult_schema::db::get_all::<T>(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| serde_json::to_value(row).unwrap())
        .collect();
    rows.sort_by_key(|row| row["id"].as_str().unwrap_or_default().to_string());
    serde_json::Value::Array(rows)
}

/// Exporting one layer writes that layer and nothing else.
#[tokio::test]
async fn an_export_of_one_layer_carries_only_what_is_in_it() {
    use pult_schema::types::scene::Layer;

    let (addr, assets) = a_console().await;
    post_mvr(&addr, a_small_rig()).await;

    let layers: Vec<Layer> = pult_schema::db::get_all(assets.pool()).await.unwrap();
    let floor = layers.iter().find(|l| l.name == "Floor").unwrap();

    let bytes = reqwest::get(format!("http://{addr}/api/export/mvr?layers={}", floor.id))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec();

    let file = pult_mvr::MvrFile::parse(&bytes).expect("what came back is an mvr");
    assert_eq!(
        file.scene.scene.layers.items.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        vec!["Floor"],
    );
    // And the fixture definition that layer's one fixture needs, and no other.
    let gdtfs: Vec<&String> =
        file.resources.keys().filter(|n| n.ends_with(".gdtf")).collect();
    assert_eq!(gdtfs.len(), 1, "{gdtfs:?}");
}
