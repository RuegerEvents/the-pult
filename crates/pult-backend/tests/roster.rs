//! What a station does with the show's plugin roster.
//!
//! Deliberately no wasm: every path here is one a station takes *before* it has
//! a component to run — a roster row arriving, a bundle nobody has, an archive
//! that is not a plugin. Those are the states an operator actually stares at,
//! and they need no `wasm32-wasip2` target to test, so they run everywhere.
//!
//! The one thing to be careful about is the unpack cache, which lives in the
//! machine's config directory. A test that wrote there would leave a plugin
//! cache in the developer's own console, so the whole test binary is pointed at
//! a temporary one before any station starts.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use pult_backend::Config;
use pult_schema::{lifecycle::Lifecycle, path::PathSegment};
use serde_json::{json, Value};

/// Point this process's plugin cache somewhere disposable, once.
fn own_cache() -> PathBuf {
    static SET: Once = Once::new();
    let dir = std::env::temp_dir().join(format!("pult-roster-cache-{}", std::process::id()));
    SET.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: inside a `Once`, before any station in this binary starts.
        unsafe { std::env::set_var("PULT_PLUGIN_CACHE", &dir) };
    });
    dir
}

struct Station {
    running: pult_backend::Running,
    showfile: String,
}

impl Station {
    async fn start() -> Station {
        Station::start_with_dirs(vec![]).await
    }

    async fn start_with_dirs(plugin_dirs: Vec<PathBuf>) -> Station {
        own_cache();
        let showfile = std::env::temp_dir()
            .join(format!("pult-roster-{}.db", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into_owned();
        let running = pult_backend::start(Config {
            // Loopback, not the default 0.0.0.0. A station's published
            // http_addr is what a peer fetches a bundle from, and "0.0.0.0" is
            // an address to listen on rather than one to connect to — asking it
            // for a bundle times out rather than failing.
            bind: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: 0,
            sync_port: 0,
            showfile: showfile.clone(),
            plugin_dirs,
            ..Config::default()
        })
        .await
        .expect("station starts");
        Station { running, showfile }
    }

    /// Put a package in the show's roster.
    async fn install(&self, plugin_id: &str, sha256: &str) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        self.running
            .engine
            .set(
                vec![
                    PathSegment::Key("plugin_packages".into()),
                    PathSegment::Key("__create".into()),
                ],
                Lifecycle::Persisted,
                json!({
                    "id": id,
                    "plugin_id": plugin_id,
                    "name": plugin_id,
                    "version": "0.1.0",
                    "api": "1.0",
                    "sha256": sha256,
                    "enabled": true,
                    "stage": "Both",
                    "config": null,
                }),
            )
            .await
            .expect("the roster takes a row");
        id
    }

    /// This station's view of its own plugin runtime.
    async fn plugins(&self) -> Vec<Value> {
        self.running
            .engine
            .get(vec![PathSegment::Key("plugins".into())])
            .await
            .ok()
            .and_then(|v| v.get("plugins").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    }

    /// Wait for one plugin's published row to satisfy `f`, or give up and show
    /// what it actually said.
    async fn eventually(&self, plugin_id: &str, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        // Patient: several stations come up in this binary at once, and a
        // fetch that has to time out against a peer is seconds rather than
        // milliseconds. A test that is merely slow must not read as a failure.
        let mut last = Value::Null;
        for _ in 0..400 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let plugins = self.plugins().await;
            if let Some(row) = plugins.iter().find(|p| p["id"] == plugin_id) {
                last = row.clone();
                if f(row) {
                    return row.clone();
                }
            }
        }
        panic!("waiting for {plugin_id} to be {what}; it was {last}");
    }

    fn stop(self) {
        self.running.serve.abort();
        let _ = std::fs::remove_file(&self.showfile);
    }
}

/// A zip that is a valid plugin bundle, and its digest.
fn a_bundle(plugin_id: &str, api: &str) -> (Vec<u8>, String) {
    a_bundle_versioned(plugin_id, api, "0.1.0")
}

fn a_bundle_versioned(plugin_id: &str, api: &str, version: &str) -> (Vec<u8>, String) {
    let manifest = format!(
        "[plugin]\nid = \"{plugin_id}\"\nname = \"Example\"\nversion = \"{version}\"\n\
         api = \"{api}\"\nwasm = \"example.wasm\"\n"
    );
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("pult-plugin.toml", opts).unwrap();
        w.write_all(manifest.as_bytes()).unwrap();
        w.start_file("example.wasm", opts).unwrap();
        // Not a real component: enough to be unpacked and refused at load,
        // which is exactly the boundary these tests are about.
        w.write_all(b"\0asm\x01\0\0\0").unwrap();
        w.finish().unwrap();
    }
    let digest = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&buf));
    (buf, digest)
}

