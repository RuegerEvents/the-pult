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
    /// What has been sent down each socket since that browser last reported.
    ///
    /// An atomic per session, held here and cloned into the socket's own send task,
    /// so counting a message costs an add and never the registry's lock — which is
    /// taken on the broadcast path for every update to every browser.
    ///
    /// The station's figure rather than the page's, because a page cannot see its own
    /// socket: there is no browser API that says how many bytes arrived on a
    /// WebSocket. So it goes in beside the other two things the station fills in for
    /// a page it will not take an unchecked word from.
    sent_bytes: HashMap<SessionId, Arc<std::sync::atomic::AtomicU64>>,
    /// Who each socket says it is, for attributing its writes.
    ///
    /// Per connection rather than per user: two browsers can be the same person, and
    /// the point of the identity is that they then share one undo history.
    users: HashMap<SessionId, Uuid>,
    subscriptions: HashMap<SessionId, Vec<PathPattern>>,
}

impl SubscriptionRegistry {
    /// Register a socket, and answer the counter its send task should add to.
    pub fn add_session(
        &self,
        id: SessionId,
        tx: mpsc::UnboundedSender<ServerMessage>,
    ) -> Arc<std::sync::atomic::AtomicU64> {
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut inner = self.0.lock().unwrap();
        inner.sessions.insert(id, tx);
        inner.sent_bytes.insert(id, counter.clone());
        counter
    }

