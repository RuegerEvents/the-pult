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
async fn a_console() -> (String, Arc<sqlx::SqlitePool>) {
    let (addr, pool, _engine) = a_console_and_its_engine().await;
    (addr, pool)
}

/// The same, with a handle on the engine.
///
/// For the tests that have to put something in the show first. Writing to the pool
/// behind the engine's back would not do: the engine holds the show in memory and
/// answers reads from there, so a row inserted underneath it is a row the routes
/// cannot see — which is the same reason the rest of the console never does it either.
async fn a_console_and_its_engine() -> (String, Arc<sqlx::SqlitePool>, crate::engine::EngineHandle) {
    let pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let (engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool.clone(), None);
    tokio::spawn(engine.run());

    let state = AssetState {
        pool: pool.clone(),
        engine: handle.clone(),
        node_id: NodeId(Uuid::new_v4()),
        // Pointed nowhere and given no disk cache: nothing in this module asks the
        // Share anything, and a client that could reach the real one from a test
        // would be a test that goes to somebody else's server.
        share: infra::interop::share::ShareHandle::with_base("http://127.0.0.1:1", None),
    };
    let app: Router = routes().with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr.to_string(), pool, handle)
}

#[tokio::test]
async fn an_uploaded_plan_comes_back_byte_for_byte() {
    let (addr, _pool) = a_console().await;
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
    let (addr, _pool) = a_console().await;

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
    let (addr, _pool) = a_console().await;

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
    let (addr, _pool) = a_console().await;
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
    let (addr, _pool) = a_console().await;

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
    let (addr, _pool) = a_console().await;

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
    let (addr, pool) = a_console().await;
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

    let types = fixture_types(&pool).await;
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].name, "Test RGBW Mover");
    assert_eq!(types[0].id.to_string(), body["fixture_type_id"].as_str().unwrap());
    assert_eq!(types[0].dmx_modes.len(), 2);
    assert_eq!(types[0].physical.weight_kg, Some(18.5));
}

#[tokio::test]
async fn the_file_itself_is_kept_so_the_row_is_a_reading_of_it_rather_than_a_replacement() {
    use pult_schema::types::fixture::FixtureTypeSource;

    let (addr, pool) = a_console().await;
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

    let types = fixture_types(&pool).await;
    let FixtureTypeSource::Gdtf { asset, .. } = &types[0].source else {
        panic!("an imported type says where it came from: {:?}", types[0].source);
    };
    let stored = assets::get(&pool, asset).await.unwrap().expect("the archive was kept");
    assert_eq!(stored.bytes, bytes, "byte for byte, so a later reader gets more out of it");
    assert_eq!(stored.mime, assets::GDTF_MIME);
}

#[tokio::test]
async fn importing_the_same_fixture_again_updates_the_row_rather_than_making_a_second() {
    let (addr, pool) = a_console().await;
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
        fixture_types(&pool).await.len(),
        1,
        "the file's own id is the row's, so every fixture patched to it follows the revision",
    );
}

#[tokio::test]
async fn a_body_that_is_not_a_gdtf_is_refused_and_leaves_nothing_behind() {
    let (addr, pool) = a_console().await;
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
    assert!(fixture_types(&pool).await.is_empty(), "a refused import stores nothing");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "and no asset either");
}

#[tokio::test]
async fn an_imported_type_exports_as_the_file_it_arrived_in() {
    let (addr, _pool) = a_console().await;
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

    let (addr, _pool, engine) = a_console_and_its_engine().await;
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
    let (addr, _pool) = a_console().await;
    let missing = Uuid::new_v4();
    let answer = reqwest::Client::new()
        .get(format!("http://{addr}/api/export/gdtf/{missing}"))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 404);
}
