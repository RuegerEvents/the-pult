use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::BoxStream;
use pult_schema::{
    commands::CommandRegistration,
    events::operation::{NodeId, Operation, VectorClock},
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    types::{
        cue::Cue,
        fixture::Fixture,
        sequence::Sequence,
        session::SessionState,
        show::Show,
    },
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    error::BackendError,
    infra::{showfile::queries as db, sync::SyncHandle},
};

// ── In-memory show state ──────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ShowState {
    #[serde(default)]
    pub show: Option<Show>,
    #[serde(default)]
    pub fixtures: HashMap<Uuid, Fixture>,
    #[serde(default)]
    pub sequences: HashMap<Uuid, Sequence>,
    #[serde(default)]
    pub cues: HashMap<Uuid, Cue>,
    /// Derived ordering — not stored in DB, rebuilt by post_load_init().
    #[serde(default)]
    pub sequence_order: Vec<Uuid>,
    /// LOCAL lifecycle: broadcast to frontends, not persisted or synced to peers.
    /// Written only by SessionManager.
    #[serde(default)]
    pub session: SessionState,
}

impl ShowState {
    /// Top-level collection keys broadcast to frontends.
    /// Add a new entry here when adding a new entity collection to this struct.
    const FRONTEND_PATHS: &'static [&'static str] = &["show", "fixtures", "sequences", "cues"];

    /// Fix up derived fields after bulk-loading state (e.g. from showfile or snapshot).
    pub fn post_load_init(&mut self) {
        if self.sequence_order.is_empty() {
            self.sequence_order = self.sequences.keys().copied().collect();
        }
    }

    pub fn get_by_path(&self, path: &Path) -> Option<serde_json::Value> {
        // Arm order matters: __create and __delete must be matched before the generic
        // [collection, id, field] patch arm, or "__delete" is taken for a field name,
        // dropped by serde as unknown, and the delete silently succeeds without deleting.
        match path.as_slice() {
            [PathSegment::Key(k)] if k == "show" => {
                self.show.as_ref().and_then(|s| serde_json::to_value(s).ok())
            }
            [PathSegment::Key(k)] if k == "fixtures" => {
                let list: Vec<&Fixture> = self.fixtures.values().collect();
                serde_json::to_value(list).ok()
            }
            [PathSegment::Key(k), PathSegment::Id(id)] if k == "fixtures" => {
                self.fixtures.get(id).and_then(|f| serde_json::to_value(f).ok())
            }
            [PathSegment::Key(k)] if k == "sequences" => {
                let ordered: Vec<&Sequence> = self
                    .sequence_order
                    .iter()
                    .filter_map(|id| self.sequences.get(id))
                    .collect();
                serde_json::to_value(ordered).ok()
            }
            [PathSegment::Key(k), PathSegment::Index(n)] if k == "sequences" => {
                self.sequence_order
                    .get(*n)
                    .and_then(|id| self.sequences.get(id))
                    .and_then(|s| serde_json::to_value(s).ok())
            }
            [PathSegment::Key(k), PathSegment::Id(id)] if k == "sequences" => {
                self.sequences.get(id).and_then(|s| serde_json::to_value(s).ok())
            }
            [PathSegment::Key(k)] if k == "cues" => {
                let list: Vec<&Cue> = self.cues.values().collect();
                serde_json::to_value(list).ok()
            }
            [PathSegment::Key(k), PathSegment::Id(id)] if k == "cues" => {
                self.cues.get(id).and_then(|c| serde_json::to_value(c).ok())
            }
            [PathSegment::Key(k)] if k == "session" => {
                serde_json::to_value(&self.session).ok()
            }
            _ => None,
        }
    }
}

// ── EngineCommand ─────────────────────────────────────────────────────────────

pub enum EngineCommand {
    Set {
        path: Path,
        value: serde_json::Value,
        lifecycle: Lifecycle,
        reply: oneshot::Sender<Result<(), BackendError>>,
    },
    Get {
        path: Path,
        reply: oneshot::Sender<Result<serde_json::Value, BackendError>>,
    },
    Subscribe {
        pattern: PathPattern,
        reply: oneshot::Sender<BoxStream<'static, serde_json::Value>>,
    },
    /// Legacy method-string dispatch; kept for backward compat with existing WS protocol.
    Call {
        method: String,
        args: serde_json::Value,
        reply: oneshot::Sender<Result<serde_json::Value, BackendError>>,
    },
    ApplyPeerOperation(Operation),
    LoadFromShowfile,
    /// Full ShowState serialized to JSON — sent by the leader to newly joined peers.
    GetSnapshot {
        reply: oneshot::Sender<serde_json::Value>,
    },
    /// Apply a snapshot received from the session leader. Replaces in-memory state
    /// and broadcasts updates to connected frontends.
    ApplyStateSnapshot(serde_json::Value),
    Stop,
}

