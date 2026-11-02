//! An older showfile, walked up to this build — and only when somebody said to.
//!
//! The file is `testdata/showfiles/generation-9.db`: the Haunt demo as the last
//! generation-9 build seeded it, with one saved workspace written the way that build
//! wrote one (into `layouts`, before it was `workspaces`). Every future step walks
//! the same file, so this suite is what holds "every bump ships a step" to account.

use std::path::{Path, PathBuf};

use pult_backend::{
    api::rpcs,
    infra::showfile::{self, bundle::Bundle, upgrade},
    Config, Console,
};
use pult_schema::path::PathSegment;
use serde_json::{json, Value};

const GENERATION_9: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/showfiles/generation-9.db");
/// The workspace the fixture holds, by the id it was written under.
const WORKSPACE: &str = "5a1d0000-0000-4000-8000-000000000009";

struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let path = std::env::temp_dir().join(format!("pult-upgrade-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Dir(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Preferences of this binary's own, so `recent.toml` is not the operator's.
fn own_preferences() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("pult-upgrade-prefs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("a preferences directory");
        // SAFETY: inside `call_once`, before any station in this process has started.
        unsafe { std::env::set_var("PULT_PREFERENCES", dir.join("preferences.toml")) };
    });
}

/// A bundle holding the generation-9 file.
fn an_old_show(dir: &Dir) -> PathBuf {
    let path = dir.0.join("shows").join("Old.pult");
    let bundle = Bundle::create(&path, "Old").expect("a bundle");
    std::fs::copy(GENERATION_9, bundle.db_path()).expect("the fixture");
    path
}

fn a_config(dir: &Dir, show: Option<PathBuf>) -> Config {
    own_preferences();
    Config {
        port: 0,
        sync_port: 0,
        show,
        shows_dir: Some(dir.0.join("shows")),
        identity: Some(dir.0.join("node")),
        ..Config::default()
    }
}

async fn config_of(port: u16) -> Value {
    reqwest::get(format!("http://127.0.0.1:{port}/api/config")).await.unwrap().json().await.unwrap()
}

async fn when_open(port: u16) -> Value {
    for _ in 0..300 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(response) = reqwest::get(format!("http://127.0.0.1:{port}/api/config")).await {
            if let Ok(config) = response.json::<Value>().await {
                if !config["show"].is_null() {
                    return config;
                }
            }
        }
    }
    panic!("the console never came back with a show open");
}

async fn generation(db: &Path) -> i64 {
    upgrade::generation_of(db).await.expect("a readable file")
}

