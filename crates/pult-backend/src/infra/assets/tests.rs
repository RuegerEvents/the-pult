use std::str::FromStr;

use sqlx::sqlite::SqliteConnectOptions;

use super::*;

/// A one-pixel PNG header, so the tests carry a real accepted type.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89,
];

async fn a_store() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    crate::infra::showfile::migrate_for_test(&pool).await.unwrap();
    pool
}

/// A station serving one asset, so a fetch has somewhere real to go.
async fn a_station_serving(body: &'static [u8]) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/assets/{sha}",
            axum::routing::get(move || async move {
                ([(axum::http::header::CONTENT_TYPE, "image/png")], body)
            }),
        );
        let _ = axum::serve(listener, app).await;
    });
    addr.to_string()
}

#[tokio::test]
async fn an_asset_comes_back_as_it_went_in() {
    let pool = a_store().await;

    let sha = put(&pool, "image/png", PNG).await.unwrap();
    let asset = get(&pool, &sha).await.unwrap().expect("it was just stored");

    assert_eq!(asset.bytes, PNG);
    assert_eq!(asset.mime, "image/png");
}

#[tokio::test]
async fn the_name_of_an_asset_is_its_contents() {
    let pool = a_store().await;

    let once = put(&pool, "image/png", PNG).await.unwrap();
    let twice = put(&pool, "image/png", PNG).await.unwrap();

    assert_eq!(once, twice, "the same drawing uploaded twice is one asset");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM assets").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn an_asset_nobody_stored_is_simply_absent() {
    let pool = a_store().await;
    assert!(get(&pool, &digest(b"never uploaded")).await.unwrap().is_none());
}

#[tokio::test]
async fn a_type_the_console_will_not_serve_is_refused() {
    let pool = a_store().await;

    // Not squeamishness about documents: an SVG is a document that can run scripts,
    // and serving one from the console's own origin would let it act as the console.
    for mime in ["image/svg+xml", "text/html", "application/pdf"] {
        assert!(put(&pool, mime, PNG).await.is_err(), "{mime} should not be accepted");
    }
}

#[tokio::test]
async fn an_empty_asset_is_refused() {
    let pool = a_store().await;
    assert!(put(&pool, "image/png", b"").await.is_err());
}

#[tokio::test]
async fn a_peer_answering_with_the_wrong_bytes_is_ignored() {
    // The point of naming an asset after its contents: what comes back over the
    // network is checked against what was asked for.
    let pool = a_store().await;
    let wanted = digest(PNG);
    let addr = a_station_serving(b"not that image").await;

    let got = fetch_from_peers(&pool, &wanted, &[addr]).await.unwrap();

    assert!(got.is_none(), "a peer must not be able to answer with a different asset");
    assert!(get(&pool, &wanted).await.unwrap().is_none(), "and nothing should have been stored");
}

#[tokio::test]
async fn an_asset_is_pulled_from_the_station_that_has_it() {
    let sha = digest(PNG);
    let addr = a_station_serving(PNG).await;
    let pool = a_store().await;

    // The first address is nothing at all, so this also covers a station being down.
    let got = fetch_from_peers(&pool, &sha, &["127.0.0.1:1".into(), addr])
        .await
        .unwrap()
        .expect("the second station has it");

    assert_eq!(got.bytes, PNG);
    assert!(
        get(&pool, &sha).await.unwrap().is_some(),
        "and it is kept, so the next viewer costs no round trip",
    );
}

#[tokio::test]
async fn a_station_with_no_peers_to_ask_says_so_rather_than_failing() {
    let pool = a_store().await;
    assert!(fetch_from_peers(&pool, &digest(PNG), &[]).await.unwrap().is_none());
}

#[tokio::test]
async fn each_kind_of_asset_has_its_own_ceiling() {
    let pool = a_store().await;

    // A drawing and a bundle are not the same size of thing, so one number for
    // both would either refuse a reasonable component or accept a photograph.
    let plan_ceiling = ceiling_for("image/png").expect("a plan is storable");
    let bundle_ceiling = ceiling_for(BUNDLE_MIME).expect("a bundle is storable");
    assert!(bundle_ceiling > plan_ceiling, "wasm is bulkier than a drawing");
    assert_eq!(MAX_BYTES, bundle_ceiling, "the body limit is the widest of them");

    let too_big = vec![0u8; plan_ceiling + 1];
    let err = put(&pool, "image/png", &too_big).await.unwrap_err().to_string();
    assert!(err.contains(&plan_ceiling.to_string()), "{err}");
    assert!(err.contains("image/png"), "the message names the kind that was refused: {err}");

    // The same bytes are within a bundle's ceiling, which is the whole point of
    // the table: the limit follows the kind, not the route.
    assert!(put(&pool, BUNDLE_MIME, &too_big).await.is_ok());
}

#[tokio::test]
async fn a_kind_the_console_does_not_store_is_refused_by_name() {
    let pool = a_store().await;

    let err = put(&pool, "image/svg+xml", PNG).await.unwrap_err().to_string();
    assert!(err.contains("image/svg+xml"), "{err}");
}
