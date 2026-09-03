//! Opening a show, which is a console stopping one station and starting another.
//!
//! What these hold on to is the part that is easy to get subtly wrong: the port has
//! to survive the switch, because the address is what an operator typed into the
//! tablet at the back of the room; a copy has to be a *different show to the
//! network*, or two bundles with one id find each other and merge; and closing a
//! show has to leave a console that is still up, since the welcome screen is served
//! over the same socket the show was.

use std::path::{Path, PathBuf};

use pult_backend::{api::rpcs, Config, Console};
use serde_json::{json, Value};

/// A directory of this test's own, taken away when it goes out of scope.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let path = std::env::temp_dir().join(format!("pult-shows-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Dir(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A console with nowhere to keep shows but this test's own directory, and an
/// identity that is not the operator's.
fn a_config(dir: &Dir) -> Config {
    Config {
        port: 0,
        sync_port: 0,
        show: None,
        shows_dir: Some(dir.path().join("shows")),
        identity: Some(dir.path().join("node")),
        ..Config::default()
    }
}

/// What `/api/config` says, which is where a page learns which show it loaded onto.
async fn config_of(port: u16) -> Value {
    reqwest::get(format!("http://127.0.0.1:{port}/api/config"))
        .await
        .expect("the console answers")
        .json()
        .await
        .expect("and answers JSON")
}

/// Give the console a moment to stop one station and start the next, then say what
/// it now has open. Polled rather than slept: how long a station takes to come up
/// depends on the machine, and a constant would be right for exactly one of them.
async fn config_when_settled(port: u16, expect_open: bool) -> Value {
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(response) = reqwest::get(format!("http://127.0.0.1:{port}/api/config")).await {
            if let Ok(config) = response.json::<Value>().await {
                if config["show"].is_null() != expect_open {
                    return config;
                }
            }
        }
    }
    panic!("the console never came back with a show {}", if expect_open { "open" } else { "closed" });
}

