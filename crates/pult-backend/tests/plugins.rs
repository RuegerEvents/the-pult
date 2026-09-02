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

/// How many plugin directories `plugins/` holds: the two reference plugins and
/// `store-probe`, which exists to be tested against rather than to be shipped.
const EXPECTED_PLUGINS: usize = 3;

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
    // Its own store as well: without one this station opens the *developer's*
    // `plugin-data.db` and writes whatever the reference plugins remember into it.
    let store = std::env::temp_dir().join(format!("pult-plugin-store-{}.db", uuid::Uuid::new_v4()));
    let running = pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        showfile: showfile.to_string_lossy().into_owned(),
        plugin_dirs: vec![dir],
        plugin_data: Some(store.clone()),
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
                plugins.len() == EXPECTED_PLUGINS
                    && plugins.iter().all(|p| p["status"]["state"] != "Loading")
            });
        if all_settled {
            break;
        }
    }
    let plugins = state["plugins"].as_array().cloned().unwrap_or_default();
    assert_eq!(plugins.len(), EXPECTED_PLUGINS, "every plugin in the directory is seen: {state}");
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

    // ── Saved groups ──────────────────────────────────────────────────────────
    //
    // `group 3` is a selection, not a command on a row, and what it hands back is
    // the group's *question* — so the browser's selection goes on following the
    // rig exactly as recalling the group in the panel does.
    let fixture_type = uuid::Uuid::new_v4();
    let mut fixture_ids = Vec::new();
    for name in ["Movement 1", "Movement 2"] {
        let id = uuid::Uuid::new_v4();
        fixture_ids.push(id);
        running
            .engine
            .set(
                vec![
                    pult_schema::path::PathSegment::Key("fixtures".into()),
                    pult_schema::path::PathSegment::Key("__create".into()),
                ],
                pult_schema::lifecycle::Lifecycle::Persisted,
                json!({
                    "id": id,
                    "name": name,
                    "fixture_type_id": fixture_type,
                    "address": { "Dmx": { "mode": "Default", "breaks": [{ "universe": 1, "address": 1 }] } },
                    "position": null,
                    "sensed_values": {}
                }),
            )
            .await
            .expect("a fixture to group");
    }
    let group_id = uuid::Uuid::new_v4();
    running
        .engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("groups".into()),
                pult_schema::path::PathSegment::Key("__create".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            json!({
                "id": group_id,
                "name": "Movers",
                "query": {
                    "clauses": [{ "combine": "Add", "term": { "kind": "OfType", "typeId": fixture_type } }],
                    "order": { "kind": "ByName" }
                }
            }),
        )
        .await
        .expect("a group to select");

    let result = exec("group 1").await.expect("exec answers");
    assert!(result["error"].is_null(), "`group 1` parses and runs: {result}");
    assert!(
        result["effects"]["selection"]["query"].is_object(),
        "a group hands back the question, not the answer: {result}"
    );
    assert!(
        result["effects"]["selection"]["fixtureIds"].is_null(),
        "and not both, which would leave the surface to choose: {result}"
    );

    // By name, and with a level in the same line — which does need the ids, so
    // this is also the station's `group.resolve` being called through the host.
    let result = exec(r#"group "Movers" at 50"#).await.expect("exec answers");
    assert!(result["error"].is_null(), "`group \"Movers\" at 50` runs: {result}");
    let held = running
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("programmer_values".into())])
        .await
        .expect("programmer readable");
    let held = held.as_array().cloned().unwrap_or_default();
    assert_eq!(held.len(), 2, "both of the group's fixtures are held: {held:?}");

    // A group nobody has says so, and changes nothing.
    let result = exec(r#"group "Nope""#).await.expect("exec answers");
    assert!(
        result["error"]["message"].as_str().unwrap_or("").contains("Nope"),
        "names the group that is not there: {result}"
    );
    assert!(result["effects"].is_null(), "and touches neither selection nor programmer: {result}");
    let still = running
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("programmer_values".into())])
        .await
        .expect("programmer readable");
    assert_eq!(still.as_array().map(Vec::len), Some(2), "the programmer is untouched: {still}");

    // ── Relative levels ───────────────────────────────────────────────────────
    //
    // `at +10` sends the *delta* to the station, which resolves it against what it
    // is showing. This plugin never reads a level and computes a destination, so two
    // operators nudging one light both get their nudge.
    let spot = uuid::Uuid::new_v4();
    running
        .engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("fixtures".into()),
                pult_schema::path::PathSegment::Key("__create".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            json!({
                "id": spot, "name": "Nudged", "fixture_type_id": uuid::Uuid::new_v4(),
                "address": { "Dmx": { "mode": "Default", "breaks": [{ "universe": 1, "address": 100 }] } },
                "position": null
            }),
        )
        .await
        .expect("a fixture to nudge");

    let held_level = || {
        let engine = running.engine.clone();
        async move {
            let rows = engine
                .get(vec![pult_schema::path::PathSegment::Key("programmer_values".into())])
                .await
                .expect("programmer readable");
            rows.as_array()
                .and_then(|rows| {
                    rows.iter()
                        .find(|r| r["fixture_id"] == json!(spot))
                        .and_then(|r| r["value"]["value"].as_f64())
                })
                .expect("the fixture is held")
        }
    };

    let position = running
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("fixtures".into())])
        .await
        .expect("fixtures readable")
        .as_array()
        .map(|f| f.iter().position(|x| x["id"] == json!(spot)).expect("it is in the rig") + 1)
        .expect("a rig");

    let result = exec(&format!("fixture {position} at 50")).await.expect("exec answers");
    assert!(result["error"].is_null(), "an absolute level still works: {result}");
    assert!((held_level().await - 0.5).abs() < 1e-5);

    // The selection lives in the browser and comes to the plugin per call, so a
    // second exec that carries one is how `at +10` is asked for from here.
    let exec_selecting = |line: &str, selection: Vec<uuid::Uuid>| {
        let payload =
            json!({ "payload": { "line": line }, "ctx": { "selection": selection } });
        running.plugins.call("command-line".into(), "surface.exec".into(), payload)
    };

    let result = exec_selecting("at +10", vec![spot]).await.expect("exec answers");
    assert!(result["error"].is_null(), "`at +10` runs: {result}");
    assert!(
        result["lines"][0]["text"].as_str().unwrap_or("").contains("brighter"),
        "and says what it did: {result}"
    );
    assert!((held_level().await - 0.6).abs() < 1e-5, "50% and ten points more");

    let _ = exec_selecting("at -20", vec![spot]).await.expect("exec answers");
    assert!((held_level().await - 0.4).abs() < 1e-5, "and down again");

    // And the ordinary case: nothing held yet, so the nudge takes the key and starts
    // from what playback has the fixture at.
    let untouched = uuid::Uuid::new_v4();
    running
        .engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("fixtures".into()),
                pult_schema::path::PathSegment::Key("__create".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            json!({
                "id": untouched, "name": "Untouched", "fixture_type_id": uuid::Uuid::new_v4(),
                "address": { "Dmx": { "mode": "Default", "breaks": [{ "universe": 1, "address": 110 }] } },
                "position": null
            }),
        )
        .await
        .expect("a fixture nobody has touched");
    // Playback is showing 0.3: a landed fade, which is how a parameter holds a value
    // now that nothing stores the number.
    running
        .engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("fixtures".into()),
                pult_schema::path::PathSegment::Id(untouched),
                pult_schema::path::PathSegment::Key("live_fades".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Local,
            json!({
                "Intensity": {
                    "from": { "type": "Float", "value": 0.3 },
                    "to": { "type": "Float", "value": 0.3 },
                    "t0": 0,
                    "duration_ms": 0,
                    "easing": "Step",
                    "cue_id": uuid::Uuid::nil(),
                }
            }),
        )
        .await
        .expect("playback is showing something");

    let result = exec_selecting("at +10", vec![untouched]).await.expect("exec answers");
    assert!(result["error"].is_null(), "a nudge takes an unheld key: {result}");
    let rows = running
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("programmer_values".into())])
        .await
        .expect("programmer readable");
    let taken = rows
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["fixture_id"] == json!(untouched)))
        .expect("the programmer took the key");
    let level = taken["value"]["value"].as_f64().expect("a float");
    assert!((level - 0.4).abs() < 1e-5, "from what playback was showing, not from zero: {taken}");

    // An unknown plugin is an answer, not a hang.
    let missing = running
        .plugins
        .call("no-such-plugin".into(), "surface.exec".into(), json!({}))
        .await;
    assert!(missing.is_err());

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
    let _ = std::fs::remove_file(&store);
}

