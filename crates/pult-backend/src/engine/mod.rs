use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::stream::BoxStream;
use pult_schema::{
    commands::CommandRegistration,
    events::operation::{NodeId, Operation, VectorClock},
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    registry::EntityMeta,
    types::session::SessionState,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{error::BackendError, infra::sync::SyncHandle};

// ── In-memory show state ──────────────────────────────────────────────────────

/// The whole show, held as JSON keyed by entity table.
///
/// No entity type is named in this file. Every collection comes from the
/// `EntityMeta` registry, so adding a `#[derive(PultSchema)]` type with a table
/// makes it readable, writable, persisted, synced, and visible to the frontend
/// with no change here.
///
/// Shape (identical to what `EntityMeta::load_all` produces and `save_all` reads):
/// collections are objects keyed by entity id, singletons hold the entity or null.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ShowState {
    #[serde(flatten)]
    entities: serde_json::Map<String, serde_json::Value>,

    /// Display order per collection, keyed by table name. `post_load_init` fills in
    /// any collection that arrives without one.
    #[serde(default)]
    order: BTreeMap<String, Vec<Uuid>>,

    /// LOCAL lifecycle: broadcast to frontends, never persisted and never sent to
    /// peers, so it is skipped by serde and preserved across snapshot application.
    /// SessionManager is the only writer.
    #[serde(skip)]
    pub session: SessionState,
}

impl ShowState {
    /// Top-level keys broadcast to frontends: every registered table, plus session.
    pub fn frontend_paths() -> Vec<String> {
        let mut paths: Vec<String> =
            EntityMeta::all_with_tables().iter().filter_map(|m| m.table_name.map(String::from)).collect();
        paths.push("session".into());
        paths
    }

    /// Fill in ordering for any collection that arrived without one, after a bulk
    /// load from the showfile or a peer snapshot.
    pub fn post_load_init(&mut self) {
        for meta in EntityMeta::all_with_tables() {
            let Some(table) = meta.table_name else { continue };
            if meta.is_singleton || self.order.contains_key(table) {
                continue;
            }
            let ids = self
                .entities
                .get(table)
                .and_then(|v| v.as_object())
                .map(|m| m.keys().filter_map(|k| Uuid::parse_str(k).ok()).collect())
                .unwrap_or_default();
            self.order.insert(table.to_string(), ids);
        }
    }

