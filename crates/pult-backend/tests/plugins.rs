//! The plugin runtime, end to end: a real station loading the real
//! `command-line` and `natural-language-control` components.
//!
//! Needs the plugins built (`scripts/build-plugins.sh`), which needs the
//! `wasm32-wasip2` target. When the components are not there, the test skips
//! with a message rather than failing — a plain `cargo test` on a machine
//! without the target stays green, and CI builds the plugins first so this
//! genuinely runs there.

use std::path::PathBuf;
use std::time::Duration;

use pult_backend::Config;
use serde_json::{json, Value};

/// Where the built plugins lie, relative to this crate: the plugins workspace,
/// with each component beside its manifest. `PULT_TEST_PLUGINS` overrides.
fn plugins_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PULT_TEST_PLUGINS") {
        return Some(PathBuf::from(dir));
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    dir.canonicalize().ok()
}

#[tokio::test]
async fn a_station_runs_the_reference_plugins() {
    let Some(dir) = plugins_dir() else {
        eprintln!("skipping: no plugins directory");
        return;
    };
    if !dir.join("command-line/command_line.wasm").is_file() {
        eprintln!("skipping: plugins not built (scripts/build-plugins.sh)");
        return;
    }

    let showfile = std::env::temp_dir().join(format!("pult-plugin-test-{}.db", uuid::Uuid::new_v4()));
    let running = pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        showfile: showfile.to_string_lossy().into_owned(),
        plugin_dirs: vec![dir],
        ..Config::default()
    })
    .await
    .expect("station starts");

    // The manager loads plugins after start returns; compiling two components
    // takes real time the first run.
    let mut state = Value::Null;
    for _ in 0..300 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        state = running
            .engine
            .get(vec![pult_schema::path::PathSegment::Key("plugins".into())])
            .await
            .unwrap_or(Value::Null);
        let all_settled = state
            .get("plugins")
            .and_then(Value::as_array)
            .is_some_and(|plugins| {
                plugins.len() == 2
                    && plugins.iter().all(|p| p["status"]["state"] != "Loading")
            });
        if all_settled {
            break;
        }
    }
    let plugins = state["plugins"].as_array().cloned().unwrap_or_default();
    assert_eq!(plugins.len(), 2, "both reference plugins are seen: {state}");
    for plugin in &plugins {
        assert_eq!(
            plugin["status"]["state"], "Running",
            "{} should run: {}",
            plugin["id"], plugin["status"]
        );
    }
    // The dependency loaded first and the dependent found it — otherwise
    // natural-language-control's init, which calls the command line for its
    // grammar, would have failed above.

    // The LOCAL state announces the declared surfaces.
    let cli = plugins.iter().find(|p| p["id"] == "command-line").unwrap();
    assert_eq!(cli["surfaces"][0]["kind"], "console");

    // Help answers, and its vocabulary comes from the real schema registries.
    let help = running
        .plugins
        .call("command-line".into(), "surface.help".into(), json!({}))
        .await
        .expect("help answers");
    let text = help["text"].as_str().unwrap_or("");
    assert!(text.contains("fixture 1 thru 5"), "teaches selection: {text}");
    assert!(text.contains("sequence"), "knows the schema's collections: {text}");

    // A real command writes the real show.
    let exec = |line: &str| {
        let payload = json!({ "payload": { "line": line }, "ctx": { "selection": [] } });
        running.plugins.call("command-line".into(), "surface.exec".into(), payload)
    };
    let result = exec(r#"create sequence "From the command line""#)
        .await
        .expect("create answers");
    assert!(result["error"].is_null(), "create succeeds: {result}");
    let sequences = running
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("sequences".into())])
        .await
        .expect("sequences readable");
    assert_eq!(
        sequences[0]["name"], "From the command line",
        "the sequence exists: {sequences}"
    );

    // A wrong command comes back as an error with a span, not as silence.
    let result = exec("sequence 1 fly").await.expect("exec answers");
    let error = &result["error"];
    assert!(
        error["message"].as_str().unwrap_or("").contains("no command"),
        "names the problem: {result}"
    );
    assert!(error["span"]["start"].is_u64(), "and points at it: {result}");
    assert!(
        error["expected"].as_array().is_some_and(|e| !e.is_empty()),
        "and offers the alternatives: {result}"
    );

    // Completion is cursor-aware and reads real entity names from the show.
    let complete = running
        .plugins
        .call(
            "command-line".into(),
            "surface.complete".into(),
            json!({ "payload": { "line": "delete sequence ", "cursor": 16 } }),
        )
        .await
        .expect("complete answers");
    let items = complete["items"].as_array().cloned().unwrap_or_default();
    assert!(
        items.iter().any(|i| i["detail"] == "From the command line"),
        "offers the sequence just created: {complete}"
    );

    // An unknown plugin is an answer, not a hang.
    let missing = running
        .plugins
        .call("no-such-plugin".into(), "surface.exec".into(), json!({}))
        .await;
    assert!(missing.is_err());

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}