#[tokio::test]
async fn an_older_file_is_refused_until_somebody_says_to_upgrade_it() {
    let dir = Dir::new();
    let show = an_old_show(&dir);
    let db = show.join("show.db");

    let refused = showfile::open(&db).await.expect_err("an older file is not walked unasked");
    let needs = refused.downcast_ref::<showfile::NeedsUpgrade>().expect("and says why");
    assert_eq!((needs.from, needs.to), (9, upgrade::CURRENT));
    assert_eq!(generation(&db).await, 9, "refusing changed nothing");

    let summary = Bundle::open(&show).unwrap().summary().await;
    assert!(summary.upgradable, "the welcome screen can offer it: {summary:?}");
    assert_eq!(summary.generation, Some(9));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_console_told_to_open_an_older_show_comes_up_with_none() {
    let dir = Dir::new();
    let show = an_old_show(&dir);

    let console = Console::start(a_config(&dir, Some(show.clone()))).await.expect("it still comes up");
    let port = console.http_addr().port();
    let shows = console.shows();
    tokio::spawn(console.serve());
    assert!(config_of(port).await["show"].is_null(), "nobody was there to be asked");
    assert_eq!(generation(&show.join("show.db")).await, 9);

    // Opening it asks, and opens nothing.
    let answer = rpcs::open_a_show("show.open", &json!({ "path": show }), &shows).await.unwrap();
    assert_eq!(answer["needsUpgrade"]["from"], json!(9), "{answer}");
    assert_eq!(answer["needsUpgrade"]["to"], json!(upgrade::CURRENT));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(config_of(port).await["show"].is_null(), "asking is not opening");

    // Said yes.
    let answer =
        rpcs::open_a_show("show.open", &json!({ "path": show, "upgrade": true }), &shows).await.unwrap();
    assert_eq!(answer["ok"], json!(true));
    let config = when_open(port).await;
    assert_eq!(config["show"]["name"], json!("Old"), "{config}");
    assert_eq!(generation(&show.join("show.db")).await, upgrade::CURRENT);
}

#[tokio::test]
async fn an_upgraded_show_keeps_its_rows_and_starts_its_history() {
    let dir = Dir::new();
    let show = an_old_show(&dir);
    let db = show.join("show.db");

    let pool = showfile::open_with(&db, showfile::Upgrade::InPlace).await.expect("it upgrades");

    // The rename: the workspace is where workspaces are, and `layouts` is the new
    // collection, empty.
    let workspaces: Vec<(String, String)> =
        sqlx::query_as("SELECT id, name FROM workspaces").fetch_all(&pool).await.unwrap();
    assert_eq!(workspaces, vec![(WORKSPACE.to_string(), "Programming".to_string())]);
    let layouts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM layouts").fetch_one(&pool).await.unwrap();
    assert_eq!(layouts, 0);
    let ordered: Vec<String> =
        sqlx::query_scalar("SELECT table_name FROM collection_order WHERE entity_id = ?")
            .bind(WORKSPACE)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(ordered, vec!["workspaces".to_string()], "its place in the list moved with it");

    // Everything else is the show it was.
    let fixtures: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fixtures").fetch_one(&pool).await.unwrap();
    assert_eq!(fixtures, 5);

    // History starts here, and the clock survives in the floor.
    let operations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oplog").fetch_one(&pool).await.unwrap();
    assert_eq!(operations, 0, "an old operation replayed would write the old shape back");
    let clock = showfile::oplog::clock(&pool).await.unwrap();
    assert_eq!(clock.0.values().copied().max(), Some(138), "{clock:?}");

    // And the show as it was is a version beside it, at its own generation.
    let versions: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT id, name, automatic FROM versions").fetch_all(&pool).await.unwrap();
    assert_eq!(versions.len(), 1, "{versions:?}");
    let (id, name, automatic) = &versions[0];
    assert_eq!(name, "\"Before upgrading from generation 9\"");
    assert!(automatic);
    let snapshot = show.join("versions").join(format!("{id}.db"));
    assert_eq!(generation(&snapshot).await, 9, "the version is the file as it was");
    pool.close().await;

    // Opened again, it is simply a current show.
    let pool = showfile::open(&db).await.expect("an upgraded file opens unasked");
    pool.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restoring_the_version_from_before_the_upgrade_asks_too() {
    let dir = Dir::new();
    let show = an_old_show(&dir);
    let mut config = a_config(&dir, Some(show.clone()));
    config.upgrade = true;
    let console = Console::start(config).await.expect("it upgrades and opens");
    let shows = console.shows();
    let engine = console.engine().expect("a station");

    let versions = engine.get(vec![PathSegment::Key("versions".into())]).await.unwrap();
    let version: uuid::Uuid =
        serde_json::from_value(versions.as_array().and_then(|v| v.first()).unwrap()["id"].clone()).unwrap();
    let user = pult_schema::types::user::User::DEFAULT_ID;

    let asked = rpcs::restore_a_show(version, false, &shows, &engine, user).await.unwrap();
    assert_eq!(asked["needsUpgrade"]["from"], json!(9), "{asked}");

    let restored = rpcs::restore_a_show(version, true, &shows, &engine, user).await.unwrap();
    assert_eq!(restored["ok"], json!(true), "{restored}");
    let workspaces = engine.get(vec![PathSegment::Key("workspaces".into())]).await.unwrap();
    assert!(workspaces.to_string().contains(WORKSPACE), "the copy was walked up too: {workspaces}");
    assert_eq!(
        generation(&show.join("versions").join(format!("{version}.db"))).await,
        9,
        "and the version itself is untouched"
    );
}

/// A second copy of the same show, in a folder of its own, as the next station holds it.
fn the_same_old_show_again(dir: &Dir) -> PathBuf {
    let path = dir.0.join("elsewhere").join("Old.pult");
    let bundle = Bundle::create(&path, "Old").expect("a bundle");
    std::fs::copy(GENERATION_9, bundle.db_path()).expect("the fixture");
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn one_station_saying_yes_upgrades_the_show_on_the_others() {
    let here = Dir::new();
    let there = Dir::new();

    // The station where somebody said yes.
    let mut config = a_config(&here, Some(an_old_show(&here)));
    config.upgrade = true;
    let upgraded = Console::start(config).await.expect("it upgrades and opens");
    let sync = std::net::SocketAddr::from(([127, 0, 0, 1], upgraded.sync_addr().port()));
    tokio::spawn(upgraded.serve());

    // The station nobody is standing at: it comes up with no show, waiting.
    let show = the_same_old_show_again(&there);
    let waiting = Console::start(a_config(&there, Some(show.clone()))).await.expect("it comes up");
    let port = waiting.http_addr().port();
    assert!(config_of(port).await["show"].is_null());
    let listed = rpcs::list_shows(&waiting.shows()).await.unwrap();
    let said = listed["waitingToUpgrade"].as_str().unwrap_or_default();
    assert!(said.ends_with("elsewhere/Old.pult"), "the welcome screen can say so: {listed}");

    // It finds its leader — a remembered one, here by address — and the show arriving
    // over a link only one build can make is the other station's yes.
    waiting.join_at(sync).await.expect("it joins");
    tokio::spawn(waiting.serve());
    let config = when_open(port).await;
    assert_eq!(config["show"]["name"], json!("Old"), "{config}");
    assert_eq!(generation(&show.join("show.db")).await, upgrade::CURRENT, "upgraded here, unasked");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_station_waiting_on_an_upgrade_upgrades_nothing_by_itself() {
    let dir = Dir::new();
    let show = an_old_show(&dir);
    let console = Console::start(a_config(&dir, Some(show.clone()))).await.expect("it comes up");
    let port = console.http_addr().port();
    tokio::spawn(console.serve());
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert!(config_of(port).await["show"].is_null());
    assert_eq!(generation(&show.join("show.db")).await, 9, "nobody anywhere said yes");
}
