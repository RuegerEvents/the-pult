//! What a plugin remembers, from the outside.
//!
//! Driven through `store-probe`, a plugin whose whole job is to call the store
//! interface and say what came back — because everything worth asserting here
//! happens on the host side of the boundary, and a guest is the only thing that
//! can provoke it.
//!
//! Needs the plugins built (`scripts/build-plugins.sh`). When they are not
//! there the tests skip with a message rather than failing, the way
//! `plugins.rs` does: a plain `cargo test` on a machine without the
//! `wasm32-wasip2` target stays green, and CI builds them first.

use std::path::PathBuf;
use std::time::Duration;

use pult_backend::{Config, Running};
use pult_schema::path::PathSegment;
use serde_json::{json, Value};

/// The probe's directory, or `None` when it has not been built.
fn probe_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/store-probe");
    let dir = dir.canonicalize().ok()?;
    dir.join("store_probe.wasm").is_file().then_some(dir)
}

/// One station store for the whole test binary.
///
/// Deliberately shared rather than one per test: the station store *is* one file
/// shared by everything on the machine, and a test that restarts its station has
/// to find what it wrote still there. The tests stay out of each other's way by
/// using different keys.
///
/// Named through `Config::plugin_data` rather than `PULT_PLUGIN_DATA`, because an
/// environment variable is one per process and these tests run at the same time in
/// one — pointing it somewhere would be pointing it there for every other station
/// in the binary too.
fn station_store_file() -> &'static PathBuf {
    static FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        std::env::temp_dir().join(format!("pult-probe-store-{}.db", uuid::Uuid::new_v4()))
    })
}

/// A station running nothing but the probe.
async fn a_station() -> Option<(Running, PathBuf)> {
    let dir = probe_dir()?;
    let _ = station_store_file();

    let showfile = std::env::temp_dir().join(format!("pult-probe-{}.db", uuid::Uuid::new_v4()));
    let running = pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        plugin_data: Some(station_store_file().clone()),
        showfile: showfile.to_string_lossy().into_owned(),
        plugin_dirs: vec![dir],
        ..Config::default()
    })
    .await
    .expect("station starts");
    wait_until_running(&running).await;
    Some((running, showfile))
}

async fn wait_until_running(running: &Running) {
    for _ in 0..300 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let state = running
            .engine
            .get(vec![PathSegment::Key("plugins".into())])
            .await
            .unwrap_or(Value::Null);
        let up = state["plugins"]
            .as_array()
            .is_some_and(|p| p.iter().any(|q| q["status"]["state"] == "Running"));
        if up {
            return;
        }
    }
    panic!("store-probe never came up");
}

/// Ask the probe to do something, as nobody in particular.
async fn probe(running: &Running, method: &str, args: Value) -> Result<Value, String> {
    running.plugins.call("store-probe".into(), method.into(), args).await
}

/// The same, as a person — which is what decides whether a write is theirs.
async fn probe_as(
    running: &Running,
    user: uuid::Uuid,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    running
        .plugins
        .call(
            "store-probe".into(),
            method.into(),
            json!({ "payload": args, "ctx": { "userId": user.to_string() } }),
        )
        .await
}

async fn set(running: &Running, store: &str, key: &str, value: Value) -> Result<Value, String> {
    probe(running, "set", json!({ "store": store, "key": key, "value": value })).await
}

async fn get(running: &Running, store: &str, key: &str) -> Value {
    probe(running, "get", json!({ "store": store, "key": key }))
        .await
        .unwrap_or(Value::Null)
}

macro_rules! station_or_skip {
    () => {
        match a_station().await {
            Some(pair) => pair,
            None => {
                eprintln!("skipping: store-probe not built (scripts/build-plugins.sh)");
                return;
            }
        }
    };
}

