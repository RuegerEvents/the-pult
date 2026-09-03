//! The station RPCs: the calls that are not entity commands.
//!
//! An entity command deserializes a row, mutates it, and hands the engine
//! something to apply — it *changes* the show, and it writes an operation
//! saying so. What is here is the other kind: calls that go to the manager
//! owning some LOCAL state, and calls that answer a question about the show
//! without changing it. Both would be lies as commands, the first because
//! there is no entity, the second because a read is not an edit and has no
//! business in anyone's undo stack.
//!
//! One table and one dispatcher, so the WebSocket handler and the plugin
//! runtime answer the same calls the same way, and introspection can list them
//! without a second place going stale. A test below holds the table and the
//! match to each other.

use pult_schema::{
    path::PathSegment,
    types::{
        evaluate,
        fixture::{
            driving, output_parameters, parameter_key, FixtureType, HeldByProgrammer,
            ParameterKind,
        },
        scene::SceneObject,
        Fixture, Group,
    },
};
use serde_json::Value;

use crate::{
    engine::EngineHandle,
    // `short_id` is the client registry's, so the eight characters on a browser's log
    // line and the eight on its row in the System panel are the same eight rather
    // than two implementations that agree today.
    infra::{clients::short_id, devices::DeviceHandle, session::SessionHandle},
};

/// What one RPC looks like from outside: enough for a command line to offer
/// it, complete it, and explain it.
pub struct LocalRpcMeta {
    pub method: &'static str,
    /// JSON array of `{ "name", "type", "optional" }`, mirroring the shape
    /// `CommandRegistration::args_schema` uses. `[]` for no arguments.
    pub args_schema: &'static str,
    pub doc: &'static str,
}