    fn ids(&self, table: &str) -> &[Uuid] {
        self.order.get(table).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Resolve one path segment to an entity id within `table`.
    /// `Id` is taken as-is; `Index` is looked up in the collection's order.
    fn resolve_id(&self, table: &str, seg: &PathSegment) -> Option<Uuid> {
        match seg {
            PathSegment::Id(id) => Some(*id),
            PathSegment::Index(n) => self.ids(table).get(*n).copied(),
            PathSegment::Key(_) => None,
        }
    }

    fn entity(&self, table: &str, id: Uuid) -> Option<&serde_json::Value> {
        self.entities.get(table)?.as_object()?.get(&id.to_string())
    }

    fn insert_entity(&mut self, table: &str, id: Uuid, value: serde_json::Value) {
        let collection = self
            .entities
            .entry(table.to_string())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if !collection.is_object() {
            *collection = serde_json::Value::Object(Default::default());
        }
        let is_new = collection
            .as_object_mut()
            .map(|m| m.insert(id.to_string(), value).is_none())
            .unwrap_or(false);
        if is_new {
            self.order.entry(table.to_string()).or_default().push(id);
        }
    }

    fn remove_entity(&mut self, table: &str, id: Uuid) {
        if let Some(collection) = self.entities.get_mut(table).and_then(|v| v.as_object_mut()) {
            collection.remove(&id.to_string());
        }
        if let Some(order) = self.order.get_mut(table) {
            order.retain(|i| *i != id);
        }
    }

    fn singleton(&self, table: &str) -> Option<&serde_json::Value> {
        self.entities.get(table).filter(|v| !v.is_null())
    }

    /// The whole collection as an array, in display order.
    fn collection_array(&self, table: &str) -> serde_json::Value {
        let values = self
            .ids(table)
            .iter()
            .filter_map(|id| self.entity(table, *id))
            .cloned()
            .collect();
        serde_json::Value::Array(values)
    }

    pub fn get_by_path(&self, path: &Path) -> Option<serde_json::Value> {
        let [PathSegment::Key(head), rest @ ..] = path.as_slice() else { return None };

        // session is LOCAL and not a registered entity, so it is matched first.
        if head == "session" {
            return descend(&serde_json::to_value(&self.session).ok()?, rest);
        }

        let meta = EntityMeta::by_table(head)?;
        if meta.is_singleton {
            return descend(self.singleton(head)?, rest);
        }
        match rest {
            [] => Some(self.collection_array(head)),
            [seg, tail @ ..] => {
                let id = self.resolve_id(head, seg)?;
                descend(self.entity(head, id)?, tail)
            }
        }
    }
}

/// Walk into a JSON value along the remaining path segments.
fn descend(value: &serde_json::Value, rest: &[PathSegment]) -> Option<serde_json::Value> {
    let mut current = value;
    for seg in rest {
        current = match seg {
            PathSegment::Key(k) => current.get(k)?,
            PathSegment::Index(n) => current.get(*n)?,
            PathSegment::Id(id) => current.get(id.to_string())?,
        };
    }
    Some(current.clone())
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

    /// Route a write to the right entity, generically.
    ///
    /// Recognised shapes, for any registered table:
    ///   `[table]`                      replace a singleton
    ///   `[table, field]`               patch a singleton field
    ///   `[table, "__create"]`          add to a collection
    ///   `[table, ref, "__delete"]`     remove from a collection
    ///   `[table, ref]`                 replace one entity
    ///   `[table, ref, command]`        run a registered command
    ///   `[table, ref, field]`          patch one field
    ///
    /// `ref` is either an id or an index into the collection's display order.
    async fn apply_set(
        &mut self,
        path: Path,
        value: serde_json::Value,
        lifecycle: Lifecycle,
    ) -> Result<(), BackendError> {
        self.next_seq += 1;
        self.clock.increment(self.node_id);

        let [PathSegment::Key(head), rest @ ..] = path.as_slice() else {
            debug!("unhandled set path: {path:?}");
            return Err(BackendError::PathNotFound(path));
        };

        // session is LOCAL, written only by SessionManager, and not a registered entity.
        if head == "session" {
            return match rest {
                [] => {
                    self.state.session = serde_json::from_value(value)?;
                    Ok(())
                }
                _ => Err(BackendError::PathNotFound(path.clone())),
            };
        }

        let Some(meta) = EntityMeta::by_table(head) else {
            debug!("unhandled set path: {path:?}");
            return Err(BackendError::PathNotFound(path));
        };
        let table = head.clone();

        if meta.is_singleton {
            return self.set_singleton(meta, &table, rest, value, &path).await;
        }

        match rest {
            [PathSegment::Key(action)] if action == "__create" => {
                self.create_entity(meta, &table, value, &path).await
            }
            [seg, PathSegment::Key(action)] if action == "__delete" => {
                let id = self
                    .state
                    .resolve_id(&table, seg)
                    .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
                self.delete_entity(meta, &table, id).await
            }
            [seg] => {
                let id = self
                    .state
                    .resolve_id(&table, seg)
                    .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
                self.replace_entity(meta, &table, id, value, &path).await
            }
            [seg, PathSegment::Key(name)] => {
                let id = self
                    .state
                    .resolve_id(&table, seg)
                    .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
                if let Some(handler) = self.commands.get(&(table.clone(), name.as_str())).copied() {
                    return self.dispatch_command(handler, meta, &table, id, value).await;
                }
                self.patch_field(meta, &table, id, name, value, lifecycle, &path).await
            }
            _ => {
                debug!("unhandled set path: {path:?}");
                Err(BackendError::PathNotFound(path))
            }
        }
    }

    async fn set_singleton(
        &mut self,
        meta: &'static EntityMeta,
        table: &str,
        rest: &[PathSegment],
        value: serde_json::Value,
        path: &Path,
    ) -> Result<(), BackendError> {
        let next = match rest {
            [] => value,
            [PathSegment::Key(field)] => {
                let mut current = self
                    .state
                    .singleton(table)
                    .cloned()
                    .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
                if meta.field_lifecycle(field).is_none() {
                    return Err(BackendError::PathNotFound(path.clone()));
                }
                set_field(&mut current, field, value);
                current
            }
            _ => return Err(BackendError::PathNotFound(path.clone())),
        };

        let next = validate(meta, next, path)?;
        self.persist(meta, &next).await?;
        self.state.entities.insert(table.to_string(), next);
        Ok(())
    }

    async fn create_entity(
        &mut self,
        meta: &'static EntityMeta,
        table: &str,
        value: serde_json::Value,
        path: &Path,
    ) -> Result<(), BackendError> {
        let entity = validate(meta, value, path)?;
        let id = entity_id(meta, &entity, path)?;
        self.persist(meta, &entity).await?;
        self.state.insert_entity(table, id, entity);
        Ok(())
    }

    async fn replace_entity(
        &mut self,
        meta: &'static EntityMeta,
        table: &str,
        id: Uuid,
        value: serde_json::Value,
        path: &Path,
    ) -> Result<(), BackendError> {
        if self.state.entity(table, id).is_none() {
            return Err(BackendError::PathNotFound(path.clone()));
        }
        let entity = validate(meta, value, path)?;
        self.persist(meta, &entity).await?;
        self.state.insert_entity(table, id, entity);
        Ok(())
    }

    async fn delete_entity(
        &mut self,
        meta: &'static EntityMeta,
        table: &str,
        id: Uuid,
    ) -> Result<(), BackendError> {
        if let Some(delete_one) = meta.delete_one {
            delete_one(self.pool.as_ref().clone(), id).await?;
        }
        self.state.remove_entity(table, id);
        Ok(())
    }

    /// Patch one field of one entity.
    ///
    /// Whether the write reaches SQLite is decided by the field's own lifecycle in the
    /// schema, not by the caller's `lifecycle` argument. The argument is only a fallback
    /// for names the schema does not know.
    async fn patch_field(
        &mut self,
        meta: &'static EntityMeta,
        table: &str,
        id: Uuid,
        field: &str,
        value: serde_json::Value,
        fallback: Lifecycle,
        path: &Path,
    ) -> Result<(), BackendError> {
        let Some(field_lifecycle) = meta.field_lifecycle(field) else {
            debug!("no field or command named {field} on {table}");
            return Err(BackendError::PathNotFound(path.clone()));
        };
        let mut entity = self
            .state
            .entity(table, id)
            .cloned()
            .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;

        set_field(&mut entity, field, value);
        let entity = validate(meta, entity, path)?;

        let _ = fallback;
        if field_lifecycle == Lifecycle::Persisted {
            self.persist(meta, &entity).await?;
        }
        self.state.insert_entity(table, id, entity);
        Ok(())
    }

    async fn persist(
        &self,
        meta: &'static EntityMeta,
        entity: &serde_json::Value,
    ) -> Result<(), BackendError> {
        if let Some(upsert_one) = meta.upsert_one {
            upsert_one(self.pool.as_ref().clone(), entity.clone()).await?;
        }
        Ok(())
    }

    /// Run a registered command against one entity, then store and broadcast the result.
    ///
    /// Commands are not written to SQLite. They move SYNCED playback state, and a
    /// showfile write on every Go press is not something a console should do.
    async fn dispatch_command(
        &mut self,
        handler: CommandHandler,
        meta: &'static EntityMeta,
        table: &str,
        id: Uuid,
        args: serde_json::Value,
    ) -> Result<(), BackendError> {
        let entity_path = vec![PathSegment::Key(table.into()), PathSegment::Id(id)];
        let entity = self
            .state
            .entity(table, id)
            .cloned()
            .ok_or_else(|| BackendError::PathNotFound(entity_path.clone()))?;

        let result = handler(entity, args).map_err(|e| BackendError::InvalidValue {
            path: entity_path.clone(),
            reason: e.to_string(),
        })?;
        let result = validate(meta, result, &entity_path)?;

        self.state.insert_entity(table, id, result.clone());
        let _ = self.broadcast.0.send((entity_path, result));
        Ok(())
    }

    /// Legacy method-string dispatch, kept for the `Call` message in the WS protocol.
    ///
    /// `"<table>.<command>"` with the entity id in `args` under `"<entity>Id"`, so
    /// `"sequences.goNext"` takes `{ "sequenceId": "..." }`. Both halves come from the
    /// registry, so this works for every entity and command without a list here.
    async fn handle_call_legacy(
        &mut self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        let not_found = || BackendError::PathNotFound(vec![PathSegment::Key(method.into())]);

        let (table, command) = method.split_once('.').ok_or_else(not_found)?;
        let meta = EntityMeta::by_table(table).ok_or_else(not_found)?;
        if !self.commands.contains_key(&(table.to_string(), command)) {
            return Err(not_found());
        }

        let id_arg = id_arg_name(meta.entity_name);
        let id: Uuid = serde_json::from_value(args[&id_arg].clone())?;

        let path = vec![
            PathSegment::Key(table.into()),
            PathSegment::Id(id),
            PathSegment::Key(command.into()),
        ];
        self.apply_set(path.clone(), args.clone(), Lifecycle::Synced).await?;
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, args, self.clock.clone()).await;
        }
        Ok(serde_json::Value::Null)
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
            [PathSegment::Key(k), _, PathSegment::Key(a)] if a == "__delete" => Some(k.as_str()),
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
        for meta in EntityMeta::all_with_tables() {
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
            // session is LOCAL: this node's own session survives a leader snapshot.
            state.session = std::mem::take(&mut self.state.session);
            self.state = state;
        }
        self.save_to_showfile().await;
        for key in ShowState::frontend_paths() {
            let path = vec![PathSegment::Key(key)];
            if let Some(val) = self.state.get_by_path(&path) {
                let _ = self.broadcast.0.send((path, val));
            }
        }
    }

    /// Persist the entire in-memory state to the local showfile.
    /// Driven by the EntityMeta registry — no entity types enumerated here.
    async fn save_to_showfile(&self) {
        let snapshot = serde_json::to_value(&self.state).unwrap_or_default();
        for meta in EntityMeta::all_with_tables() {
            if let Some(save) = meta.save_all {
                if let Err(e) = save(self.pool.as_ref().clone(), snapshot.clone()).await {
                    warn!("[engine] save_to_showfile failed for {}: {e}", meta.entity_name);
                }
            }
        }
    }
}

