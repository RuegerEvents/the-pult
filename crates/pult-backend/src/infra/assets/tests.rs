use super::*;

/// A one-pixel PNG header, so the tests carry a real accepted type.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89,
];

/// A store with a directory of its own, taken away when the test ends.
///
/// The database is in memory and the bytes are on disk, which is the split the
/// store itself has: the rows say what exists and the files are what it is.
struct OwnStore {
    dir: PathBuf,
    store: AssetStore,
}

impl std::ops::Deref for OwnStore {
    type Target = AssetStore;
    fn deref(&self) -> &AssetStore {
        &self.store
    }
}

impl Drop for OwnStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn a_store() -> OwnStore {
    let pool = Arc::new(crate::infra::showfile::open_in_memory().await.unwrap());
    let dir = std::env::temp_dir().join(format!("pult-assets-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    OwnStore { store: AssetStore::new(Some(dir.clone()), pool), dir }
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
    let store = a_store().await;

    let sha = store.put("image/png", PNG).await.unwrap();
    let asset = store.get(&sha).await.unwrap().expect("it was just stored");

    assert_eq!(asset.bytes, PNG);
    assert_eq!(asset.mime, "image/png");
}

#[tokio::test]
async fn the_name_of_an_asset_is_its_contents() {
    let store = a_store().await;

    let once = store.put("image/png", PNG).await.unwrap();
    let twice = store.put("image/png", PNG).await.unwrap();

    assert_eq!(once, twice, "the same drawing uploaded twice is one asset");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM assets").fetch_one(store.pool()).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn an_asset_nobody_stored_is_simply_absent() {
    let store = a_store().await;
    assert!(store.get(&digest(b"never uploaded")).await.unwrap().is_none());
}

#[tokio::test]
async fn a_type_the_console_will_not_serve_is_refused() {
    let store = a_store().await;

    // Not squeamishness about documents: an SVG is a document that can run scripts,
    // and serving one from the console's own origin would let it act as the console.
    for mime in ["image/svg+xml", "text/html", "application/pdf"] {
        assert!(store.put(mime, PNG).await.is_err(), "{mime} should not be accepted");
    }
}

#[tokio::test]
async fn an_empty_asset_is_refused() {
    let store = a_store().await;
    assert!(store.put("image/png", b"").await.is_err());
}

#[tokio::test]
async fn a_peer_answering_with_the_wrong_bytes_is_ignored() {
    // The point of naming an asset after its contents: what comes back over the
    // network is checked against what was asked for.
    let store = a_store().await;
    let wanted = digest(PNG);
    let addr = a_station_serving(b"not that image").await;

    let got = fetch_from_peers(&store, &wanted, &[addr]).await.unwrap();

    assert!(
        got.asset().is_none(),
        "a peer must not be able to answer with a different asset"
    );
    assert!(store.get(&wanted).await.unwrap().is_none(), "and nothing should have been stored");
}

#[tokio::test]
async fn an_asset_is_pulled_from_the_station_that_has_it() {
    let sha = digest(PNG);
    let addr = a_station_serving(PNG).await;
    let store = a_store().await;

    // The first address is nothing at all, so this also covers a station being down.
    let got = fetch_from_peers(&store, &sha, &["127.0.0.1:1".into(), addr])
        .await
        .unwrap()
        .asset()
        .expect("the second station has it");

    assert_eq!(got.bytes, PNG);
    assert!(
        store.get(&sha).await.unwrap().is_some(),
        "and it is kept, so the next viewer costs no round trip",
    );
}

#[tokio::test]
async fn a_station_with_no_peers_to_ask_says_so_rather_than_failing() {
    let store = a_store().await;
    assert!(matches!(
        fetch_from_peers(&store, &digest(PNG), &[]).await.unwrap(),
        Fetched::NobodyHasIt
    ));
}

/// The two ways of not getting it, told apart.
///
/// "Nobody has it" sends an operator to install the bundle somewhere; "I could not
/// reach that station" sends them to the network. Reporting the second as the first
/// is worse than useless, and it is also what stopped a fetch being worth retrying:
/// a peer that never answered has not said it lacks anything.
#[tokio::test]
async fn a_station_that_could_not_be_reached_is_not_a_station_without_it() {
    let store = a_store().await;

    let answered_no = a_station_serving(b"something else entirely").await;
    assert!(
        matches!(
            fetch_from_peers(&store, &digest(PNG), &[answered_no]).await.unwrap(),
            Fetched::NobodyHasIt
        ),
        "a peer that answered, wrongly, has still answered"
    );

    assert!(
        matches!(
            fetch_from_peers(&store, &digest(PNG), &["127.0.0.1:1".into()]).await.unwrap(),
            Fetched::Unreachable(1)
        ),
        "a peer that could not be asked is counted as such"
    );
}

#[tokio::test]
async fn each_kind_of_asset_has_its_own_ceiling() {
    let store = a_store().await;

    // A drawing and a bundle are not the same size of thing, so one number for
    // both would either refuse a reasonable component or accept a photograph.
    let plan_ceiling = ceiling_for("image/png").expect("a plan is storable");
    let bundle_ceiling = ceiling_for(BUNDLE_MIME).expect("a bundle is storable");
    let gdtf_ceiling = ceiling_for(GDTF_MIME).expect("a fixture definition is storable");
    assert!(bundle_ceiling > plan_ceiling, "wasm is bulkier than a drawing");
    assert!(
        gdtf_ceiling > bundle_ceiling,
        "a moving head's meshes and gobo images are bulkier than either",
    );
    assert_eq!(MAX_BYTES, gdtf_ceiling, "the body limit is the widest of them");

    let too_big = vec![0u8; plan_ceiling + 1];
    let err = store.put("image/png", &too_big).await.unwrap_err().to_string();
    assert!(err.contains(&plan_ceiling.to_string()), "{err}");
    assert!(err.contains("image/png"), "the message names the kind that was refused: {err}");

    // The same bytes are within a bundle's ceiling, which is the whole point of
    // the table: the limit follows the kind, not the route.
    assert!(store.put(BUNDLE_MIME, &too_big).await.is_ok());
}

#[tokio::test]
async fn a_kind_the_console_does_not_store_is_refused_by_name() {
    let store = a_store().await;

    let err = store.put("image/svg+xml", PNG).await.unwrap_err().to_string();
    assert!(err.contains("image/svg+xml"), "{err}");
}

#[tokio::test]
async fn the_bytes_are_a_file_the_snapshots_can_share() {
    // Why they left the database: a version is a `VACUUM INTO` copy of `show.db`,
    // and a 256 MB GDTF inside it would be copied per save. As a file beside it,
    // fifty versions of a show hold one copy of each mesh.
    let store = a_store().await;

    let sha = store.put("image/png", PNG).await.unwrap();

    assert!(store.dir().unwrap().join(&sha).is_file(), "named by its own contents");
    assert!(store.holds(&sha));
    let byte_len: i64 = sqlx::query_scalar("SELECT byte_len FROM assets WHERE sha256 = ?")
        .bind(&sha)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(byte_len as usize, PNG.len(), "and the row still says how big it is");
}

#[tokio::test]
async fn a_row_whose_file_went_missing_is_absent_rather_than_an_error() {
    // What half a copied folder looks like. `None` is the answer that sends the
    // caller down the peer-fetch path, which is the recovery that is wanted.
    let store = a_store().await;
    let sha = store.put("image/png", PNG).await.unwrap();

    std::fs::remove_file(store.dir().unwrap().join(&sha)).unwrap();

    assert!(store.get(&sha).await.unwrap().is_none());
    assert!(!store.holds(&sha));
}

#[tokio::test]
async fn a_console_with_no_show_open_has_nowhere_to_put_anything() {
    let pool = Arc::new(crate::infra::showfile::open_in_memory().await.unwrap());
    let store = AssetStore::closed(pool);

    let err = store.put("image/png", PNG).await.unwrap_err().to_string();
    assert!(err.contains("no show is open"), "{err}");
    assert!(store.get(&digest(PNG)).await.unwrap().is_none());
}

#[tokio::test]
async fn a_name_that_is_not_a_hash_never_reaches_the_filesystem() {
    // A sha comes off a URL path. One with a separator in it would be a way to read
    // whatever else is on the disk.
    let store = a_store().await;
    for name in ["../../etc/passwd", "not-hex", "", &"a".repeat(65)] {
        assert!(store.get(name).await.unwrap().is_none(), "{name}");
        assert!(!store.holds(name), "{name}");
    }
}