pub const LOCAL_RPCS: &[LocalRpcMeta] = &[
    LocalRpcMeta {
        method: "device.adopt",
        args_schema: r#"[{"name":"serial","type":"string","optional":false}]"#,
        doc: "Adopt a discovered OpenHaunt device; answers with the new fixture's id.",
    },
    LocalRpcMeta {
        method: "device.identify",
        args_schema: r#"[{"name":"serial","type":"string","optional":false}]"#,
        doc: "Ask a discovered device to identify itself, e.g. by blinking.",
    },
    LocalRpcMeta {
        method: "device.forget",
        args_schema: r#"[{"name":"serial","type":"string","optional":false}]"#,
        doc: "Forget an adopted device and release its fixture.",
    },
    LocalRpcMeta {
        method: "session.join",
        args_schema: r#"[{"name":"sessionId","type":"string","optional":false}]"#,
        doc: "Join a discovered session as a follower.",
    },
    LocalRpcMeta {
        method: "session.leave",
        args_schema: "[]",
        doc: "Leave the session this station is following.",
    },
    LocalRpcMeta {
        method: "session.create",
        args_schema: r#"[{"name":"showName","type":"string","optional":false},{"name":"showId","type":"string","optional":false}]"#,
        doc: "Start a session around this station's show; answers with the session id.",
    },
    // Named for what it answers about rather than for `groups`, deliberately: an
    // RPC's prefix is a word the command line's grammar can no longer use for a
    // collection, and `group 3` has to go on meaning the fixtures in group 3.
    LocalRpcMeta {
        method: "selection.resolve",
        args_schema: r#"[{"name":"groupId","type":"string","optional":false}]"#,
        doc: "The fixtures a saved group picks out of the rig right now, in its order.",
    },
    // A read, so deliberately not a command: asking what a light is doing must not
    // write history. It exists because nothing stores the answer any more — the
    // console keeps what is *driving* each parameter and evaluates on demand — and a
    // caller that cannot hold the whole stack still has to be able to ask.
    LocalRpcMeta {
        method: "parameter.value",
        args_schema: r#"[{"name":"fixtureId","type":"string","optional":false},{"name":"parameterKind","type":"object","optional":true}]"#,
        doc: "What a fixture's parameters are putting out right now; one when named, all of them when not.",
    },
    // The console's own log. Reads and diagnostics, so RPCs rather than commands
    // for the reason `parameter.value` is one: none of this is anybody's to undo,
    // and a log that wrote history every time somebody looked at it would be a
    // strange thing indeed.
    LocalRpcMeta {
        method: "log.tail",
        args_schema: r#"[{"name":"limit","type":"number","optional":true},{"name":"level","type":"string","optional":true}]"#,
        doc: "The recent log, oldest first, with this station's levels and where the file is.",
    },
    LocalRpcMeta {
        method: "log.setLevel",
        args_schema: r#"[{"name":"level","type":"string","optional":true},{"name":"peerLevel","type":"string","optional":true}]"#,
        doc: "Set what this station keeps in its log, and what it publishes to peers.",
    },
    LocalRpcMeta {
        method: "log.watch",
        args_schema: r#"[{"name":"nodeId","type":"string","optional":false},{"name":"level","type":"string","optional":false}]"#,
        doc: "Ask a peer to publish its log at this level while this client is watching.",
    },
    LocalRpcMeta {
        method: "log.unwatch",
        args_schema: r#"[{"name":"nodeId","type":"string","optional":false}]"#,
        doc: "Stop watching a peer's log; the peer drops back when nobody else is.",
    },
    LocalRpcMeta {
        method: "log.report",
        args_schema: r#"[{"name":"level","type":"string","optional":false},{"name":"message","type":"string","optional":false},{"name":"count","type":"number","optional":true}]"#,
        doc: "Report something that went wrong in a browser into the station's log.",
    },
    // The other half of what a browser can say about itself: numbers rather than
    // sentences. A page is a console evaluating a rig at frame rate against a clock
    // it had to estimate, and nothing else on this station can measure any of that.
    //
    // One object rather than a dozen named arguments, because nobody types this — it
    // is a page describing itself every couple of seconds, not a verb for the command
    // line — and a shape that grows a field should not grow a signature.
    // What is actually on the wire. Asked for rather than published, for the reason
    // `log.watch` is: a universe image is 512 bytes forty times a second, and a
    // station broadcasting that to browsers nobody has looked at — or across the
    // link carrying the show — would be paying for a picture nobody is reading.
    // `nodeId` may be a peer's: the ask crosses the sync link and that station's
    // connector answers it, since only the station holding a socket can say what
    // went through it.
    LocalRpcMeta {
        method: "output.watch",
        args_schema: r#"[{"name":"outputId","type":"string","optional":false},{"name":"nodeId","type":"string","optional":true},{"name":"focus","type":"string","optional":true}]"#,
        doc: "Watch what an output is putting on the wire while this client is looking.",
    },
    LocalRpcMeta {
        method: "output.unwatch",
        args_schema: r#"[{"name":"outputId","type":"string","optional":false},{"name":"nodeId","type":"string","optional":true}]"#,
        doc: "Stop watching an output; the connector stops drawing when nobody is.",
    },
    LocalRpcMeta {
        method: "client.report",
        args_schema: r#"[{"name":"stats","type":"object","optional":false}]"#,
        doc: "What a browser is costing itself: frame rate, evaluator time, memory, clock offset. Answers the key it landed under.",
    },
];

/// What dispatching needs to reach. Cheap to clone: a few channel handles.
#[derive(Clone)]
pub struct LocalRpcDeps {
    pub session: SessionHandle,
    pub devices: DeviceHandle,
    /// The console's own log, where this process installed one.
    pub log: Option<crate::logging::LogHandle>,
    /// Who is watching which peer's log. Kept beside the log rather than in it,
    /// because it is about this station's *clients* and not about its lines.
    pub log_watchers: crate::logging::Watchers,
    /// The link to the peers, for the one thing here that reaches one: telling a
    /// peer what to publish.
    pub sync: Option<crate::infra::sync::SyncHandle>,
    /// The WebSocket session that asked, where one did.
    ///
    /// `None` for a plugin, which has no browser behind it — so `log.watch` and
    /// `log.report`, the two calls whose whole meaning is "while *this* client is
    /// here", say so rather than pretending. Every other call ignores it.
    pub caller: Option<uuid::Uuid>,
    /// For the calls that answer a question about the show. Reads only — anything
    /// here that wanted to write would be an entity command instead.
    pub engine: EngineHandle,
    /// The sockets this station is serving, for the one thing a page cannot measure
    /// about itself: how much has been sent to it. `None` where there is no HTTP
    /// server behind these deps, which is what a test constructs.
    pub ws_registry: Option<crate::api::ws::SubscriptionRegistry>,
    /// Which station this is, so an ask about an output can be told apart from an
    /// ask about a peer's — the first goes to a connector here, the second down a
    /// link.
    pub node_id: pult_schema::events::operation::NodeId,
    /// Who is watching what an output is putting on the wire.
    pub viewers: crate::infra::connectors::Viewers,
    /// What the browsers on this station are saying about themselves.
    ///
    /// `None` where there is no HTTP server behind these deps at all, which is what
    /// a test constructs. A plugin's deps carry one and still cannot report: the call
    /// needs a `caller`, and a plugin has no browser to be.
    pub clients: Option<crate::infra::clients::ClientRegistry>,
}