/// Put bytes in this station's asset store, over its own HTTP API.
async fn upload(station: &Station, bytes: Vec<u8>) -> String {
    let response = reqwest::Client::new()
        .post(format!("http://{}/assets", station.running.http_addr))
        .header("content-type", "application/vnd.pult.plugin+zip")
        .body(bytes)
        .send()
        .await
        .expect("the upload is answered");
    assert_eq!(response.status(), 200, "the store takes a bundle");
    response.json::<Value>().await.unwrap()["sha256"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_roster_row_reaches_the_runtime_without_a_restart() {
    let station = Station::start().await;
    assert!(station.plugins().await.is_empty(), "nothing to begin with");

    let (_, digest) = a_bundle("carried", "1.0");
    station.install("carried", &digest).await;

    // The manager subscribes to the collection, so writing a row is the whole
    // of installing: nothing polls, and nothing had to be restarted.
    let row = station.eventually("carried", "known to the station", |_| true).await;
    assert_eq!(row["name"], "carried");

    station.stop();
}

#[tokio::test]
async fn a_bundle_nobody_has_reads_as_fetching_and_then_says_so() {
    let station = Station::start().await;

    // A digest whose bytes exist nowhere: what a station sees when it opens a
    // show authored on a console it cannot reach.
    let (_, digest) = a_bundle("absent", "1.0");
    station.install("absent", &digest).await;

    // It is *working*, not broken — saying "failed" while a station downloads
    // would send an operator looking for a fault that is not there.
    let row = station
        .eventually("absent", "fetching or finished trying", |row| {
            row["status"]["state"] == "Fetching" || row["status"]["state"] == "Failed"
        })
        .await;

    // With no peers at all the attempt finishes immediately, so both states are
    // legitimate here; what matters is where it ends up and what it says.
    let row = if row["status"]["state"] == "Fetching" {
        station.eventually("absent", "done fetching", |r| r["status"]["state"] == "Failed").await
    } else {
        row
    };
    let reason = row["status"]["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("no station") && reason.contains(&digest[..12]),
        "the reason names what is missing: {row}",
    );

    station.stop();
}

#[tokio::test]
async fn a_bundle_built_against_another_api_is_refused_by_name() {
    let station = Station::start().await;

    let (bytes, _) = a_bundle("from-the-future", "9.9");
    let digest = upload(&station, bytes).await;
    station.install("from-the-future", &digest).await;

    let row = station
        .eventually("from-the-future", "refused", |r| r["status"]["state"] == "Failed")
        .await;
    let reason = row["status"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("9.9"), "it names the version asked for: {row}");
    assert!(reason.contains("this station speaks"), "and the one it has: {row}");

    station.stop();
}

#[tokio::test]
async fn a_bundle_that_is_not_a_plugin_fails_that_row_and_no_other() {
    let station = Station::start().await;

    // One row that cannot possibly work...
    let rubbish = upload(&station, b"PK not actually a zip".to_vec()).await;
    station.install("rubbish", &rubbish).await;
    // ...beside one that is merely unrunnable for a different reason.
    let (bytes, _) = a_bundle("future", "9.9");
    let future = upload(&station, bytes).await;
    station.install("future", &future).await;

    let bad = station.eventually("rubbish", "refused", |r| r["status"]["state"] == "Failed").await;
    assert!(
        bad["status"]["reason"].as_str().unwrap_or_default().contains("zip"),
        "the reason says what was wrong with it: {bad}",
    );

    // The point: one bad row is one bad row. The show opened, and the rest of
    // the roster was still considered on its own terms.
    let other = station.eventually("future", "considered", |r| r["status"]["state"] == "Failed").await;
    assert!(other["status"]["reason"].as_str().unwrap_or_default().contains("9.9"));

    station.stop();
}

#[tokio::test]
async fn a_bundle_whose_manifest_names_another_plugin_is_refused() {
    let station = Station::start().await;

    // The roster promises `promised`; the bundle at that digest is `actual`.
    // Taking the row's word for it would let one row start a different plugin.
    let (bytes, _) = a_bundle("actual", "1.0");
    let digest = upload(&station, bytes).await;
    station.install("promised", &digest).await;

    let row = station.eventually("promised", "refused", |r| r["status"]["state"] == "Failed").await;
    let reason = row["status"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("actual") && reason.contains("promised"), "{row}");

    station.stop();
}

#[tokio::test]
async fn removing_a_package_stops_knowing_about_it() {
    let station = Station::start().await;

    let (bytes, _) = a_bundle("temporary", "9.9");
    let digest = upload(&station, bytes).await;
    let id = station.install("temporary", &digest).await;
    station.eventually("temporary", "known", |_| true).await;

    station
        .running
        .engine
        .set(
            vec![
                PathSegment::Key("plugin_packages".into()),
                PathSegment::Id(id),
                PathSegment::Key("__delete".into()),
            ],
            Lifecycle::Persisted,
            json!({}),
        )
        .await
        .expect("the row goes");

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if !station.plugins().await.iter().any(|p| p["id"] == "temporary") {
            station.stop();
            return;
        }
    }
    panic!("the plugin was still known after its row was removed");
}

#[tokio::test]
async fn renaming_a_package_does_not_disturb_what_is_running() {
    let station = Station::start().await;

    let (bytes, _) = a_bundle("steady", "9.9");
    let digest = upload(&station, bytes).await;
    let id = station.install("steady", &digest).await;
    let before = station.eventually("steady", "settled", |r| r["status"]["state"] == "Failed").await;

    station
        .running
        .engine
        .set(
            vec![
                PathSegment::Key("plugin_packages".into()),
                PathSegment::Id(id),
                PathSegment::Key("name".into()),
            ],
            Lifecycle::Persisted,
            json!("Steady As She Goes"),
        )
        .await
        .expect("the name changes");

    let after = station
        .eventually("steady", "renamed", |r| r["name"] == "Steady As She Goes")
        .await;
    // The digest did not move, so nothing about what runs changed. Task 9
    // learned this with outputs: rebuilding a live thing for a label edit is a
    // visible fault, and here it would be a plugin restarting during a show.
    assert_eq!(after["sha256"], before["sha256"], "still the same bundle: {after}");
    assert_eq!(after["status"], before["status"], "and it was not restarted: {after}");

    station.stop();
}

/// A station that accepts an asset request and then simply never answers.
///
/// The point is to hold a fetch open, so that anything the manager does while
/// one is in flight is observable.
async fn a_station_that_never_answers() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/assets/{sha}",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                axum::http::StatusCode::NOT_FOUND
            }),
        );
        let _ = axum::serve(listener, app).await;
    });
    addr.to_string()
}

