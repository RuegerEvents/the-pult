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
        Fixture, Group,
    },
};
use serde_json::Value;

use crate::{
    engine::EngineHandle,
    infra::{devices::DeviceHandle, session::SessionHandle},
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
];

/// What dispatching needs to reach. Cheap to clone: three channel handles.
#[derive(Clone)]
pub struct LocalRpcDeps {
    pub session: SessionHandle,
    pub devices: DeviceHandle,
    /// For the calls that answer a question about the show. Reads only — anything
    /// here that wanted to write would be an entity command instead.
    pub engine: EngineHandle,
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

    let ids = evaluate(&group.query, &fixtures, None);
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