fn no_log() -> String {
    "this station has no log; it was started without one".to_string()
}


/// Read a level argument, which may be absent but must not be nonsense.
fn level_arg(args: &Value, name: &str) -> Result<Option<pult_schema::ws::LogLevel>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let text = v.as_str().ok_or_else(|| format!("{name} should be a level, as a string"))?;
            pult_schema::ws::LogLevel::parse(text)
                .map(Some)
                .ok_or_else(|| format!("no such level: {text}"))
        }
    }
}

/// Tell a peer what to publish now that who is watching it has changed.
async fn raise_peer(deps: &LocalRpcDeps, node_id: uuid::Uuid, level: Option<pult_schema::ws::LogLevel>) {
    if let Some(sync) = &deps.sync {
        sync.raise_peer_log(pult_schema::events::operation::NodeId(node_id), level).await;
    }
}

/// Answer one station RPC. `Err` is a message for whoever asked, so it is a
/// plain string the way the WebSocket's `CallResult.error` is.
pub async fn dispatch(method: &str, args: Value, deps: &LocalRpcDeps) -> Result<Value, String> {
    let serial = || {
        args["serial"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "missing serial".to_string())
    };

    match method {
        "device.adopt" => {
            let id = deps.devices.adopt(serial()?).await?;
            serde_json::to_value(id).map_err(|e| e.to_string())
        }
        "device.identify" => {
            deps.devices.identify(serial()?).await?;
            Ok(Value::Null)
        }
        "device.forget" => {
            deps.devices.forget(serial()?).await?;
            Ok(Value::Null)
        }
        "parameter.value" => {
            let fixture_id: uuid::Uuid = serde_json::from_value(args["fixtureId"].clone())
                .map_err(|e| format!("invalid fixtureId: {e}"))?;
            let kind = match args.get("parameterKind") {
                Some(k) if !k.is_null() => Some(
                    serde_json::from_value::<ParameterKind>(k.clone())
                        .map_err(|e| format!("invalid parameterKind: {e}"))?,
                ),
                _ => None,
            };
            what_is_it_doing(&deps.engine, fixture_id, kind).await
        }
        "log.tail" => {
            let log = deps.log.as_ref().ok_or_else(no_log)?;
            let limit = args["limit"].as_u64().unwrap_or(1_000).min(5_000) as usize;
            let level = level_arg(&args, "level")?;
            serde_json::to_value(serde_json::json!({
                "lines": log.tail(limit, level),
                // The panel's header is built from these: which station this is,
                // what it is keeping, what it is telling its peers, and where the
                // rest of the log went when it scrolled out of the ring.
                "nodeId": log.node_id(),
                "captureLevel": log.capture_level(),
                "publishLevel": log.publish_level(),
                "file": log.file_path().map(|p| p.display().to_string()),
                "raised": deps.log_watchers.raised(),
            }))
            .map_err(|e| e.to_string())
        }
        "log.setLevel" => {
            let log = deps.log.as_ref().ok_or_else(no_log)?;
            let mut prefs = crate::infra::preferences::load();
            if let Some(level) = level_arg(&args, "level")? {
                log.set_capture_level(level);
                prefs.log_level = level.as_str().to_string();
            }
            if let Some(level) = level_arg(&args, "peerLevel")? {
                prefs.peer_log_level = level.as_str().to_string();
            }
            // Re-read through `sane`, which is what applies the rule that a station
            // never promises its peers more than it keeps for itself.
            let prefs = prefs.sane();
            log.set_capture_level(prefs.capture_level());
            log.set_publish_level(prefs.peer_level());
            // A level that cannot be written down still takes effect for this run;
            // the panel said what it said, and refusing the change because the disk
            // is read-only would be the wrong half to give up on.
            if let Err(e) = crate::infra::preferences::save(&prefs) {
                tracing::warn!("could not write the log level to preferences: {e}");
            }
            Ok(serde_json::json!({
                "captureLevel": log.capture_level(),
                "publishLevel": log.publish_level(),
            }))
        }
        "log.watch" => {
            let caller = deps.caller.ok_or_else(|| {
                "log.watch is about one client watching, so it needs one".to_string()
            })?;
            let node_id: uuid::Uuid = serde_json::from_value(args["nodeId"].clone())
                .map_err(|e| format!("invalid nodeId: {e}"))?;
            let level = level_arg(&args, "level")?.unwrap_or(pult_schema::ws::LogLevel::Debug);
            if let Some(level) = deps.log_watchers.watch(node_id, caller, level) {
                raise_peer(deps, node_id, level).await;
            }
            Ok(Value::Null)
        }
        "log.unwatch" => {
            let caller = deps.caller.ok_or_else(|| "nobody is watching".to_string())?;
            let node_id: uuid::Uuid = serde_json::from_value(args["nodeId"].clone())
                .map_err(|e| format!("invalid nodeId: {e}"))?;
            if let Some(level) = deps.log_watchers.unwatch(node_id, caller) {
                raise_peer(deps, node_id, level).await;
            }
            Ok(Value::Null)
        }
        "log.report" => {
            let log = deps.log.as_ref().ok_or_else(no_log)?;
            let level = level_arg(&args, "level")?.unwrap_or(pult_schema::ws::LogLevel::Error);
            let message = args["message"].as_str().unwrap_or_default();
            if message.is_empty() {
                return Err("nothing to report".to_string());
            }
            // The browser has already deduped and rate-limited; `count` is how many
            // times the thing it is reporting happened. Said here rather than
            // repeated, so a panel erroring every frame is one line and a number.
            let count = args["count"].as_u64().unwrap_or(1);
            let message = if count > 1 {
                format!("{message} (×{count})")
            } else {
                message.to_string()
            };
            // Truncated, because a stack trace is not a log line and a browser is
            // not a trusted length.
            let message: String = message.chars().take(4_000).collect();
            let source = pult_schema::ws::LogSource::Browser(
                deps.caller.map(short_id).unwrap_or_else(|| "?".into()),
            );
            log.emit(level, "browser", source, message);
            Ok(Value::Null)
        }
        "client.report" => {
            let caller = deps.caller.ok_or_else(|| {
                "client.report is a browser describing itself, so it needs one".to_string()
            })?;
            let clients = deps
                .clients
                .as_ref()
                .ok_or_else(|| "this station is not serving browsers".to_string())?;
            let stats: pult_schema::types::client::ClientStats =
                serde_json::from_value(args["stats"].clone())
                    .map_err(|e| format!("invalid stats: {e}"))?;
            // Trimmed for the same reason a reported message is: a browser is not a
            // trusted length, and this one ends up on every panel showing the station.
            let stats = pult_schema::types::client::ClientStats {
                label: stats.label.chars().take(120).collect(),
                ..stats
            };
            // What this station has sent down that socket since the page last
            // reported — the one figure here the page cannot honestly supply, because
            // no browser API says how many bytes arrived on a WebSocket. Drained
            // here, on the page's own schedule, so the window is the gap between two
            // of its reports.
            let sent_bytes = deps
                .ws_registry
                .as_ref()
                .map(|registry| registry.take_sent_bytes(caller))
                .unwrap_or(0);
            let key = clients.report(caller, stats, sent_bytes).await;
            // Answering the key it wrote under is the whole of how a page knows
            // which row in the panel is itself: a browser is not told its session id
            // anywhere else, and it must not be able to name one for itself.
            Ok(Value::String(key))
        }
        "output.watch" | "output.unwatch" => {
            let caller = deps.caller.ok_or_else(|| {
                "watching is about one client looking, so it needs one".to_string()
            })?;
            let output_id: uuid::Uuid = serde_json::from_value(args["outputId"].clone())
                .map_err(|e| format!("invalid outputId: {e}"))?;
            // The station that holds the socket, defaulting to this one — which is
            // what an output with no station of its own means anyway.
            let node = match args.get("nodeId") {
                Some(v) if !v.is_null() => pult_schema::events::operation::NodeId(
                    serde_json::from_value(v.clone())
                        .map_err(|e| format!("invalid nodeId: {e}"))?,
                ),
                _ => deps.node_id,
            };
            let moved = if method == "output.watch" {
                let focus = args["focus"].as_str().map(str::to_string);
                deps.viewers.watch(node, output_id, caller, focus)
            } else {
                deps.viewers.unwatch(node, output_id, caller)
            };
            // A peer's connector only draws while it is asked to, so the new answer
            // goes down the link — including the answer "nobody, stop". Nothing is
            // sent for an ask that did not move.
            if node != deps.node_id {
                if let (Some(sync), Some(ask)) = (&deps.sync, moved) {
                    sync.watch_peer_output(node, output_id, ask).await;
                }
            }
            Ok(Value::Null)
        }
        "selection.resolve" => {
            let group_id: uuid::Uuid = serde_json::from_value(args["groupId"].clone())
                .map_err(|e| format!("invalid groupId: {e}"))?;
            resolve_group(&deps.engine, group_id).await
        }
        "session.join" => {
            let session_id: uuid::Uuid = serde_json::from_value(args["sessionId"].clone())
                .map_err(|e| format!("invalid sessionId: {e}"))?;
            deps.session.join_session(session_id).await?;
            Ok(Value::Null)
        }
        "session.leave" => {
            let _ = deps
                .session
                .0
                .send(crate::infra::session::SessionCommand::Leave)
                .await;
            Ok(Value::Null)
        }
        "session.create" => {
            let show_name = args["showName"].as_str().unwrap_or("Untitled Show").to_string();
            let show_id: uuid::Uuid = serde_json::from_value(args["showId"].clone())
                .map_err(|e| format!("invalid showId: {e}"))?;
            match deps.session.create_session(show_name, show_id).await {
                Some(id) => serde_json::to_value(id).map_err(|e| e.to_string()),
                None => Err("failed to create session".into()),
            }
        }
        _ => Err(format!("unknown method: {method}")),
    }
}