#[tokio::test]
async fn what_a_plugin_writes_it_reads_back() {
    let (running, showfile) = station_or_skip!();

    // Nothing yet, and that is an answer rather than an error: a cache's first
    // run is not a failure.
    assert_eq!(get(&running, "carried", "greeting").await, Value::Null);

    set(&running, "carried", "greeting", json!("hello")).await.expect("the write is taken");
    assert_eq!(get(&running, "carried", "greeting").await, json!("hello"));

    // Writing again replaces rather than adding beside: the key is the key.
    set(&running, "carried", "greeting", json!("goodbye")).await.expect("the write is taken");
    assert_eq!(get(&running, "carried", "greeting").await, json!("goodbye"));

    // And the same for the machine-local kind, through the same calls — which
    // is the point of the scope living in the manifest and not in the API.
    set(&running, "local", "reads-back", json!({ "n": 1 })).await.expect("the write is taken");
    assert_eq!(get(&running, "local", "reads-back").await, json!({ "n": 1 }));

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

#[tokio::test]
async fn keys_are_listed_by_prefix_and_forgotten_on_request() {
    let (running, showfile) = station_or_skip!();

    for key in ["macro/one", "macro/two", "other"] {
        set(&running, "carried", key, json!(key)).await.expect("the write is taken");
    }

    let all = probe(&running, "keys", json!({ "store": "carried", "prefix": "" }))
        .await
        .expect("keys answers");
    assert_eq!(all, json!(["macro/one", "macro/two", "other"]), "in order, and all of them");

    let some = probe(&running, "keys", json!({ "store": "carried", "prefix": "macro/" }))
        .await
        .expect("keys answers");
    assert_eq!(some, json!(["macro/one", "macro/two"]), "exactly those with the prefix");

    probe(&running, "delete", json!({ "store": "carried", "key": "other" }))
        .await
        .expect("delete answers");
    assert_eq!(get(&running, "carried", "other").await, Value::Null);

    // Forgetting what was never there is not an error.
    probe(&running, "delete", json!({ "store": "carried", "key": "never-was" }))
        .await
        .expect("deleting nothing is fine");

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

#[tokio::test]
async fn a_store_the_manifest_did_not_declare_cannot_be_touched() {
    let (running, showfile) = station_or_skip!();

    // Declaring the store is the permission, so this is refused before
    // anything is read or written — and it is refused by name, because an
    // author who mistyped a store id should be told which one.
    let err = set(&running, "not-declared", "k", json!(1)).await.unwrap_err();
    assert!(err.contains("not-declared"), "names the store it refused: {err}");
    assert!(err.contains("declares no store"), "{err}");

    let err = probe(&running, "get", json!({ "store": "not-declared", "key": "k" }))
        .await
        .unwrap_err();
    assert!(err.contains("declares no store"), "reads are refused too: {err}");

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

#[tokio::test]
async fn a_store_is_bounded_and_a_refused_write_changes_nothing() {
    let (running, showfile) = station_or_skip!();

    // `small` is declared with room for two keys and 64 bytes.
    set(&running, "small", "a", json!("x")).await.expect("first fits");
    set(&running, "small", "b", json!("y")).await.expect("second fits");

    let err = set(&running, "small", "c", json!("z")).await.unwrap_err();
    assert!(err.contains("its 2 keys"), "names the limit: {err}");
    assert_eq!(get(&running, "small", "c").await, Value::Null, "and wrote nothing");
    assert_eq!(get(&running, "small", "a").await, json!("x"), "and disturbed nothing");

    // The byte ceiling is the other half, and replacing a key spends only the
    // difference — so a big value in an existing key is refused on size.
    let err = set(&running, "small", "a", json!("x".repeat(100))).await.unwrap_err();
    assert!(err.contains("may hold"), "names the limit: {err}");
    assert_eq!(get(&running, "small", "a").await, json!("x"), "and left it as it was");

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

/// The default: a plugin's write is the plugin's, and an operator's undo takes
/// back the operator's own last change rather than the plugin's bookkeeping.
#[tokio::test]
async fn an_ordinary_store_write_is_not_the_operators() {
    let (running, showfile) = station_or_skip!();
    let user = uuid::Uuid::new_v4();

    // Something of the operator's own to go back to.
    let sequence = uuid::Uuid::new_v4();
    running
        .engine
        .set_as(
            user,
            None,
            vec![
                PathSegment::Key("sequences".into()),
                PathSegment::Key("__create".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            json!({ "id": sequence, "name": "theirs", "cue_ids": [] }),
        )
        .await
        .expect("the operator's edit lands");

    // Then the plugin writes, on that operator's call.
    probe_as(&running, user, "set", json!({ "store": "carried", "key": "k", "value": 1 }))
        .await
        .expect("the write is taken");

    // The history is a record of people, and this was not one.
    let history = running.engine.history(50).await;
    assert!(
        !history.iter().any(|op| path_names_plugin_data(&op.path)),
        "no plugin bookkeeping in the history: {:?}",
        history.iter().map(|op| &op.path).collect::<Vec<_>>()
    );

    // And undo takes back what the operator did, not what the plugin did.
    running.engine.undo(user, false).await;
    assert_eq!(get(&running, "carried", "k").await, json!(1), "the plugin's write stands");
    let sequences = running
        .engine
        .get(vec![PathSegment::Key("sequences".into())])
        .await
        .unwrap_or(Value::Null);
    assert_eq!(
        sequences.as_array().map(|s| s.len()),
        Some(0),
        "and it was the operator's own last change that went back: {sequences}"
    );

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

/// The opt-in: a store the manifest declared `undoable` is the operator's, and
/// Ctrl-Z means what they saved.
#[tokio::test]
async fn a_store_that_says_so_undoes_like_any_edit() {
    let (running, showfile) = station_or_skip!();
    let user = uuid::Uuid::new_v4();

    probe_as(
        &running,
        user,
        "set",
        json!({ "store": "deliberate", "key": "opening", "value": "the macro" }),
    )
    .await
    .expect("the write is taken");
    assert_eq!(get(&running, "deliberate", "opening").await, json!("the macro"));

    // It is in the history, because a person did it.
    let history = running.engine.history(50).await;
    assert!(
        history.iter().any(|op| path_names_plugin_data(&op.path)),
        "what the operator saved is in the history"
    );

    // And it comes back out again.
    let moved = running.engine.undo(user, false).await;
    assert!(!moved.is_empty(), "the undo moved something");
    assert_eq!(
        get(&running, "deliberate", "opening").await,
        Value::Null,
        "what the operator asked the plugin to save is taken back"
    );

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

/// The rule falls out of attribution rather than being a second one: a write
/// nobody asked for has no operator to attribute it to, whatever the store says.
#[tokio::test]
async fn an_undoable_store_written_with_nobody_behind_it_is_still_not_undoable() {
    let (running, showfile) = station_or_skip!();

    // No ctx, so no user — the shape of a timer, or of `init`.
    set(&running, "deliberate", "by-nobody", json!(1)).await.expect("the write is taken");

    let history = running.engine.history(50).await;
    assert!(
        !history.iter().any(|op| path_names_plugin_data(&op.path)),
        "a write no person made is not in the record of what people did"
    );

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

fn path_names_plugin_data(path: &[PathSegment]) -> bool {
    matches!(path.first(), Some(PathSegment::Key(k)) if k == "plugin_data")
}

/// Station-scoped data is not the show's: it is not in the showfile, and a
/// second station in the same session never sees it.
#[tokio::test]
async fn station_scoped_data_stays_on_the_machine() {
    let (running, showfile) = station_or_skip!();

    set(&running, "local", "stays-put", json!("mine")).await.expect("the write is taken");
    set(&running, "carried", "shared", json!("ours")).await.expect("the write is taken");

    // The show carries the one and not the other.
    let rows = running
        .engine
        .get(vec![PathSegment::Key("plugin_data".into())])
        .await
        .unwrap_or(Value::Null);
    let stores: Vec<&str> = rows
        .as_array()
        .map(|rows| rows.iter().filter_map(|r| r["store"].as_str()).collect())
        .unwrap_or_default();
    assert!(stores.contains(&"carried"), "the show-scoped write is show data: {rows}");
    assert!(
        !stores.contains(&"local"),
        "the station-scoped write is not, and so cannot replicate or be backed up: {rows}"
    );

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

/// And it does not belong to the show that happened to be open when it was
/// written, which is the whole reason it is a separate file.
#[tokio::test]
async fn station_scoped_data_outlives_the_show_it_was_written_under() {
    let (running, showfile) = station_or_skip!();
    set(&running, "local", "outlives", json!("still here")).await.expect("the write is taken");
    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);

    // A different show, on the same machine, with the same station store.
    let dir = probe_dir().expect("the probe is built");
    let other = std::env::temp_dir().join(format!("pult-probe-other-{}.db", uuid::Uuid::new_v4()));
    let running = pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        plugin_data: Some(station_store_file().clone()),
        showfile: other.to_string_lossy().into_owned(),
        plugin_dirs: vec![dir],
        ..Config::default()
    })
    .await
    .expect("station starts");
    wait_until_running(&running).await;

    assert_eq!(
        get(&running, "local", "outlives").await,
        json!("still here"),
        "the machine remembers, whichever show is open"
    );
    // The show-scoped store is empty here, because that one *was* the show's.
    assert_eq!(get(&running, "carried", "shared").await, Value::Null);

    running.serve.abort();
    let _ = std::fs::remove_file(&other);
}

/// The property that keeps a plugin from filling the oplog.
///
/// One inbound call is one gesture, and a write inside a gesture replaces that
/// gesture's earlier write to the same path. So ten writes to one key collapse
/// — but to *two* rows, not one: the first is a create, and creates are never
/// folded, because every create in a collection shares the `__create` path and
/// folding two would lose a row.
#[tokio::test]
async fn repeated_writes_in_one_call_collapse_in_the_log() {
    let (running, showfile) = station_or_skip!();
    let user = uuid::Uuid::new_v4();

    probe_as(
        &running,
        user,
        "set-repeatedly",
        json!({ "store": "deliberate", "key": "hammered", "times": 10 }),
    )
    .await
    .expect("the writes are taken");

    let rows = running
        .engine
        .history(200)
        .await
        .into_iter()
        .filter(|op| path_names_plugin_data(&op.path))
        .count();
    assert_eq!(rows, 2, "the create, and one folded value write — not ten");

    // A second call is a second gesture, so it does not fold into the first:
    // two separate things a person did stay two rows.
    probe_as(
        &running,
        user,
        "set-repeatedly",
        json!({ "store": "deliberate", "key": "hammered", "times": 10 }),
    )
    .await
    .expect("the writes are taken");
    let rows = running
        .engine
        .history(200)
        .await
        .into_iter()
        .filter(|op| path_names_plugin_data(&op.path))
        .count();
    assert_eq!(rows, 3, "one more row for the second gesture");

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

/// Removing a plugin does not delete what it remembered.
///
/// Deliberate, so that a plugin removed by mistake and put back finds its
/// macros where it left them. The data is the operator's; the plugin is only
/// what reads it.
#[tokio::test]
async fn data_outlives_the_plugin_that_wrote_it() {
    let dir = match probe_dir() {
        Some(dir) => dir,
        None => {
            eprintln!("skipping: store-probe not built (scripts/build-plugins.sh)");
            return;
        }
    };
    let _ = station_store_file();
    let showfile = std::env::temp_dir().join(format!("pult-probe-{}.db", uuid::Uuid::new_v4()));

    // The plugin is here, and writes something.
    let running = start_with(&showfile, vec![dir.clone()]).await;
    wait_until_running(&running).await;
    set(&running, "carried", "outlives", json!("the macro")).await.expect("the write is taken");
    running.serve.abort();

    // Now it is gone — no plugin directory at all — and the show still carries
    // what it wrote, under the id of the plugin that is no longer installed.
    let running = start_with(&showfile, vec![]).await;
    let rows = running
        .engine
        .get(vec![PathSegment::Key("plugin_data".into())])
        .await
        .unwrap_or(Value::Null);
    let orphaned = rows
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|r| r["plugin_id"] == "store-probe" && r["key"] == "outlives")
                .count()
        })
        .unwrap_or(0);
    assert_eq!(orphaned, 1, "the data is still in the show, and says whose it was: {rows}");
    running.serve.abort();

    // And when it comes back, it reads what it left.
    let running = start_with(&showfile, vec![dir.clone()]).await;
    wait_until_running(&running).await;
    assert_eq!(
        get(&running, "carried", "outlives").await,
        json!("the macro"),
        "a plugin removed by mistake finds its data where it left it"
    );
    running.serve.abort();

    // The same holds for an upgrade, because the version plays no part in where
    // the data lives: a row is keyed by plugin id, store and key, so a different
    // build under the same id is the same plugin as far as its memory goes.
    let upgraded = std::env::temp_dir().join(format!("pult-probe-v2-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(upgraded.join("store-probe")).expect("a directory for the new build");
    for file in ["store_probe.wasm", "pult-plugin.toml"] {
        std::fs::copy(dir.join(file), upgraded.join("store-probe").join(file))
            .expect("the build is copied");
    }
    let manifest = upgraded.join("store-probe/pult-plugin.toml");
    let text = std::fs::read_to_string(&manifest).expect("the manifest reads");
    std::fs::write(&manifest, text.replace(r#"version = "0.1.0""#, r#"version = "9.9.9""#))
        .expect("the manifest is rewritten");

    let running = start_with(&showfile, vec![upgraded.clone()]).await;
    wait_until_running(&running).await;
    assert_eq!(
        get(&running, "carried", "outlives").await,
        json!("the macro"),
        "and a new version of the same plugin reads what the old one stored"
    );

    running.serve.abort();
    let _ = std::fs::remove_dir_all(&upgraded);
    let _ = std::fs::remove_file(&showfile);
}

async fn start_with(showfile: &PathBuf, plugin_dirs: Vec<PathBuf>) -> Running {
    pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        plugin_data: Some(station_store_file().clone()),
        showfile: showfile.to_string_lossy().into_owned(),
        plugin_dirs,
        ..Config::default()
    })
    .await
    .expect("station starts")
}
