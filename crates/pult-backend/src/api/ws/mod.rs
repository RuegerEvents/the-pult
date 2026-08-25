use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use pult_schema::{
    path::PathPattern,
    ws::{ClientMessage, ServerMessage},
};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::state::AppState;

// ── Subscription registry ─────────────────────────────────────────────────────

type SessionId = Uuid;

#[derive(Default, Clone)]
pub struct SubscriptionRegistry(Arc<Mutex<RegistryInner>>);

#[derive(Default)]
struct RegistryInner {
    sessions: HashMap<SessionId, mpsc::UnboundedSender<ServerMessage>>,
    subscriptions: HashMap<SessionId, Vec<PathPattern>>,
}

impl SubscriptionRegistry {
    pub fn add_session(&self, id: SessionId, tx: mpsc::UnboundedSender<ServerMessage>) {
        self.0.lock().unwrap().sessions.insert(id, tx);
    }

    pub fn remove_session(&self, id: SessionId) {
        let mut inner = self.0.lock().unwrap();
        inner.sessions.remove(&id);
        inner.subscriptions.remove(&id);
    }

    pub fn subscribe(&self, id: SessionId, pattern: PathPattern) {
        self.0
            .lock()
            .unwrap()
            .subscriptions
            .entry(id)
            .or_default()
            .push(pattern);
    }

    pub fn unsubscribe(&self, id: SessionId, pattern: &PathPattern) {
        if let Some(patterns) = self.0.lock().unwrap().subscriptions.get_mut(&id) {
            patterns.retain(|p| p != pattern);
        }
    }

    pub fn broadcast_update(
        &self,
        path: &pult_schema::path::Path,
        value: serde_json::Value,
    ) {
        let inner = self.0.lock().unwrap();
        for (session_id, patterns) in &inner.subscriptions {
            if patterns.iter().any(|p| p.matches(path)) {
                if let Some(tx) = inner.sessions.get(session_id) {
                    let _ = tx.send(ServerMessage::Update {
                        path: path.clone(),
                        value: value.clone(),
                    });
                }
            }
        }
    }
}

// ── WebSocket upgrade handler ─────────────────────────────────────────────────

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let session_id = Uuid::new_v4();
    let (mut sink, mut stream) = socket.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<ServerMessage>();

    state.ws_registry.add_session(session_id, outgoing_tx);
    debug!("WebSocket session {session_id} connected");

    // Spawn task to forward outgoing messages to the WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = outgoing_rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if sink.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => warn!("serialize server message: {e}"),
            }
        }
    });

    // Forward engine updates to subscribed sessions in a background task
    let registry = state.ws_registry.clone();
    let broadcast = state.broadcast.clone();
    let broadcast_task = tokio::spawn(async move {
        let mut stream = broadcast.subscribe_all();
        while let Some((path, value)) = stream.next().await {
            registry.broadcast_update(&path, value);
        }
    });

    // Handle incoming messages
    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        let client_msg: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                debug!("failed to parse client message: {e}");
                continue;
            }
        };

        handle_client_message(client_msg, session_id, &state).await;
    }

    state.ws_registry.remove_session(session_id);
    send_task.abort();
    broadcast_task.abort();
    debug!("WebSocket session {session_id} disconnected");
}

async fn handle_client_message(
    msg: ClientMessage,
    session_id: SessionId,
    state: &AppState,
) {
    match msg {
        ClientMessage::Subscribe { pattern } => {
            state.ws_registry.subscribe(session_id, pattern);
        }

        ClientMessage::Unsubscribe { pattern } => {
            state.ws_registry.unsubscribe(session_id, &pattern);
        }

        ClientMessage::Get { path, request_id } => {
            // Always resolve the request — return null for paths that don't exist yet
            // (e.g. 'show' on a fresh database). Returning Error would leave the
            // client's pending promise hanging forever because Error has no request_id.
            let value = state.engine.get(path.clone()).await.unwrap_or(serde_json::Value::Null);
            send_to_session(state, session_id, ServerMessage::GetResult { path, value, request_id });
        }

        ClientMessage::Set { path, value, request_id } => {
            // Determine lifecycle from path — all top-level sets default to Persisted
            // unless the path corresponds to a known SYNCED field.
            let lifecycle = infer_lifecycle(&path);
            let result = state.engine.set(path.clone(), lifecycle, value.clone()).await;
            let msg = match result {
                Ok(()) => ServerMessage::SetAck { request_id, ok: true, error: None },
                Err(e) => ServerMessage::SetAck {
                    request_id,
                    ok: false,
                    error: Some(e.to_string()),
                },
            };
            send_to_session(state, session_id, msg);
        }

        ClientMessage::Call { method, args, request_id } => {
            let msg = if method.starts_with("session.") {
                handle_session_call(&method, args, state).await
                    .map(|v| ServerMessage::CallResult { request_id: request_id.clone(), result: Some(v), error: None })
                    .unwrap_or_else(|e| ServerMessage::CallResult {
                        request_id,
                        result: None,
                        error: Some(e),
                    })
            } else {
                match state.engine.call(method, args).await {
                    Ok(result) => ServerMessage::CallResult {
                        request_id,
                        result: Some(result),
                        error: None,
                    },
                    Err(e) => ServerMessage::CallResult {
                        request_id,
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            };
            send_to_session(state, session_id, msg);
        }

        ClientMessage::Ping => {
            send_to_session(state, session_id, ServerMessage::Pong);
        }
    }
}

async fn handle_session_call(
    method: &str,
    args: serde_json::Value,
    state: &AppState,
) -> Result<serde_json::Value, String> {
    match method {
        "session.join" => {
            let session_id: uuid::Uuid = serde_json::from_value(args["sessionId"].clone())
                .map_err(|e| format!("invalid sessionId: {e}"))?;
            state.session.join_session(session_id).await.map_err(|e| e)?;
            Ok(serde_json::Value::Null)
        }
        "session.leave" => {
            let _ = state.session.0.send(crate::infra::session::SessionCommand::Leave).await;
            Ok(serde_json::Value::Null)
        }
        "session.create" => {
            let show_name = args["showName"]
                .as_str()
                .unwrap_or("Untitled Show")
                .to_string();
            let show_id: uuid::Uuid = serde_json::from_value(args["showId"].clone())
                .map_err(|e| format!("invalid showId: {e}"))?;
            let session_id = state.session.create_session(show_name, show_id).await;
            match session_id {
                Some(id) => serde_json::to_value(id).map_err(|e| e.to_string()),
                None => Err("failed to create session".into()),
            }
        }
        _ => Err(format!("unknown session method: {method}")),
    }
}

fn send_to_session(state: &AppState, id: SessionId, msg: ServerMessage) {
    let inner = state.ws_registry.0.lock().unwrap();
    if let Some(tx) = inner.sessions.get(&id) {
        let _ = tx.send(msg);
    }
}

fn infer_lifecycle(path: &pult_schema::path::Path) -> pult_schema::lifecycle::Lifecycle {
    pult_schema::registry::path_lifecycle(path)
}