/// Does this method name belong to the station rather than to an entity?
pub fn is_local_rpc(method: &str) -> bool {
    method.starts_with("session.")
        || method.starts_with("device.")
        || method.starts_with("selection.")
        || method.starts_with("parameter.")
        || method.starts_with("log.")
        || method.starts_with("output.")
        || method.starts_with("client.")
}

/// What a fixture's parameters are putting out, right now.
///
/// One parameter when named, and every one an operator can set when not — the two
/// shapes `__home` and `__set_home` take, so the trio reads as a trio. The answer is a
/// map keyed by parameter key, which is the same key `home_values` and `live_fades`
/// use, so a caller can line it up with anything else it has read.
///
/// Evaluated at one instant for the whole fixture. Two readings of the clock would put
/// a mover's pan and its tilt a millisecond apart, which is a pose nothing ever struck.
async fn what_is_it_doing(
    engine: &EngineHandle,
    fixture_id: uuid::Uuid,
    kind: Option<ParameterKind>,
) -> Result<Value, String> {
    let row = engine
        .get(vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture_id)])
        .await
        .map_err(|_| format!("there is no fixture {fixture_id}"))?;
    if row.is_null() {
        return Err(format!("there is no fixture {fixture_id}"));
    }
    let fixture: Fixture = serde_json::from_value(row)
        .map_err(|e| format!("fixture {fixture_id} does not parse: {e}"))?;

    let types = engine
        .get(vec![PathSegment::Key("fixture_types".into())])
        .await
        .map_err(|e| format!("cannot read the fixture types: {e}"))?;
    let types: Vec<FixtureType> = serde_json::from_value(types).unwrap_or_default();
    let fixture_type = types.iter().find(|t| t.id == fixture.fixture_type_id);

    let entries = engine
        .get(vec![PathSegment::Key("programmer_values".into())])
        .await
        .map_err(|e| format!("cannot read the programmer: {e}"))?;
    let entries: Vec<pult_schema::types::programmer::ProgrammerValue> =
        serde_json::from_value(entries).unwrap_or_default();
    let held = HeldByProgrammer::of(&entries);

    let keys: Vec<String> = match (&kind, fixture_type) {
        (Some(kind), _) => vec![parameter_key(kind)],
        (None, Some(fixture_type)) => {
            output_parameters(fixture_type).map(|p| parameter_key(&p.kind)).collect()
        }
        // Patched to a type this station has not got. Whatever the fixture overrides
        // for itself is still the truth about it, so that is what it can answer with.
        (None, None) => fixture.home_values.keys().cloned().collect(),
    };

    let now_ms = pult_schema::types::sequence::now_ms();
    let values: std::collections::HashMap<String, pult_schema::types::fixture::ParameterValue> =
        keys.into_iter()
            .filter_map(|key| {
                let driving = driving(&fixture, fixture_type, held.get(fixture_id, &key), &key);
                pult_render::value_at(&driving, now_ms).map(|value| (key, value))
            })
            .collect();
    serde_json::to_value(values).map_err(|e| e.to_string())
}

