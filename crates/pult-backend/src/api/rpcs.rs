//! The station RPCs: calls against LOCAL state, which the engine's command
//! registry knows nothing about — they go to the manager that owns the state
//! rather than to an entity.
//!
//! One table and one dispatcher, so the WebSocket handler and the plugin
//! runtime answer the same calls the same way, and introspection can list them
//! without a second place going stale. A test below holds the table and the
//! match to each other.

use serde_json::Value;

use crate::infra::{devices::DeviceHandle, session::SessionHandle};

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
];

/// What dispatching needs to reach. Cheap to clone: two channel handles.
#[derive(Clone)]
pub struct LocalRpcDeps {
    pub session: SessionHandle,
    pub devices: DeviceHandle,
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
    method.starts_with("session.") || method.starts_with("device.")
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
        let deps = LocalRpcDeps {
            session: SessionHandle(session_tx),
            devices: DeviceHandle(device_tx),
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
