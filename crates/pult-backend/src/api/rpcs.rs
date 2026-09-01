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
    types::{evaluate, Fixture, Group},
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
}
