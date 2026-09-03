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
async fn show_id(bundle: &Path) -> String {
    use std::str::FromStr;
    let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
        "sqlite:{}?mode=ro",
        bundle.join("show.db").display()
    ))
    .unwrap();
    let pool = sqlx::SqlitePool::connect_with(opts).await.expect("the show opens");
    let id: String =
        sqlx::query_scalar("SELECT id FROM show").fetch_one(&pool).await.expect("a show row");
    pool.close().await;
    id
}