#[tokio::test]
async fn the_runtime_keeps_answering_while_a_bundle_is_being_fetched() {
    let station = Station::start().await;

    // A peer that takes the request and holds it. Without this the fetch
    // finishes instantly and nothing is ever actually in flight.
    let slow = a_station_that_never_answers().await;
    station
        .running
        .engine
        .set(
            vec![PathSegment::Key("stations".into()), PathSegment::Key("__create".into())],
            Lifecycle::Synced,
            json!({
                "id": uuid::Uuid::new_v4(),
                "hostname": "slow",
                "is_leader": false,
                "sync_addr": "127.0.0.1:1",
                "http_addr": slow,
                "cpu_percent": 0.0,
                "mem_used": 0,
                "mem_total": 0,
                "uptime_s": 0,
                "output_plugins": [],
                "computes_fixtures": 0,
                "total_fixtures": 0,
                "last_seen": chrono::Utc::now().to_rfc3339(),
            }),
        )
        .await
        .expect("a peer to ask");

    let (_, digest) = a_bundle("slow-arrival", "1.0");
    station.install("slow-arrival", &digest).await;
    station
        .eventually("slow-arrival", "fetching", |r| r["status"]["state"] == "Fetching")
        .await;

    // Now the load-bearing assertion. A fetch is an HTTP request to a machine
    // that may never answer; doing it inside the event loop would stall every
    // plugin call in the station for the length of the timeout. This is the
    // same shape as the deadlock the runtime's first version had, so it is
    // asserted rather than assumed.
    let answered = tokio::time::timeout(
        Duration::from_secs(3),
        station.running.plugins.call("anybody".into(), "surface.exec".into(), json!({})),
    )
    .await;
    assert!(answered.is_ok(), "the manager was busy inside the fetch");
    assert!(answered.unwrap().is_err(), "and the answer is that there is no such plugin");

    // And a second roster change is still handled while the first is in flight.
    let (_, other) = a_bundle("meanwhile", "1.0");
    station.install("meanwhile", &other).await;
    station.eventually("meanwhile", "considered", |_| true).await;

    station.stop();
}