// ── JSON helpers ──────────────────────────────────────────────────────────────

/// Set one field on an entity value, turning a non-object into an object first.
fn set_field(entity: &mut serde_json::Value, field: &str, value: serde_json::Value) {
    if !entity.is_object() {
        *entity = serde_json::Value::Object(Default::default());
    }
    if let Some(map) = entity.as_object_mut() {
        map.insert(field.to_owned(), value);
    }
}

/// Round-trip a value through its concrete Rust type. This is where a bad write is
/// caught: the engine holds JSON, but nothing enters the state without deserializing
/// cleanly into the schema type first.
fn validate(
    meta: &'static EntityMeta,
    value: serde_json::Value,
    path: &Path,
) -> Result<serde_json::Value, BackendError> {
    (meta.validate)(value).map_err(|e| BackendError::InvalidValue {
        path: path.clone(),
        reason: e.to_string(),
    })
}

/// The args key holding an entity id in a legacy Call: `Sequence` becomes `sequenceId`.
fn id_arg_name(entity_name: &str) -> String {
    let mut chars = entity_name.chars();
    let first = chars.next().map(|c| c.to_ascii_lowercase()).unwrap_or_default();
    format!("{first}{}Id", chars.as_str())
}

/// Read an entity's primary key out of its JSON.
fn entity_id(
    meta: &'static EntityMeta,
    entity: &serde_json::Value,
    path: &Path,
) -> Result<Uuid, BackendError> {
    let field = meta.primary_key.ok_or_else(|| BackendError::InvalidValue {
        path: path.clone(),
        reason: format!("{} has no primary key", meta.entity_name),
    })?;
    entity
        .get(field)
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| BackendError::InvalidValue {
            path: path.clone(),
            reason: format!("missing or malformed {field}"),
        })
}

#[cfg(test)]
mod tests;
