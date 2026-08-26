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