/// A plugin built against an *earlier* minor of the contract still runs here.
///
/// This is the property the whole versioning scheme rests on, and it is not
/// self-evident: a component's imports carry the package version they were built
/// against (`pult:plugin/data@1.0.0`), so the host only satisfies them because
/// wasmtime resolves component imports semver-compatibly. That is also why the
/// package had to leave `0.x` — under semver a `0.x` minor bump is breaking, so
/// wasmtime treats `0.1` and `0.2` as unrelated and every import fails to link.
///
/// Ignored by default because it needs a component built against a version this
/// tree no longer has. `scripts/check-api-compat.sh` produces one and runs this;
/// there is nothing to check in, which is the point — the fixture is a build
/// output, not a file.
#[tokio::test]
#[ignore = "needs a component built against an older API; run scripts/check-api-compat.sh"]
async fn a_plugin_built_against_an_earlier_api_still_runs() {
    let Ok(dir) = std::env::var("PULT_OLD_API_PLUGINS") else {
        panic!("PULT_OLD_API_PLUGINS must name a plugin directory built against an older API");
    };

    let showfile =
        std::env::temp_dir().join(format!("pult-oldapi-{}.db", uuid::Uuid::new_v4()));
    let running = pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        showfile: showfile.to_string_lossy().into_owned(),
        plugin_dirs: vec![PathBuf::from(dir)],
        ..Config::default()
    })
    .await
    .expect("station starts");

    let mut state = Value::Null;
    for _ in 0..300 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        state = running
            .engine
            .get(vec![pult_schema::path::PathSegment::Key("plugins".into())])
            .await
            .unwrap_or(Value::Null);
        let settled = state
            .get("plugins")
            .and_then(Value::as_array)
            .is_some_and(|p| !p.is_empty() && p.iter().all(|q| q["status"]["state"] != "Loading"));
        if settled {
            break;
        }
    }

    let plugins = state["plugins"].as_array().cloned().unwrap_or_default();
    assert!(!plugins.is_empty(), "the older plugin is seen: {state}");
    for plugin in &plugins {
        assert_eq!(
            plugin["status"]["state"], "Running",
            "{} was built against an earlier API and must still run: {}",
            plugin["id"], plugin["status"]
        );
    }

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
}