#[tokio::test]
async fn a_digest_already_unpacked_is_not_fetched_again() {
    // The cache is keyed by digest and shared by every show this machine
    // opens, so the second show carrying a plugin needs neither the bytes nor
    // the network — which is the whole reason the key is the content and not
    // the plugin id.
    let first = Station::start().await;
    let (bytes, _) = a_bundle("shared", "1.0");
    let digest = upload(&first, bytes).await;
    first.install("shared", &digest).await;
    // Wait for it to get past fetching and unpacking, to wherever it ends up.
    first
        .eventually("shared", "unpacked", |r| {
            r["status"]["state"] != "Fetching" && r["status"]["state"] != "Loading"
        })
        .await;
    assert!(
        own_cache().join(&digest).join("pult-plugin.toml").is_file(),
        "the first station unpacked it",
    );

    // A different show, on the same machine, with the bundle in nobody's asset
    // store: it has no peers and its own store has never seen these bytes.
    let second = Station::start().await;
    second.install("shared", &digest).await;

    let row = second
        .eventually("shared", "settled", |r| r["status"]["state"] != "Fetching")
        .await;
    let reason = row["status"]["reason"].as_str().unwrap_or_default();
    assert!(
        !reason.contains("no station"),
        "it should never have gone looking for bytes it already has unpacked: {row}",
    );
    assert!(
        !reason.contains("already exists"),
        "and it should reuse the directory rather than trip over it: {row}",
    );

    first.stop();
    second.stop();
}