// ── EngineHandle ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct EngineHandle(pub mpsc::Sender<EngineCommand>);

impl EngineHandle {
    pub async fn set(
        &self,
        path: Path,
        lifecycle: Lifecycle,
        value: serde_json::Value,
    ) -> Result<(), BackendError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(EngineCommand::Set { path, value, lifecycle, reply: tx })
            .await
            .map_err(|_| BackendError::ChannelClosed)?;
        rx.await?
    }

    pub async fn get(&self, path: Path) -> Result<serde_json::Value, BackendError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(EngineCommand::Get { path, reply: tx })
            .await
            .map_err(|_| BackendError::ChannelClosed)?;
        rx.await?
    }

    pub async fn subscribe_pattern(&self, pattern: PathPattern) -> BoxStream<'static, serde_json::Value> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(EngineCommand::Subscribe { pattern, reply: tx }).await;
        rx.await.unwrap_or_else(|_| Box::pin(futures::stream::empty()))
    }

    pub async fn get_snapshot(&self) -> serde_json::Value {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(EngineCommand::GetSnapshot { reply: tx }).await;
        rx.await.unwrap_or(serde_json::Value::Object(Default::default()))
    }

    pub async fn apply_state_snapshot(&self, snapshot: serde_json::Value) {
        let _ = self.0.send(EngineCommand::ApplyStateSnapshot(snapshot)).await;
    }

    pub async fn call(
        &self,
        method: String,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(EngineCommand::Call { method, args, reply: tx })
            .await
            .map_err(|_| BackendError::ChannelClosed)?;
        rx.await?
    }
}

// ── UpdateBroadcast ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct UpdateBroadcast(pub broadcast::Sender<(Path, serde_json::Value)>);