/// The same, waiting for a particular show rather than for any.
async fn config_when_open(port: u16, name: &str) -> Value {
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(response) = reqwest::get(format!("http://127.0.0.1:{port}/api/config")).await {
            if let Ok(config) = response.json::<Value>().await {
                if config["show"]["name"] == json!(name) {
                    return config;
                }
            }
        }
    }
    panic!("the console never came back with {name} open");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_console_with_no_show_still_serves_and_can_be_given_one() {
    let dir = Dir::new();
    let console = Console::start(a_config(&dir)).await.expect("a console starts");
    let port = console.http_addr().port();
    let shows = console.shows();
    tokio::spawn(console.serve());

    // Nothing open. This is what a console started with no arguments is, and the
    // whole of how a page knows to draw the welcome screen.
    let before = config_of(port).await;
    assert!(before["show"].is_null(), "{before}");
    assert!(before["showsDir"].is_string(), "and it can say where a new one would go");

    rpcs::open_a_show("show.new", &json!({ "name": "Panto" }), &shows)
        .await
        .expect("a new show is taken");

    let after = config_when_settled(port, true).await;
    assert_eq!(after["show"]["name"], "Panto");
    assert_eq!(
        after["port"], before["port"],
        "the port survives the switch: it is what an operator typed into the tablet",
    );
    assert!(
        after["show"]["path"].as_str().unwrap().ends_with("Panto.pult"),
        "{after}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_is_a_new_show_to_the_network_and_the_original_is_untouched() {
    let dir = Dir::new();
    let mut config = a_config(&dir);
    config.show = Some(dir.path().join("Original.pult"));
    let console = Console::start(config).await.expect("a console starts");
    let port = console.http_addr().port();
    let shows = console.shows();
    tokio::spawn(console.serve());

    let original = PathBuf::from(config_of(port).await["show"]["path"].as_str().unwrap());
    let before = show_id(&original).await;

    rpcs::open_a_show("show.saveAs", &json!({ "name": "Original copy" }), &shows)
        .await
        .expect("the copy is taken");

    // The switch closes the socket, so there is a moment with nothing to ask.
    let copy = PathBuf::from(config_when_open(port, "Original copy").await["show"]["path"]
        .as_str()
        .unwrap());
    assert!(copy.ends_with("Original copy.pult"), "{}", copy.display());

    assert_ne!(
        show_id(&copy).await,
        before,
        "two bundles with one id would find each other over mDNS and merge",
    );
    assert_eq!(show_id(&original).await, before, "and the original is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_show_leaves_a_console_that_is_still_up() {
    let dir = Dir::new();
    let mut config = a_config(&dir);
    config.show = Some(dir.path().join("Briefly.pult"));
    let console = Console::start(config).await.expect("a console starts");
    let port = console.http_addr().port();
    let shows = console.shows();
    tokio::spawn(console.serve());

    assert_eq!(config_of(port).await["show"]["name"], "Briefly");

    rpcs::open_a_show("show.close", &json!({}), &shows).await.expect("closing is taken");

    let after = config_when_settled(port, false).await;
    assert!(after["show"].is_null(), "{after}");
    assert_eq!(after["port"], json!(port), "on the same port, so the tablet is not lost");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_is_what_a_welcome_screen_can_offer() {
    let dir = Dir::new();
    let console = Console::start(a_config(&dir)).await.expect("a console starts");
    let shows = console.shows();
    let port = console.http_addr().port();
    tokio::spawn(console.serve());

    rpcs::open_a_show("show.new", &json!({ "name": "Listed" }), &shows).await.unwrap();
    config_when_settled(port, true).await;

    let listed = rpcs::list_shows(&shows).await.expect("it lists");
    let in_dir = listed["inDir"].as_array().expect("an array");
    assert_eq!(in_dir.len(), 1, "{listed}");
    assert_eq!(in_dir[0]["name"], "Listed");
    assert_eq!(in_dir[0]["fixtures"], 0);
    // Read out of the `show` row, which the engine seeded from the bundle's name and
    // this station's preferences. It used to be a button in the Show panel, so a
    // console nobody had opened a browser onto had no show at all.
    assert!(in_dir[0]["createdAt"].is_string(), "{listed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_is_not_a_show_is_refused_while_somebody_is_still_looking_at_it() {
    // Rather than taken, acted on, and discovered afterwards — which would take the
    // console down and bring it back with nothing open and an error in the log.
    let dir = Dir::new();
    let console = Console::start(a_config(&dir)).await.expect("a console starts");
    let shows = console.shows();

    let err = rpcs::open_a_show("show.open", &json!({ "path": dir.path() }), &shows)
        .await
        .unwrap_err();
    assert!(err.contains("bundle.toml"), "{err}");
}

/// The `show` row's id, read without the engine.
///
/// Polled, because the engine seeds the row on the way up and a station is answering
/// before that write is on the disk — which is the whole point of the writer being
/// off the actor, and is a race a test has to wait out rather than assume away.
async fn show_id(bundle: &Path) -> String {
    use std::str::FromStr;
    for _ in 0..200 {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
            "sqlite:{}?mode=ro",
            bundle.join("show.db").display()
        ))
        .unwrap();
        if let Ok(pool) = sqlx::SqlitePool::connect_with(opts).await {
            let id: Option<String> =
                sqlx::query_scalar("SELECT id FROM show").fetch_optional(&pool).await.ok().flatten();
            pool.close().await;
            if let Some(id) = id {
                return id;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("{} never got a show row", bundle.display());
}

// ── Versions ──────────────────────────────────────────────────────────────────

/// Take a version and wait for its file, which is written on a task of its own.
async fn a_version(station: &pult_backend::Running, name: Option<&str>) -> uuid::Uuid {
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment};

    let args = match name {
        Some(name) => json!({ "name": name }),
        None => json!({}),
    };
    station
        .engine
        .set(
            vec![
                PathSegment::Key("versions".into()),
                PathSegment::Key("__checkpoint".into()),
            ],
            Lifecycle::Persisted,
            args,
        )
        .await
        .expect("the version is taken");

    let versions = versions_of(station).await;
    versions.last().expect("a version row").id
}

async fn versions_of(station: &pult_backend::Running) -> Vec<pult_schema::types::Version> {
    use pult_schema::path::PathSegment;
    let value = station
        .engine
        .get(vec![PathSegment::Key("versions".into())])
        .await
        .expect("the versions read");
    let mut rows: Vec<pult_schema::types::Version> =
        serde_json::from_value(value).unwrap_or_default();
    rows.sort_by_key(|row| row.created_at);
    rows
}

/// Wait until this station says it holds — or no longer holds — a snapshot.
///
/// `versions_here` rather than the file, deliberately, and it is the same
/// distinction the panel makes. `VACUUM INTO` creates its file and *then* fills it,
/// so a file that exists is not yet a database anybody can read; the station
/// publishes what it holds only once the copy has finished.
async fn wait_until_here(station: &pult_backend::Running, id: uuid::Uuid, present: bool) {
    use pult_schema::path::PathSegment;
    for _ in 0..200 {
        let value = station
            .engine
            .get(vec![PathSegment::Key("versions_here".into())])
            .await
            .unwrap_or(Value::Null);
        let here: Vec<uuid::Uuid> = serde_json::from_value(value).unwrap_or_default();
        if here.contains(&id) == present {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("{id} was never {}", if present { "written" } else { "removed" });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_contains_its_own_row() {
    // The rule the whole checkpointer is arranged around. Copy the show before the
    // version's row is durable and the snapshot does not contain the version it is a
    // snapshot of — so every restore quietly forgets the point it restored to.
    let dir = Dir::new();
    let mut config = a_config(&dir);
    let show = dir.path().join("Saved.pult");
    config.show = Some(show.clone());
    let station = pult_backend::start(config).await.expect("a station starts");

    let id = a_version(&station, Some("Before act two")).await;
    wait_until_here(&station, id, true).await;

    let inside = versions_inside(&show.join("versions").join(format!("{id}.db"))).await;
    assert!(
        inside.iter().any(|row| row.id == id),
        "the snapshot knows about itself: {inside:?}",
    );
    assert_eq!(inside[0].name.as_deref(), Some("Before act two"));

    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_version_takes_its_file_with_it() {
    // Which is what Ctrl-Z after an accidental Save has to do: the row is undone
    // like any other create, and the file follows the row.
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment};

    let dir = Dir::new();
    let mut config = a_config(&dir);
    let show = dir.path().join("Undone.pult");
    config.show = Some(show.clone());
    let station = pult_backend::start(config).await.expect("a station starts");

    let id = a_version(&station, None).await;
    wait_until_here(&station, id, true).await;
    let file = show.join("versions").join(format!("{id}.db"));

    station
        .engine
        .set(
            vec![
                PathSegment::Key("versions".into()),
                PathSegment::Id(id),
                PathSegment::Key("__delete".into()),
            ],
            Lifecycle::Persisted,
            Value::Null,
        )
        .await
        .expect("the version is dropped");

    wait_until_here(&station, id, false).await;
    assert!(!file.exists(), "the file goes with the row");
    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_station_says_which_snapshots_it_actually_holds() {
    // The row replicates and the file does not, so the panel can only say "not on
    // this station" because the station publishes what it has.
    use pult_schema::path::PathSegment;

    let dir = Dir::new();
    let mut config = a_config(&dir);
    config.show = Some(dir.path().join("Here.pult"));
    let station = pult_backend::start(config).await.expect("a station starts");

    let id = a_version(&station, None).await;
    let mut here: Vec<uuid::Uuid> = Vec::new();
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let value = station
            .engine
            .get(vec![PathSegment::Key("versions_here".into())])
            .await
            .unwrap_or(Value::Null);
        here = serde_json::from_value(value).unwrap_or_default();
        if here.contains(&id) {
            break;
        }
    }
    assert_eq!(here, vec![id]);

    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_show_is_the_show_as_it_was() {
    let dir = Dir::new();
    let mut config = a_config(&dir);
    let show = dir.path().join("Rewound.pult");
    config.show = Some(show.clone());
    let console = Console::start(config).await.expect("a console starts");
    let port = console.http_addr().port();
    let shows = console.shows();
    let sync = console.sync();
    let engine = console.engine().expect("a station is running");

    // Something to lose, then a point to come back to, then losing it.
    let id = {
        let station_engine = console.engine().expect("a station is running");
        rename_show(&station_engine, "Act One").await;
        let id = a_version_through(&station_engine).await;
        wait_until_taken(&station_engine, id).await;
        rename_show(&station_engine, "Act Two").await;
        id
    };
    tokio::spawn(console.serve());
    assert_eq!(show_name(&show).await, "Act Two");

    rpcs::restore_a_show(id, &shows, sync.as_ref(), &engine).await.expect("nobody else is here");

    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if show_name(&show).await == "Act One" {
            // And the point that was left behind is still reachable: the restore
            // takes a version of what it is about to overwrite.
            assert!(config_of(port).await["show"].is_object(), "and the console is up");
            return;
        }
    }
    panic!(
        "the show never came back as it was; it says {:?}, the snapshot is {},          the console says {}",
        show_name(&show).await,
        show.join("versions").join(format!("{id}.db")).exists(),
        config_of(port).await,
    );
}

/// What the show calls itself, read off the file rather than out of the station:
/// the point of a restore is what is on the disk afterwards.
async fn show_name(bundle: &Path) -> String {
    use std::str::FromStr;
    let Ok(opts) = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
        "sqlite:{}?mode=ro",
        bundle.join("show.db").display()
    )) else {
        return String::new();
    };
    let Ok(pool) = sqlx::SqlitePool::connect_with(opts).await else { return String::new() };
    let name: String =
        sqlx::query_scalar("SELECT name FROM show").fetch_one(&pool).await.unwrap_or_default();
    pool.close().await;
    name
}

async fn rename_show(engine: &pult_backend::engine::EngineHandle, name: &str) {
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment};
    engine
        .set(
            vec![PathSegment::Key("show".into()), PathSegment::Key("name".into())],
            Lifecycle::Persisted,
            json!(name),
        )
        .await
        .expect("the show is renamed");
}

/// The same wait, for a caller that has an engine handle and not a `Running`.
async fn wait_until_taken(engine: &pult_backend::engine::EngineHandle, id: uuid::Uuid) {
    use pult_schema::path::PathSegment;
    for _ in 0..200 {
        let value = engine
            .get(vec![PathSegment::Key("versions_here".into())])
            .await
            .unwrap_or(Value::Null);
        let here: Vec<uuid::Uuid> = serde_json::from_value(value).unwrap_or_default();
        if here.contains(&id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("{id} was never written");
}

async fn a_version_through(engine: &pult_backend::engine::EngineHandle) -> uuid::Uuid {
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment};
    engine
        .set(
            vec![
                PathSegment::Key("versions".into()),
                PathSegment::Key("__checkpoint".into()),
            ],
            Lifecycle::Persisted,
            json!({}),
        )
        .await
        .expect("the version is taken");
    let value = engine
        .get(vec![PathSegment::Key("versions".into())])
        .await
        .expect("the versions read");
    let mut rows: Vec<pult_schema::types::Version> =
        serde_json::from_value(value).unwrap_or_default();
    rows.sort_by_key(|row| row.created_at);
    rows.last().expect("a version").id
}

/// The `versions` rows a snapshot carries about itself.
async fn versions_inside(file: &Path) -> Vec<pult_schema::types::Version> {
    use std::str::FromStr;
    let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
        "sqlite:{}?mode=ro",
        file.display()
    ))
    .unwrap();
    let pool = sqlx::SqlitePool::connect_with(opts).await.expect("the snapshot opens");
    let rows: Vec<pult_schema::types::Version> =
        pult_schema::db::get_all(&pool).await.unwrap_or_default();
    pool.close().await;
    rows
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_this_disk_holds_that_the_show_forgot_gets_its_row_back() {
    // The case a restore always produces: the "Before restoring…" version is taken
    // *after* the database that is about to be put back was written, so its row is
    // not in that database. Without this, the safety net an operator reaches for
    // when the restore was a mistake would be a file with nothing naming it.
    let dir = Dir::new();
    let mut config = a_config(&dir);
    let show = dir.path().join("Orphan.pult");
    config.show = Some(show.clone());

    let station = pult_backend::start(config.clone()).await.expect("a station starts");
    let id = a_version(&station, Some("Kept")).await;
    wait_until_here(&station, id, true).await;
    station.shutdown().await;

    // Take the row out from under the show, leaving the file: what a restore does.
    forget_the_row(&show, id).await;

    let station = pult_backend::start(config).await.expect("it opens again");
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if versions_of(&station).await.iter().any(|row| row.id == id) {
            station.shutdown().await;
            return;
        }
    }
    panic!("the snapshot on this disk never got its row back");
}

#[tokio::test(flavor = "multi_thread")]
async fn both_stations_snapshot_one_save() {
    // The row replicates; the file does not. Each station makes its own copy of its
    // own showfile, which is the only thing either of them could honestly copy.
    let dir = Dir::new();

    let mut here = a_config(&dir);
    here.show = Some(dir.path().join("Booth.pult"));
    here.identity = Some(dir.path().join("booth.node"));
    let booth = pult_backend::start(here).await.expect("a station starts");

    let mut there = a_config(&dir);
    there.show = Some(dir.path().join("Roof.pult"));
    there.identity = Some(dir.path().join("roof.node"));
    let roof = pult_backend::start(there).await.expect("the other starts");

    roof.sync
        .connect_peer(vec![booth.sync_addr], uuid::Uuid::new_v4(), uuid::Uuid::nil())
        .await
        .expect("the two stations connect");
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if booth.sync.peer_count().await > 0 && roof.sync.peer_count().await > 0 {
            break;
        }
    }

    let id = a_version(&booth, Some("Interval")).await;

    wait_until_here(&booth, id, true).await;
    wait_until_here(&roof, id, true).await;

    roof.shutdown().await;
    booth.shutdown().await;
}

/// Delete a `versions` row from a closed show, leaving its file behind.
async fn forget_the_row(bundle: &Path, id: uuid::Uuid) {
    use std::str::FromStr;
    let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
        "sqlite:{}",
        bundle.join("show.db").display()
    ))
    .unwrap();
    let pool = sqlx::SqlitePool::connect_with(opts).await.expect("the show opens");
    sqlx::query("DELETE FROM versions WHERE id = ?")
        .bind(id.to_string())
        .execute(&pool)
        .await
        .expect("the row goes");
    pool.close().await;
}