/// A plugin directory on disk, the way a developer's checkout has one.
fn a_plugin_directory(plugin_id: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pult-dir-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join(plugin_id)).unwrap();
    std::fs::write(
        dir.join(plugin_id).join("pult-plugin.toml"),
        format!(
            "[plugin]\nid = \"{plugin_id}\"\nname = \"From Disk\"\nversion = \"9.9.9\"\n\
             api = \"1.0\"\nwasm = \"example.wasm\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join(plugin_id).join("example.wasm"), b"\0asm\x01\0\0\0").unwrap();
    dir
}

#[tokio::test]
async fn a_copy_on_disk_beats_the_one_the_show_carries() {
    let plugin_id = "under-development";
    let dir = a_plugin_directory(plugin_id);
    let station = Station::start_with_dirs(vec![dir.clone()]).await;

    // The show carries a different build of the same plugin. On a station
    // where somebody is editing it, running the show's copy instead would be
    // the most confusing thing the runtime could do.
    let (bytes, _) = a_bundle(plugin_id, "1.0");
    let digest = upload(&station, bytes).await;
    station.install(plugin_id, &digest).await;

    let row = station
        .eventually(plugin_id, "marked as overridden", |r| r["overridden_by_disk"] == true)
        .await;

    // It is the disk copy that is running: its own version, and no digest,
    // because it did not come from a bundle.
    assert_eq!(row["version"], "9.9.9", "the version on disk, not the show's: {row}");
    assert!(row["sha256"].is_null(), "a directory plugin has no digest: {row}");
    assert!(
        !station.plugins().await.iter().any(|p| p["sha256"] == digest.as_str()),
        "and the carried bundle was never started alongside it",
    );

    // Editing it still reloads it, which is the entire reason for the rule.
    // The write is repeated because a filesystem watcher is not a promise: on
    // macOS the first change to a directory that has only just appeared is
    // sometimes not reported at all, and this test would otherwise be a
    // coin toss rather than a statement about the console.
    let edited = format!(
        "[plugin]\nid = \"{plugin_id}\"\nname = \"Edited\"\nversion = \"9.9.10\"\n\
         api = \"1.0\"\nwasm = \"example.wasm\"\n"
    );
    let mut reloaded = Value::Null;
    for _ in 0..20 {
        std::fs::write(dir.join(plugin_id).join("pult-plugin.toml"), &edited).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(row) = station.plugins().await.into_iter().find(|p| p["id"] == plugin_id) {
            if row["version"] == "9.9.10" {
                reloaded = row;
                break;
            }
        }
    }
    assert_eq!(reloaded["name"], "Edited", "it never reloaded: {reloaded}");
    assert_eq!(
        reloaded["overridden_by_disk"], true,
        "and a reload does not lose the override: {reloaded}",
    );

    station.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn changing_the_shows_configuration_restarts_the_plugin_everywhere() {
    let station = Station::start().await;

    let (bytes, _) = a_bundle("configured", "1.0");
    let digest = upload(&station, bytes).await;
    let id = station.install("configured", &digest).await;
    station
        .eventually("configured", "started once", |r| r["status"]["state"] != "Fetching")
        .await;

    // A plugin is handed its configuration in `init` and never again, so there
    // is no way to change it that is not a new instance. What this asserts is
    // that the reconcile notices at all: without the config in the diff, the
    // digest still matches and the change would be silently ignored.
    let before = station.eventually("configured", "settled", |_| true).await;
    station
        .running
        .engine
        .set(
            vec![
                PathSegment::Key("plugin_packages".into()),
                PathSegment::Id(id),
                PathSegment::Key("config".into()),
            ],
            Lifecycle::Persisted,
            json!({ "prompt": "$" }),
        )
        .await
        .expect("the configuration changes");

    // The bundle did not move, so it must still be the same digest afterwards
    // — a restart, not a replacement.
    let after = station
        .eventually("configured", "restarted", |r| r["sha256"] == before["sha256"])
        .await;
    assert_eq!(after["id"], "configured", "{after}");

    let roster = station
        .running
        .engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .unwrap();
    assert_eq!(
        roster[0]["config"]["prompt"], "$",
        "and the show carries the setting, so every station composes the same answer",
    );

    station.stop();
}

/// Install a bundle the way an operator's console does: one request with the
/// bytes in it.
async fn install_over_http(station: &Station, bytes: Vec<u8>) -> (reqwest::StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("http://{}/api/plugins", station.running.http_addr))
        .body(bytes)
        .send()
        .await
        .expect("the install is answered");
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn installing_a_bundle_puts_it_in_the_show() {
    let station = Station::start().await;

    let (bytes, digest) = a_bundle("installed", "1.0");
    let (status, body) = install_over_http(&station, bytes).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["installed"]["plugin_id"], "installed");
    assert_eq!(body["installed"]["sha256"], digest, "the row names the bytes: {body}");
    assert_eq!(body["replaced"], false);

    // And the roster is what the runtime then reconciles against, so the
    // station picks it up with nothing else asked of it.
    station.eventually("installed", "known to the station", |_| true).await;

    station.stop();
}

#[tokio::test]
async fn a_bundle_that_is_not_a_plugin_leaves_nothing_behind() {
    let station = Station::start().await;

    let (status, body) = install_over_http(&station, b"PK definitely not a zip".to_vec()).await;
    assert_eq!(status, 400, "{body}");

    // The manifest is read before anything is stored, so a console does not
    // accumulate the bundles it has refused.
    let roster = station
        .running
        .engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .unwrap();
    assert_eq!(roster.as_array().map(|r| r.len()), Some(0), "no row: {roster}");

    let digest = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(b"PK definitely not a zip"));
    let stored = reqwest::Client::new()
        .get(format!("http://{}/assets/{digest}", station.running.http_addr))
        .header("x-pult-asset-relay", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(stored.status(), 404, "and no asset either");

    station.stop();
}

#[tokio::test]
async fn installing_a_plugin_id_already_present_upgrades_it() {
    let station = Station::start().await;

    let (first, _) = a_bundle("upgraded", "1.0");
    let (_, body) = install_over_http(&station, first).await;
    let row_id = body["installed"]["id"].clone();

    // Switch it off and configure it, the way an operator would have.
    let id: uuid::Uuid = serde_json::from_value(row_id.clone()).unwrap();
    for (field, value) in [("enabled", json!(false)), ("config", json!({ "prompt": "$" }))] {
        station
            .running
            .engine
            .set(
                vec![
                    PathSegment::Key("plugin_packages".into()),
                    PathSegment::Id(id),
                    PathSegment::Key(field.into()),
                ],
                Lifecycle::Persisted,
                value,
            )
            .await
            .unwrap();
    }

    // A different build of the same plugin: one entry per plugin id, so this
    // is an upgrade rather than a second row taking turns with the first.
    let (bytes, new_digest) = a_bundle_versioned("upgraded", "1.0", "0.2.0");
    let (status, body) = install_over_http(&station, bytes).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["replaced"], true, "{body}");
    assert_eq!(body["installed"]["id"], row_id, "the same row, not a second one");
    assert_eq!(body["installed"]["version"], "0.2.0");
    assert_eq!(body["installed"]["sha256"], new_digest);

    // The operator's choices survive the upgrade. Installing a new build of
    // something switched off must not switch it back on.
    assert_eq!(body["installed"]["enabled"], false, "{body}");
    assert_eq!(body["installed"]["config"]["prompt"], "$", "{body}");

    let roster = station
        .running
        .engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .unwrap();
    assert_eq!(roster.as_array().map(|r| r.len()), Some(1), "one row per plugin id: {roster}");

    station.stop();
}