    /// Take what has been sent to this socket since the last time anybody asked.
    ///
    /// Zero for a session that has gone, which is right: the row it would have gone
    /// on goes with it.
    pub fn take_sent_bytes(&self, id: SessionId) -> u64 {
        self.0
            .lock()
            .unwrap()
            .sent_bytes
            .get(&id)
            .map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Remember who this socket says it is, or forget if it says nobody.
    pub fn identify(&self, id: SessionId, user_id: Option<Uuid>) {
        let mut inner = self.0.lock().unwrap();
        match user_id {
            Some(user) => inner.users.insert(id, user),
            None => inner.users.remove(&id),
        };
    }

    pub fn user_of(&self, id: SessionId) -> Option<Uuid> {
        self.0.lock().unwrap().users.get(&id).copied()
    }

    /// Who this socket's writes belong to.
    ///
    /// A client that has not said falls back to the show's default operator rather
    /// than to nobody, because a write carrying no author can never be taken back —
    /// not later, and not once the operator finally says who they are. Every show has
    /// that user: the engine seeds it when it loads one.
    ///
    /// The fallback is here rather than only in the browser so that the guarantee does
    /// not depend on a well-behaved client. The engine's *own* writes — a fade
    /// advancing, a station publishing its memory use — never come through here, and
    /// stay nobody's.
    pub fn user_for_writes(&self, id: SessionId) -> Uuid {
        self.user_of(id).unwrap_or(pult_schema::types::user::User::DEFAULT_ID)
    }

    pub fn remove_session(&self, id: SessionId) {
        let mut inner = self.0.lock().unwrap();
        inner.sessions.remove(&id);
        inner.users.remove(&id);
        inner.subscriptions.remove(&id);
        inner.sent_bytes.remove(&id);
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

    let sent_bytes = state.ws_registry.add_session(session_id, outgoing_tx);
    debug!("WebSocket session {session_id} connected");

    // Spawn task to forward outgoing messages to the WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = outgoing_rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    // Counted before the send and after the encode: the length of the
                    // frame this station handed the socket. What TCP then does with
                    // it — a header per segment, a retransmit — is not this figure.
                    sent_bytes.fetch_add(json.len() as u64, std::sync::atomic::Ordering::Relaxed);
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
    // Whatever this browser had a peer raised to, it no longer wants. This is the
    // whole of the unwind: nothing expires, because an ask is recomputed from who
    // is actually here whenever that changes — and a browser closing its tab is
    // exactly that change. A peer this station is no longer connected to is simply
    // not told, which is right: its raise died with the connection.
    for (node_id, level) in state.log_watchers.forget_session(session_id) {
        state.sync.raise_peer_log(pult_schema::events::operation::NodeId(node_id), level).await;
    }
    // And whatever it last said it was costing. A client row is a reading of a live
    // page, so a page that has gone leaves none — the sweep in `infra::clients` is
    // only for the other way of going quiet, which is not hanging up.
    state.clients.forget(session_id).await;
    // And whatever it was watching leave the console. Same rule as the raised log:
    // recomputed from who is here, so a tab that closes stops a connector drawing
    // for it — here, or on the peer that was drawing on its behalf.
    for ((node_id, output_id), ask) in state.viewers.forget(session_id) {
        if node_id != state.node_id {
            state.sync.watch_peer_output(node_id, output_id, ask).await;
        }
    }
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

        ClientMessage::Identify { user_id } => {
            state.ws_registry.identify(session_id, user_id);
        }

        ClientMessage::Undo { redo, request_id } => {
            // The same user its writes are attributed to, so a client takes back its
            // own work — including a client that never said who it is, whose work is
            // the default operator's.
            let user_id = state.ws_registry.user_for_writes(session_id);
            let moved = state.engine.undo(user_id, redo).await;
            // Named by the first path written, which is the newest thing the gesture
            // touched and so the one the operator last saw move.
            let msg = ServerMessage::UndoResult {
                request_id,
                changed: moved.len() as u32,
                undone: moved.into_iter().next(),
            };
            send_to_session(state, session_id, msg);
        }

        ClientMessage::History { limit, request_id } => {
            let log = state.engine.history(limit.min(500)).await;
            let entries = log
                .iter()
                .map(|op| pult_schema::ws::HistoryEntry {
                    id: op.id,
                    user_id: op.user_id,
                    path: op.path.clone(),
                    at: op.timestamp.to_rfc3339(),
                    undoes: op.undoes,
                    undoable: op.is_undoable(),
                })
                .collect();
            send_to_session(state, session_id, ServerMessage::HistoryResult { request_id, entries });
        }

        ClientMessage::Set { path, value, request_id, gesture } => {
            // Determine lifecycle from path — all top-level sets default to Persisted
            // unless the path corresponds to a known SYNCED field.
            let lifecycle = infer_lifecycle(&path);
            // Always attributed, so it can always be taken back. A client that has
            // not said who it is writes as the show's default operator rather than as
            // nobody — an unattributed write is one that can never be undone, which is
            // a worse answer than a shared one.
            let user_id = state.ws_registry.user_for_writes(session_id);
            let result =
                state.engine.set_as(user_id, gesture, path.clone(), lifecycle, value.clone()).await;
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
            let msg = if let Some(rest) = method.strip_prefix("plugin.") {
                // `plugin.<id>.<method...>` goes to that plugin's `rpc.handle`.
                let (plugin, plugin_method) = rest.split_once('.').unwrap_or((rest, ""));
                state
                    .plugins
                    .call(plugin.to_string(), plugin_method.to_string(), args)
                    .await
                    .map(|v| ServerMessage::CallResult { request_id: request_id.clone(), result: Some(v), error: None })
                    .unwrap_or_else(|e| ServerMessage::CallResult {
                        request_id,
                        result: None,
                        error: Some(e),
                    })
            } else if crate::api::rpcs::is_local_rpc(&method) {
                let deps = crate::api::rpcs::LocalRpcDeps {
                    session: state.session.clone(),
                    devices: state.devices.clone(),
                    engine: state.engine.clone(),
                    log: state.config.log.clone(),
                    log_watchers: state.log_watchers.clone(),
                    node_id: state.node_id,
                    viewers: state.viewers.clone(),
                    sync: Some(state.sync.clone()),
                    // Which browser is asking, so a watch can end when it does.
                    caller: Some(session_id),
                    clients: Some(state.clients.clone()),
                    ws_registry: Some(state.ws_registry.clone()),
                };
                crate::api::rpcs::dispatch(&method, args, &deps).await
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

        ClientMessage::ClockSync { sent_at } => {
            // Answered inline rather than through the engine: the number wanted is
            // what the show clock says *now*, and queueing the question behind
            // whatever the engine is doing would put that delay inside the round trip
            // the client is about to halve.
            send_to_session(
                state,
                session_id,
                ServerMessage::ClockSync {
                    sent_at,
                    station_ms: pult_schema::types::sequence::now_ms() as f64,
                },
            );
        }

        ClientMessage::Ping => {
            send_to_session(state, session_id, ServerMessage::Pong);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::user::User;

    /// The property the default user exists for, at the seam that decides it: a
    /// socket's writes always belong to somebody, so they can always be taken back.
    #[test]
    fn a_socket_that_never_said_writes_as_the_operator() {
        let registry = SubscriptionRegistry::default();
        let session = Uuid::new_v4();

        assert_eq!(registry.user_of(session), None, "it has not said");
        assert_eq!(
            registry.user_for_writes(session),
            User::DEFAULT_ID,
            "but its writes are the operator's rather than nobody's"
        );
    }

    #[test]
    fn a_socket_that_said_writes_as_who_it_said() {
        let registry = SubscriptionRegistry::default();
        let session = Uuid::new_v4();
        let sam = Uuid::new_v4();

        registry.identify(session, Some(sam));

        assert_eq!(registry.user_for_writes(session), sam);
    }

    /// `Identify { user_id: None }` is what a client built before this change sends
    /// when somebody signs out. It forgets who they were, and the socket falls back
    /// to the operator — not to a state where nothing can be undone.
    #[test]
    fn signing_out_lands_on_the_operator_rather_than_on_nobody() {
        let registry = SubscriptionRegistry::default();
        let session = Uuid::new_v4();
        registry.identify(session, Some(Uuid::new_v4()));

        registry.identify(session, None);

        assert_eq!(registry.user_of(session), None, "it has forgotten who they were");
        assert_eq!(registry.user_for_writes(session), User::DEFAULT_ID, "and is not nobody");
    }
}
