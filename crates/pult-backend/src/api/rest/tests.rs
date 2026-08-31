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
    let pool = Arc::new(showfile::open_in_memory().await.unwrap());
    let (engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool.clone(), None);
    tokio::spawn(engine.run());

    let state = AssetState { pool: pool.clone(), engine: handle, node_id: NodeId(Uuid::new_v4()) };
    let app: Router = routes().with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr.to_string(), pool)
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