/// The fixtures a group picks out of the rig as it is now.
///
/// Reads the rig every time and caches nothing, which is what makes a group survive
/// a re-patch: a fixture hung this afternoon is in this morning's group without
/// anybody re-saving anything. The evaluator is `pult-schema`'s, the same one the
/// browser runs, held to it by `testdata/selection-queries.json`.
///
/// A group that is not there is an error rather than an empty answer, so a command
/// line can tell "you have no such group" from "that group is currently empty".
async fn resolve_group(engine: &EngineHandle, group_id: uuid::Uuid) -> Result<Value, String> {
    let row = engine
        .get(vec![PathSegment::Key("groups".into()), PathSegment::Id(group_id)])
        .await
        .map_err(|_| format!("there is no group {group_id}"))?;
    if row.is_null() {
        return Err(format!("there is no group {group_id}"));
    }
    let group: Group =
        serde_json::from_value(row).map_err(|e| format!("group {group_id} does not parse: {e}"))?;

    let rig = engine
        .get(vec![PathSegment::Key("fixtures".into())])
        .await
        .map_err(|e| format!("cannot read the rig: {e}"))?;
    let fixtures: Vec<Fixture> = serde_json::from_value(rig).unwrap_or_default();

    // A light on a truss is where the truss put it, so the objects it may hang off
    // are part of the question.
    let drawing = engine
        .get(vec![PathSegment::Key("scene_objects".into())])
        .await
        .map_err(|e| format!("cannot read the rig: {e}"))?;
    let objects: Vec<SceneObject> = serde_json::from_value(drawing).unwrap_or_default();

    let ids = evaluate(&group.query, &fixtures, None, &objects);
    serde_json::to_value(ids).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every listed RPC must reach a real arm in `dispatch`, and every arm must
    /// be listed: the table is what a command line offers, the match is what
    /// happens, and they must not drift apart. "Missing serial" and the like
    /// prove the arm was entered; only "unknown method" proves it was not.
    #[tokio::test]
    async fn the_table_and_the_dispatcher_agree() {
        let (session_tx, _session_rx) = tokio::sync::mpsc::channel(1);
        let (device_tx, _device_rx) = tokio::sync::mpsc::channel(1);
        let (engine_tx, _engine_rx) = tokio::sync::mpsc::channel(1);
        let deps = LocalRpcDeps {
            session: SessionHandle(session_tx),
            devices: DeviceHandle(device_tx),
            engine: EngineHandle(engine_tx),
            // Nothing here asks about the log, and a station without one is a
            // real configuration rather than a test fiction.
            log: None,
            log_watchers: Default::default(),
            sync: None,
            caller: None,
            clients: None,
            node_id: pult_schema::events::operation::NodeId::new(),
            viewers: Default::default(),
            ws_registry: None,
        };
        for meta in LOCAL_RPCS {
            assert!(is_local_rpc(meta.method), "{} is not routed here", meta.method);
            // Empty args, so every arm answers fast — a validation error, or a
            // shrug at the closed channels above — instead of waiting.
            if let Err(err) = dispatch(meta.method, Value::Null, &deps).await {
                assert!(
                    !err.starts_with("unknown method"),
                    "{} is listed but not dispatched",
                    meta.method
                );
            }
            let parsed: serde_json::Value =
                serde_json::from_str(meta.args_schema).expect("args_schema is JSON");
            assert!(parsed.is_array(), "{} args_schema is not an array", meta.method);
        }
    }

    /// A plugin, or anything else that can call but cannot read the rig, asking what
    /// a light is doing mid-fade — and getting the value for the moment it asked.
    ///
    /// This is what replaced reading a stored map. Nothing keeps the number any more,
    /// so the caller that could once have read it has to be able to ask instead, and
    /// asking must not write anything down: it is a read, so it is an RPC rather than
    /// a command.
    #[tokio::test]
    async fn asking_what_a_parameter_is_doing_answers_for_the_moment_it_was_asked() {
        use pult_schema::{
            lifecycle::Lifecycle,
            types::fixture::{
                FixtureType, ParameterDefinition,
                ParameterKind, ParameterValue,
            },
        };

        let pool =
            std::sync::Arc::new(crate::infra::showfile::open_in_memory().await.expect("showfile"));
        let (engine, handle, _broadcast) = crate::engine::ShowEngine::new(
            pult_schema::events::operation::NodeId(uuid::Uuid::new_v4()),
            pool,
            None,
        );
        tokio::spawn(engine.run());

        let fixture_type = FixtureType {
            id: uuid::Uuid::new_v4(),
            name: "Dimmer".into(),
            manufacturer: "Acme".into(),
            channel_count: 1,
            parameters: vec![ParameterDefinition::new(
                ParameterKind::Intensity,
                ParameterValue::Float(0.0),
            )],
            ..FixtureType::default()
        };
        let fixture_id = uuid::Uuid::new_v4();
        let create = |table: &str| {
            vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())]
        };
        handle
            .set(
                create("fixture_types"),
                Lifecycle::Persisted,
                serde_json::to_value(&fixture_type).unwrap(),
            )
            .await
            .unwrap();
        handle
            .set(
                create("fixtures"),
                Lifecycle::Persisted,
                serde_json::json!({
                    "id": fixture_id,
                    "name": "Spot",
                    "fixture_type_id": fixture_type.id,
                    "address": { "Dmx": { "mode": "Default", "breaks": [{ "universe": 1, "address": 1 }] } },
                    "position": null,
                }),
            )
            .await
            .unwrap();

        // Nothing driving it: it answers with where it rests.
        let deps = LocalRpcDeps {
            session: SessionHandle(tokio::sync::mpsc::channel(1).0),
            devices: DeviceHandle(tokio::sync::mpsc::channel(1).0),
            engine: handle.clone(),
            // Nothing here asks about the log, and a station without one is a
            // real configuration rather than a test fiction.
            log: None,
            log_watchers: Default::default(),
            sync: None,
            caller: None,
            clients: None,
            node_id: pult_schema::events::operation::NodeId::new(),
            viewers: Default::default(),
            ws_registry: None,
        };
        let ask = || {
            dispatch("parameter.value", serde_json::json!({ "fixtureId": fixture_id }), &deps)
        };
        assert_eq!(ask().await.unwrap()["Intensity"]["value"], 0.0, "where it rests");

        // A four second fade, one second old.
        let now = pult_schema::types::sequence::now_ms();
        handle
            .set(
                vec![
                    PathSegment::Key("fixtures".into()),
                    PathSegment::Id(fixture_id),
                    PathSegment::Key("live_fades".into()),
                ],
                Lifecycle::Local,
                serde_json::json!({
                    "Intensity": {
                        "from": ParameterValue::Float(0.0),
                        "to": ParameterValue::Float(1.0),
                        "t0": now - 1_000,
                        "duration_ms": 4_000,
                        "easing": "Linear",
                        "cue_id": uuid::Uuid::nil(),
                    }
                }),
            )
            .await
            .unwrap();

        let first = ask().await.unwrap()["Intensity"]["value"].as_f64().unwrap();
        assert!(first > 0.2 && first < 0.35, "a quarter of the way up: {first}");

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let later = ask().await.unwrap()["Intensity"]["value"].as_f64().unwrap();
        assert!(later > first, "and it has moved on since, with nothing written: {later}");
    }
}