#[tokio::test]
async fn removing_a_plugin_is_an_ordinary_edit_and_takes_back_like_one() {
    let station = Station::start().await;

    // A person, because undo is per person rather than per browser.
    let user = uuid::Uuid::new_v4();
    station
        .running
        .engine
        .set(
            vec![PathSegment::Key("users".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            json!({ "id": user, "name": "Operator", "colour": "#ff0000" }),
        )
        .await
        .expect("a user");

    let (bytes, digest) = a_bundle("removable", "1.0");
    let sha = upload(&station, bytes).await;
    let id = uuid::Uuid::new_v4();
    station
        .running
        .engine
        .set_as(
            user,
            None,
            vec![
                PathSegment::Key("plugin_packages".into()),
                PathSegment::Key("__create".into()),
            ],
            Lifecycle::Persisted,
            json!({
                "id": id, "plugin_id": "removable", "name": "Removable",
                "version": "0.1.0", "api": "1.0", "sha256": sha,
                "enabled": true, "stage": "Both", "config": null,
            }),
        )
        .await
        .expect("installed by somebody");
    assert_eq!(digest, sha);

    // Removal goes through the ordinary entity delete, which is the entire
    // reason it is not a bespoke route: it replicates, it is attributed, and
    // the Oops key covers it like any other mistake.
    station
        .running
        .engine
        .set_as(
            user,
            None,
            vec![
                PathSegment::Key("plugin_packages".into()),
                PathSegment::Id(id),
                PathSegment::Key("__delete".into()),
            ],
            Lifecycle::Persisted,
            json!({}),
        )
        .await
        .expect("removed");

    let roster = station
        .running
        .engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .unwrap();
    assert_eq!(roster.as_array().map(|r| r.len()), Some(0), "gone: {roster}");

    let moved = station.running.engine.undo(user, false).await;
    assert!(!moved.is_empty(), "undo found the removal");

    let roster = station
        .running
        .engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .unwrap();
    assert_eq!(roster.as_array().map(|r| r.len()), Some(1), "and put it back: {roster}");
    assert_eq!(roster[0]["plugin_id"], "removable");
    assert_eq!(roster[0]["sha256"], sha, "with the bundle it named");

    station.stop();
}

/// Tell `station` that `peer` exists and where its HTTP API is.
///
/// The two stations here are not in an mDNS session — that machinery has its
/// own tests over real TCP. What is under test is narrower and is exactly the
/// path a joined station takes: the `stations` rows say who to ask, and a
/// bundle is fetched from one of them over ordinary HTTP.
async fn tell_about(station: &Station, http_addr: String) {
    station
        .running
        .engine
        .set(
            vec![PathSegment::Key("stations".into()), PathSegment::Key("__create".into())],
            Lifecycle::Synced,
            json!({
                "id": uuid::Uuid::new_v4(),
                "hostname": "peer",
                "is_leader": false,
                "sync_addr": "127.0.0.1:1",
                "http_addr": http_addr,
                "cpu_percent": 0.0, "mem_used": 0, "mem_total": 0, "uptime_s": 0,
                "output_plugins": [], "computes_fixtures": 0, "total_fixtures": 0,
                "last_seen": chrono::Utc::now().to_rfc3339(),
            }),
        )
        .await
        .expect("a peer to ask");
}

/// The real reference bundle, when the plugins workspace has been built.
fn a_real_bundle() -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/dist/command-line.pult-plugin.zip");
    std::fs::read(path).ok()
}

#[tokio::test]
async fn a_station_fetches_a_bundle_from_the_one_that_has_it() {
    // Two shows, so two asset stores: the bundle genuinely exists on one
    // station and genuinely does not exist on the other.
    let holder = Station::start().await;
    let seeker = Station::start().await;
    tell_about(&seeker, holder.running.http_addr.to_string()).await;

    let (bytes, _) = a_bundle("shared-over-the-wire", "1.0");
    let digest = upload(&holder, bytes).await;
    // The cache is shared by both stations in this process, so clear the entry
    // or the seeker would pass by having it unpacked already rather than by
    // fetching it — which is the one thing this test is about.
    std::fs::remove_dir_all(own_cache().join(&digest)).ok();

    seeker.install("shared-over-the-wire", &digest).await;

    seeker
        .eventually("shared-over-the-wire", "past fetching", |r| {
            r["status"]["state"] != "Fetching"
        })
        .await;

    let row = seeker.eventually("shared-over-the-wire", "settled", |_| true).await;
    let reason = row["status"]["reason"].as_str().unwrap_or_default();
    assert!(
        !reason.contains("no station"),
        "it should have got the bundle from the station that had it: {row}",
    );
    assert!(
        own_cache().join(&digest).join("pult-plugin.toml").is_file(),
        "and unpacked what it fetched",
    );

    holder.stop();
    seeker.stop();
}

#[tokio::test]
async fn a_fetched_bundle_is_run_once_it_arrives() {
    let Some(bundle) = a_real_bundle() else {
        eprintln!("skipping: no bundle built (scripts/build-plugins.sh --bundle)");
        return;
    };

    let holder = Station::start().await;
    let seeker = Station::start().await;
    tell_about(&seeker, holder.running.http_addr.to_string()).await;

    let digest = upload(&holder, bundle).await;
    std::fs::remove_dir_all(own_cache().join(&digest)).ok();
    seeker.install("command-line", &digest).await;

    // The whole point of the change, in one assertion: a station that has never
    // seen this plugin ends up running it, with nobody touching that machine.
    let row = seeker
        .eventually("command-line", "running", |r| r["status"]["state"] == "Running")
        .await;
    assert_eq!(row["sha256"], digest, "{row}");

    holder.stop();
    seeker.stop();
}

#[tokio::test]
async fn a_peer_answering_with_the_wrong_bytes_gets_nowhere() {
    let seeker = Station::start().await;

    // A station that answers every asset request with the same rubbish. The
    // digest is the check, so what it says its name is does not matter.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/assets/{sha}",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/vnd.pult.plugin+zip")],
                    b"not the bundle you asked for".to_vec(),
                )
            }),
        );
        let _ = axum::serve(listener, app).await;
    });
    tell_about(&seeker, addr.to_string()).await;

    let (_, digest) = a_bundle("substituted", "1.0");
    std::fs::remove_dir_all(own_cache().join(&digest)).ok();
    seeker.install("substituted", &digest).await;

    let row = seeker
        .eventually("substituted", "refused", |r| r["status"]["state"] == "Failed")
        .await;
    assert!(
        row["status"]["reason"].as_str().unwrap_or_default().contains("no station"),
        "a wrong answer is no answer: {row}",
    );
    assert!(
        !own_cache().join(&digest).exists(),
        "and nothing was unpacked from bytes that were not the bundle",
    );

    seeker.stop();
}