/// The worked example for stores: this console remembers which model it talks
/// to, and remembers it across a restart.
///
/// Station-scoped on purpose — the model installed on this machine is not a
/// fact about the show — so what is asserted here is exactly what an operator
/// would notice: they picked a provider once, and the next start still speaks
/// to it without being told again.
#[tokio::test]
async fn the_language_plugin_remembers_which_model_this_console_uses() {
    let Some(dir) = plugins_dir() else {
        eprintln!("skipping: no plugins directory");
        return;
    };
    if !dir.join("natural-language-control/natural_language_control.wasm").is_file() {
        eprintln!("skipping: plugins not built (scripts/build-plugins.sh)");
        return;
    }
    // A station store of this test's own, so it is not reading a real console's.
    // Named in the config rather than exported: `PULT_PLUGIN_DATA` is one variable
    // per process, and the other test in this binary starts its own station at the
    // same time — setting it here would decide where *that* station's plugins
    // remember things too.
    let store =
        std::env::temp_dir().join(format!("pult-nl-prefs-{}.db", uuid::Uuid::new_v4()));

    let showfile = std::env::temp_dir().join(format!("pult-nl-{}.db", uuid::Uuid::new_v4()));
    let start = || async {
        let running = pult_backend::start(Config {
            port: 0,
            sync_port: 0,
            showfile: showfile.to_string_lossy().into_owned(),
            plugin_dirs: vec![dir.clone()],
            plugin_data: Some(store.clone()),
            ..Config::default()
        })
        .await
        .expect("station starts");
        for _ in 0..300 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let state = running
                .engine
                .get(vec![pult_schema::path::PathSegment::Key("plugins".into())])
                .await
                .unwrap_or(Value::Null);
            let up = state["plugins"].as_array().is_some_and(|plugins| {
                plugins.iter().any(|p| {
                    p["id"] == "natural-language-control" && p["status"]["state"] == "Running"
                })
            });
            if up {
                break;
            }
        }
        running
    };
    let ask = |running: &pult_backend::Running, method: &str, args: Value| {
        let running = running.plugins.clone();
        let method = method.to_string();
        async move { running.call("natural-language-control".into(), method, args).await }
    };

    let running = start().await;

    // With an empty store it is what the manifest configured.
    let before = ask(&running, "provider", json!({})).await.expect("it answers");
    assert_eq!(before["label"], "ollama · qwen3:4b", "the manifest's own default: {before}");

    // The operator picks something else.
    let chosen = ask(&running, "use", json!({ "provider": "openai" }))
        .await
        .expect("the choice is taken");
    assert_eq!(chosen["label"], "openai · gpt-4.1-mini", "{chosen}");

    // A name that is not a provider is refused, and changes nothing — one
    // mistyped word must not leave a console unable to start.
    let err = ask(&running, "use", json!({ "provider": "not-a-provider" })).await.unwrap_err();
    assert!(err.contains("unknown provider"), "{err}");

    running.serve.abort();

    // The console comes back speaking to what it was told, without being told.
    let running = start().await;
    let after = ask(&running, "provider", json!({})).await.expect("it answers");
    assert_eq!(
        after["label"], "openai · gpt-4.1-mini",
        "the operator said once and this machine remembered: {after}"
    );

    running.serve.abort();
    let _ = std::fs::remove_file(&showfile);
    let _ = std::fs::remove_file(&store);
}