impl UpdateBroadcast {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self(tx)
    }

    pub fn subscribe_filtered(&self, pattern: PathPattern) -> BoxStream<'static, serde_json::Value> {
        use futures::StreamExt;
        let mut rx = self.0.subscribe();
        Box::pin(async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok((path, value)) if pattern.matches(&path) => yield value,
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("subscriber lagged by {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }

    pub fn subscribe_all(&self) -> BoxStream<'static, (Path, serde_json::Value)> {
        use futures::StreamExt;
        let mut rx = self.0.subscribe();
        Box::pin(async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(pair) => yield pair,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("ws broadcast subscriber lagged by {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

// ── Command dispatch table ────────────────────────────────────────────────────

type CommandHandler = fn(serde_json::Value, serde_json::Value) -> anyhow::Result<serde_json::Value>;

type CommandTable = HashMap<(String, &'static str), CommandHandler>;

fn build_command_table() -> HashMap<(String, &'static str), CommandHandler> {
    inventory::iter::<CommandRegistration>()
        .map(|r| (((r.entity_table)().to_string(), r.command_name), r.handler))
        .collect()
}

// ── ShowEngine ────────────────────────────────────────────────────────────────

pub struct ShowEngine {
    node_id: NodeId,
    next_seq: u64,
    clock: VectorClock,
    state: ShowState,
    rx: mpsc::Receiver<EngineCommand>,
    broadcast: UpdateBroadcast,
    pool: Arc<SqlitePool>,
    sync: Option<SyncHandle>,
    commands: CommandTable,
}

impl ShowEngine {
    pub fn new(
        node_id: NodeId,
        pool: Arc<SqlitePool>,
        sync: Option<SyncHandle>,
    ) -> (Self, EngineHandle, UpdateBroadcast) {
        let (tx, rx) = mpsc::channel(256);
        let (engine, broadcast) = Self::new_with_rx(node_id, rx, pool, sync);
        (engine, EngineHandle(tx), broadcast)
    }

    pub fn new_with_rx(
        node_id: NodeId,
        rx: mpsc::Receiver<EngineCommand>,
        pool: Arc<SqlitePool>,
        sync: Option<SyncHandle>,
    ) -> (Self, UpdateBroadcast) {
        let broadcast = UpdateBroadcast::new();
        let engine = ShowEngine {
            node_id,
            next_seq: 1,
            clock: VectorClock::default(),
            state: ShowState::default(),
            rx,
            broadcast: broadcast.clone(),
            pool,
            sync,
            commands: build_command_table(),
        };
        (engine, broadcast)
    }

    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                EngineCommand::Stop => break,
                EngineCommand::LoadFromShowfile => {
                    self.load_from_showfile().await;
                }
                EngineCommand::Get { path, reply } => {
                    let result = self
                        .state
                        .get_by_path(&path)
                        .ok_or_else(|| BackendError::PathNotFound(path));
                    let _ = reply.send(result);
                }
                EngineCommand::Set { path, value, lifecycle, reply } => {
                    let result = self.apply_set(path.clone(), value.clone(), lifecycle).await;
                    if result.is_ok() {
                        self.broadcast_after_set(&path, value.clone());
                        if lifecycle != Lifecycle::Local {
                            if let Some(sync) = &self.sync {
                                sync.broadcast_synced(path, value, self.clock.clone()).await;
                            }
                        }
                    }
                    let _ = reply.send(result);
                }
                EngineCommand::Subscribe { pattern, reply } => {
                    let stream = self.broadcast.subscribe_filtered(pattern);
                    let _ = reply.send(stream);
                }
                EngineCommand::Call { method, args, reply } => {
                    let result = self.handle_call_legacy(&method, args).await;
                    let _ = reply.send(result);
                }
                EngineCommand::ApplyPeerOperation(op) => {
                    self.apply_peer_operation(op).await;
                }
                EngineCommand::GetSnapshot { reply } => {
                    let snapshot = self.build_snapshot();
                    let _ = reply.send(snapshot);
                }
                EngineCommand::ApplyStateSnapshot(snapshot) => {
                    self.apply_snapshot(snapshot).await;
                }
            }
        }
    }

    async fn apply_set(
        &mut self,
        path: Path,
        value: serde_json::Value,
        lifecycle: Lifecycle,
    ) -> Result<(), BackendError> {
        self.next_seq += 1;
        self.clock.increment(self.node_id);

        // ── Path-based command dispatch ──
        // Pattern: [Key(entity_collection), Id(id), Key(command_name)]
        if let [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(cmd)] = path.as_slice() {
            if let Some(handler) = self.commands.get(&(k.clone(), cmd.as_str())) {
                return self.dispatch_command(*handler, k, *id, value).await;
            }
        }

        // Arm order matters: __create and __delete must be matched before the generic
        // [collection, id, field] patch arm, or "__delete" is taken for a field name,
        // dropped by serde as unknown, and the delete silently succeeds without deleting.
        match path.as_slice() {
            [PathSegment::Key(k)] if k == "show" => {
                let show: Show = serde_json::from_value(value)?;
                if lifecycle == Lifecycle::Persisted {
                    db::upsert(&self.pool, &show).await?;
                }
                self.state.show = Some(show);
            }
            [PathSegment::Key(k), PathSegment::Key(action)] if k == "fixtures" && action == "__create" => {
                let fixture: Fixture = serde_json::from_value(value)?;
                let id = fixture.id;
                db::upsert(&self.pool, &fixture).await?;
                self.state.fixtures.insert(id, fixture);
            }
            [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(action)]
                if k == "fixtures" && action == "__delete" =>
            {
                db::delete::<Fixture>(&self.pool, *id).await?;
                self.state.fixtures.remove(id);
            }
            [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(field)] if k == "fixtures" => {
                if let Some(fixture) = self.state.fixtures.get_mut(id) {
                    apply_field_patch(fixture, field, value.clone())?;
                    if lifecycle == Lifecycle::Persisted {
                        db::upsert(&self.pool, fixture as &Fixture).await?;
                    }
                } else {
                    return Err(BackendError::PathNotFound(path));
                }
            }
            [PathSegment::Key(k), PathSegment::Key(action)] if k == "sequences" && action == "__create" => {
                let seq: Sequence = serde_json::from_value(value)?;
                let id = seq.id;
                db::upsert(&self.pool, &seq).await?;
                self.state.sequences.insert(id, seq);
                self.state.sequence_order.push(id);
            }
            [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(action)]
                if k == "sequences" && action == "__delete" =>
            {
                db::delete::<Sequence>(&self.pool, *id).await?;
                self.state.sequences.remove(id);
                self.state.sequence_order.retain(|i| i != id);
            }
            [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(field)] if k == "sequences" => {
                if let Some(seq) = self.state.sequences.get_mut(id) {
                    apply_field_patch(seq, field, value.clone())?;
                    if lifecycle == Lifecycle::Persisted {
                        db::upsert(&self.pool, seq as &Sequence).await?;
                    }
                } else {
                    return Err(BackendError::PathNotFound(path));
                }
            }
            [PathSegment::Key(k), PathSegment::Key(action)] if k == "cues" && action == "__create" => {
                let cue: Cue = serde_json::from_value(value)?;
                let id = cue.id;
                db::upsert(&self.pool, &cue).await?;
                self.state.cues.insert(id, cue);
            }
            [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(action)]
                if k == "cues" && action == "__delete" =>
            {
                db::delete::<Cue>(&self.pool, *id).await?;
                self.state.cues.remove(id);
            }
            [PathSegment::Key(k), PathSegment::Id(id), PathSegment::Key(field)] if k == "cues" => {
                if let Some(cue) = self.state.cues.get_mut(id) {
                    apply_field_patch(cue, field, value.clone())?;
                    if lifecycle == Lifecycle::Persisted {
                        db::upsert(&self.pool, cue as &Cue).await?;
                    }
                } else {
                    return Err(BackendError::PathNotFound(path));
                }
            }
            // Session state is LOCAL — written only by SessionManager, never persisted or synced.
            [PathSegment::Key(k)] if k == "session" => {
                self.state.session = serde_json::from_value(value)?;
            }
            _ => {
                debug!("unhandled set path: {path:?}");
                return Err(BackendError::PathNotFound(path));
            }
        }
        Ok(())
    }

    /// Dispatch a path-based command: look up entity by id, run handler, update state, broadcast.
    async fn dispatch_command(
        &mut self,
        handler: CommandHandler,
        entity_key: &str,
        id: Uuid,
        args: serde_json::Value,
    ) -> Result<(), BackendError> {
        let entity_path = vec![
            PathSegment::Key(entity_key.into()),
            PathSegment::Id(id),
        ];
        let entity_json = self.state.get_by_path(&entity_path)
            .ok_or_else(|| BackendError::PathNotFound(entity_path.clone()))?;

        let result_json = handler(entity_json, args).map_err(|e| BackendError::InvalidValue {
            path: entity_path.clone(),
            reason: e.to_string(),
        })?;

        // Apply result back to in-memory state
        self.apply_entity_result(entity_key, id, result_json.clone())?;

        // Broadcast full entity update
        let _ = self.broadcast.0.send((entity_path, result_json));
        Ok(())
    }

    fn apply_entity_result(
        &mut self,
        entity_key: &str,
        id: Uuid,
        json: serde_json::Value,
    ) -> Result<(), BackendError> {
        match entity_key {
            "sequences" => {
                let seq: Sequence = serde_json::from_value(json)?;
                self.state.sequences.insert(id, seq);
            }
            "fixtures" => {
                let f: Fixture = serde_json::from_value(json)?;
                self.state.fixtures.insert(id, f);
            }
            "cues" => {
                let c: Cue = serde_json::from_value(json)?;
                self.state.cues.insert(id, c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Legacy method-string dispatch for backward compat with existing WS Call messages.
    async fn handle_call_legacy(
        &mut self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        match method {
            "sequences.goNext" => {
                let seq_id: Uuid = serde_json::from_value(args["sequenceId"].clone())?;
                let path = vec![
                    PathSegment::Key("sequences".into()),
                    PathSegment::Id(seq_id),
                    PathSegment::Key("goNext".into()),
                ];
                self.apply_set(path.clone(), args.clone(), Lifecycle::Synced).await?;
                if let Some(sync) = &self.sync {
                    sync.broadcast_synced(path, args, self.clock.clone()).await;
                }
                Ok(serde_json::Value::Null)
            }
            "sequences.goToCue" => {
                let seq_id: Uuid = serde_json::from_value(args["sequenceId"].clone())?;
                let path = vec![
                    PathSegment::Key("sequences".into()),
                    PathSegment::Id(seq_id),
                    PathSegment::Key("goToCue".into()),
                ];
                self.apply_set(path.clone(), args.clone(), Lifecycle::Synced).await?;
                if let Some(sync) = &self.sync {
                    sync.broadcast_synced(path, args, self.clock.clone()).await;
                }
                Ok(serde_json::Value::Null)
            }
            _ => Err(BackendError::PathNotFound(vec![PathSegment::Key(method.into())])),
        }
    }

    async fn apply_peer_operation(&mut self, op: Operation) {
        self.clock.merge(&op.clock);
        if self.apply_set(op.path.clone(), op.value.clone(), op.lifecycle).await.is_ok() {
            self.broadcast_after_set(&op.path, op.value);
        } else {
            warn!("failed to apply peer operation: {:?}", op.path);
        }
    }

    /// Broadcast the right value for a completed set operation.
    /// __create/__delete paths broadcast the updated parent collection so frontends
    /// subscribed to e.g. "cues" see the change without re-fetching.
    /// All other paths broadcast the path/value pair as-is.
    fn broadcast_after_set(&self, path: &Path, value: serde_json::Value) {
        let collection_key = match path.as_slice() {
            [PathSegment::Key(k), PathSegment::Key(a)] if a == "__create" => Some(k.as_str()),
            [PathSegment::Key(k), PathSegment::Id(_), PathSegment::Key(a)] if a == "__delete" => Some(k.as_str()),
            _ => None,
        };
        if let Some(key) = collection_key {
            let col_path = vec![PathSegment::Key(key.into())];
            if let Some(col_val) = self.state.get_by_path(&col_path) {
                let _ = self.broadcast.0.send((col_path, col_val));
            }
        } else {
            let _ = self.broadcast.0.send((path.clone(), value));
        }
    }

    /// Load the showfile into memory.
    /// Driven by the EntityMeta registry — no entity types enumerated here.
    async fn load_from_showfile(&mut self) {
        let mut state_map = serde_json::Map::new();
        for meta in inventory::iter::<pult_schema::registry::EntityMeta>() {
            let (Some(table), Some(load)) = (meta.table_name, meta.load_all) else { continue };
            match load(self.pool.as_ref().clone()).await {
                Ok(val) => { state_map.insert(table.to_string(), val); }
                Err(e) => warn!("[engine] load_from_showfile failed for {table}: {e}"),
            }
        }
        match serde_json::from_value::<ShowState>(serde_json::Value::Object(state_map)) {
            Ok(mut state) => {
                state.post_load_init();
                self.state = state;
            }
            Err(e) => warn!("[engine] load_from_showfile: ShowState deserialization failed: {e}"),
        }
    }

    fn build_snapshot(&self) -> serde_json::Value {
        serde_json::to_value(&self.state).unwrap_or_default()
    }

    async fn apply_snapshot(&mut self, data: serde_json::Value) {
        if let Ok(mut state) = serde_json::from_value::<ShowState>(data) {
            state.post_load_init();
            self.state = state;
        }
        self.save_to_showfile().await;
        for key in ShowState::FRONTEND_PATHS {
            let path = vec![PathSegment::Key((*key).into())];
            if let Some(val) = self.state.get_by_path(&path) {
                let _ = self.broadcast.0.send((path, val));
            }
        }
    }

    /// Persist the entire in-memory state to the local showfile.
    /// Driven by the EntityMeta registry — no entity types enumerated here.
    async fn save_to_showfile(&self) {
        let snapshot = serde_json::to_value(&self.state).unwrap_or_default();
        for meta in inventory::iter::<pult_schema::registry::EntityMeta>() {
            if let Some(save) = meta.save_all {
                if let Err(e) = save(self.pool.as_ref().clone(), snapshot.clone()).await {
                    warn!("[engine] save_to_showfile failed for {}: {e}", meta.entity_name);
                }
            }
        }
    }
}

fn apply_field_patch<T: serde::Serialize + serde::de::DeserializeOwned>(
    entity: &mut T,
    field: &str,
    value: serde_json::Value,
) -> Result<(), BackendError> {
    let mut map = serde_json::to_value(&entity)?
        .as_object()
        .cloned()
        .unwrap_or_default();
    map.insert(field.to_owned(), value);
    *entity = serde_json::from_value(serde_json::Value::Object(map))?;
    Ok(())
}

#[cfg(test)]
mod tests;
