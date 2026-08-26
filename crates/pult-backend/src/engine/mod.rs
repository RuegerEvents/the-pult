use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::stream::BoxStream;
use pult_schema::{
    commands::CommandRegistration,
    events::operation::{NodeId, Operation, VectorClock},
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    registry::EntityMeta,
    types::{devices::DevicesState, session::SessionState},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    error::BackendError,
    infra::{connectors::OutputHandle, showfile::{oplog, order}, sync::SyncHandle},
    model::playback::{Playback, PlaybackEffect, ShowView, TICK},
    model::triggers::{InputEvent, TriggerEffect, Triggers},
};

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
#[derive(Debug, Serialize, Deserialize)]
pub struct ShowState {
    #[serde(flatten)]
    entities: serde_json::Map<String, serde_json::Value>,

    /// Display order per collection, keyed by table name. `post_load_init` fills in
    /// any collection that arrives without one.
    #[serde(default)]
    order: BTreeMap<String, Vec<Uuid>>,

    /// LOCAL lifecycle, keyed by top-level path: broadcast to frontends, never
    /// persisted and never sent to peers, so it is skipped by serde and carried
    /// across snapshot application by hand. One manager owns each key.
    #[serde(skip, default = "seed_local")]
    local: BTreeMap<String, serde_json::Value>,
}

/// The LOCAL top-level paths and what an empty one looks like.
///
/// Seeding matters: a frontend that asks for `devices` before the device manager
/// has said anything must get an empty state rather than a path error, and a
/// snapshot from a leader must not leave this node's own view blank.
const LOCAL_STATE: &[(&str, fn() -> serde_json::Value)] = &[
    ("session", || serde_json::to_value(SessionState::default()).unwrap_or_default()),
    ("devices", || serde_json::to_value(DevicesState::default()).unwrap_or_default()),
];

fn seed_local() -> BTreeMap<String, serde_json::Value> {
    LOCAL_STATE.iter().map(|(key, empty)| ((*key).to_string(), empty())).collect()
}

impl Default for ShowState {
    fn default() -> Self {
        Self {
            entities: Default::default(),
            order: Default::default(),
            local: seed_local(),
        }
    }
}

impl ShowState {
    /// Top-level keys broadcast to frontends: every registered table, plus the
    /// LOCAL paths this node maintains for itself.
    pub fn frontend_paths() -> Vec<String> {
        let mut paths: Vec<String> =
            EntityMeta::all_with_tables().iter().filter_map(|m| m.table_name.map(String::from)).collect();
        paths.extend(LOCAL_STATE.iter().map(|(key, _)| (*key).to_string()));
        paths
    }

    /// Is this node following someone else's session? Read straight out of the LOCAL
    /// session state, which SessionManager keeps current.
    fn is_follower(&self) -> bool {
        self.local
            .get("session")
            .and_then(|v| v.get("is_follower"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Reconcile ordering against the entities actually present, after a bulk load
    /// from the showfile or a peer snapshot.
    ///
    /// Known ids keep their position. Anything the order does not mention is appended,
    /// and anything it mentions that is no longer there is dropped, so a stale order
    /// row cannot resurrect a deleted entity or hide a new one.
    pub fn post_load_init(&mut self) {
        for meta in EntityMeta::all_with_tables() {
            let Some(table) = meta.table_name else { continue };
            if meta.is_singleton {
                continue;
            }
            let present: Vec<Uuid> = self
                .entities
                .get(table)
                .and_then(|v| v.as_object())
                .map(|m| m.keys().filter_map(|k| Uuid::parse_str(k).ok()).collect())
                .unwrap_or_default();

            let known = self.order.entry(table.to_string()).or_default();
            known.retain(|id| present.contains(id));
            for id in present {
                if !known.contains(&id) {
                    known.push(id);
                }
            }
        }
    }

    /// Seed ordering from the showfile before entities are reconciled against it.
    pub fn set_order(&mut self, order: std::collections::HashMap<String, Vec<Uuid>>) {
        self.order = order.into_iter().collect();
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

        // LOCAL paths are not registered entities, so they are matched first.
        if let Some(value) = self.local.get(head) {
            return descend(value, rest);
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

#[allow(dead_code, reason = "Subscribe and Stop are protocol surface with no caller yet")]
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
    /// Merge one key into a fixture's live values.
    ///
    /// A device writing a sensor reading cannot read-modify-write from outside the
    /// actor: two ports reporting in the same millisecond would each write back a
    /// map missing the other's key. Merging inside the actor is the whole point.
    SetLiveValue {
        fixture_id: Uuid,
        key: String,
        value: serde_json::Value,
        reply: oneshot::Sender<Result<(), BackendError>>,
    },
    ApplyPeerOperation(Operation),
    /// Apply a batch of operations a peer sent to catch this node up. Ordered oldest
    /// first, so replaying them in sequence lands on the same state the peer has.
    ApplyOperationBatch(Vec<Operation>),
    /// This node's vector clock, so a peer can work out what it has not seen.
    GetClock { reply: oneshot::Sender<VectorClock> },
    /// Operations the holder of this clock is missing, and whether a snapshot would
    /// be cheaper than sending them.
    GetOperationsSince {
        known: VectorClock,
        reply: oneshot::Sender<Option<Vec<Operation>>>,
    },
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

    #[allow(dead_code, reason = "used by EngineDataHandle and the tests, not from main")]
    pub async fn subscribe_pattern(&self, pattern: PathPattern) -> BoxStream<'static, serde_json::Value> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(EngineCommand::Subscribe { pattern, reply: tx }).await;
        rx.await.unwrap_or_else(|_| Box::pin(futures::stream::empty()))
    }

    /// Merge one key into a fixture's live values, replicating the result.
    ///
    /// Unlike a playback effect, which every node derives for itself from cue state,
    /// an input only exists on the node the device is talking to. It has to be sent.
    pub async fn set_live_value(
        &self,
        fixture_id: Uuid,
        key: String,
        value: serde_json::Value,
    ) -> Result<(), BackendError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(EngineCommand::SetLiveValue { fixture_id, key, value, reply: tx })
            .await
            .map_err(|_| BackendError::ChannelClosed)?;
        rx.await?
    }

    pub async fn get_clock(&self) -> VectorClock {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(EngineCommand::GetClock { reply: tx }).await;
        rx.await.unwrap_or_default()
    }

    /// Operations the holder of `known` is missing, or None when a full snapshot
    /// would be the cheaper way to catch them up.
    pub async fn operations_since(&self, known: VectorClock) -> Option<Vec<Operation>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(EngineCommand::GetOperationsSince { known, reply: tx }).await;
        rx.await.ok().flatten()
    }

    pub async fn apply_operation_batch(&self, operations: Vec<Operation>) {
        let _ = self.0.send(EngineCommand::ApplyOperationBatch(operations)).await;
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
    playback: Playback,
    triggers: Triggers,
    /// Parameter changes since the last trigger tick.
    ///
    /// Queued rather than read from the state on the tick, because a button pressed
    /// and released between two ticks would otherwise look like nothing happening.
    input_events: Vec<InputEvent>,
    output: Option<OutputHandle>,
    /// The clock and author of the last accepted write at each replicated path.
    /// Only replicated paths are tracked, so playback output does not grow this.
    path_clocks: HashMap<Path, (VectorClock, NodeId)>,
    /// Bumped by anything that changes the show, so an idle playback can skip the tick
    /// instead of deserializing the whole state 40 times a second for nothing.
    state_version: u64,
    playback_seen: u64,
}

impl ShowEngine {
    /// Build an engine that owns its command channel. `main` uses `new_with_rx` so it
    /// can hand out the handle before the engine exists; the tests use this.
    #[allow(dead_code, reason = "used by the tests, not from main")]
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
            playback: Playback::default(),
            triggers: Triggers::default(),
            input_events: Vec::new(),
            output: None,
            path_clocks: HashMap::new(),
            state_version: 0,
            playback_seen: 0,
        };
        (engine, broadcast)
    }

    /// Attach an output plugin manager. Call before `run`.
    pub fn set_output(&mut self, output: OutputHandle) {
        self.output = Some(output);
    }

    pub async fn run(mut self) {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let cmd = tokio::select! {
                cmd = self.rx.recv() => cmd,
                _ = ticker.tick() => {
                    self.playback_tick().await;
                    self.triggers_tick().await;
                    continue;
                }
            };
            let Some(cmd) = cmd else { break };
            match cmd {
                EngineCommand::Stop => break,
                EngineCommand::LoadFromShowfile => {
                    self.load_from_showfile().await;
                    self.state_version += 1;
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
                        self.state_version += 1;
                        self.record_write(&path, lifecycle);
                        self.log_local_write(&path, &value, lifecycle).await;
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
                    self.state_version += 1;
                    let _ = reply.send(result);
                }
                EngineCommand::SetLiveValue { fixture_id, key, value, reply } => {
                    let result = self.set_live_value(fixture_id, key, value).await;
                    if result.is_ok() {
                        self.state_version += 1;
                    }
                    let _ = reply.send(result);
                }
                EngineCommand::ApplyPeerOperation(op) => {
                    self.apply_peer_operation(op).await;
                    self.state_version += 1;
                }
                EngineCommand::ApplyOperationBatch(operations) => {
                    let count = operations.len();
                    for op in operations {
                        self.apply_peer_operation(op).await;
                    }
                    self.state_version += 1;
                    debug!("[sync] caught up on {count} operations");
                }
                EngineCommand::GetClock { reply } => {
                    let _ = reply.send(self.clock.clone());
                }
                EngineCommand::GetOperationsSince { known, reply } => {
                    let _ = reply.send(self.operations_since(&known).await);
                }
                EngineCommand::GetSnapshot { reply } => {
                    let snapshot = self.build_snapshot();
                    let _ = reply.send(snapshot);
                }
                EngineCommand::ApplyStateSnapshot(snapshot) => {
                    self.apply_snapshot(snapshot).await;
                    self.state_version += 1;
                }
            }
        }
    }

    // ── Playback ──────────────────────────────────────────────────────────────

    /// Advance cue playback one tick and apply what it asks for.
    ///
    /// Effects land with LOCAL lifecycle. Live values and active-cue flags are derived
    /// from cue state, which is already replicated, so every node computes the same
    /// output for itself rather than each fanning its copy out to all the others.
    async fn playback_tick(&mut self) {
        if !self.playback.has_work() && self.state_version == self.playback_seen {
            return;
        }
        self.playback_seen = self.state_version;

        let sequences: Vec<pult_schema::types::sequence::Sequence> = self.read_collection("sequences");
        let cues: Vec<pult_schema::types::cue::Cue> = self.read_collection("cues");
        let fixtures: Vec<pult_schema::types::fixture::Fixture> = self.read_collection("fixtures");

        let effects = {
            let view = ShowView::new(&sequences, &cues, &fixtures);
            self.playback.tick(tokio::time::Instant::now().into_std(), &view)
        };

        // A follower takes its cue positions from the leader, so only the leader
        // fires follow cues. Both ends still run their own fades.
        let is_follower = self.state.is_follower();
        let mut moved: Vec<Uuid> = Vec::new();

        for effect in effects {
            match effect {
                PlaybackEffect::SetLiveValues { fixture_id, values } => {
                    let path = entity_field_path("fixtures", fixture_id, "live_values");
                    self.apply_local(path, serde_json::to_value(values).unwrap_or_default()).await;
                    moved.push(fixture_id);
                }
                PlaybackEffect::SetCueActive { cue_id, is_active } => {
                    let path = entity_field_path("cues", cue_id, "is_active");
                    self.apply_local(path, serde_json::Value::Bool(is_active)).await;
                }
                PlaybackEffect::GoNext { sequence_id } => {
                    if is_follower {
                        continue;
                    }
                    let path = entity_field_path("sequences", sequence_id, "goNext");
                    self.run_synced_command(path, serde_json::json!({})).await;
                }
            }
        }

        self.push_output(moved).await;
    }

    /// Run a registered command and replicate the result.
    ///
    /// The engine's own way of pressing Go: everything a `Call` from a frontend does,
    /// minus the reply. Shared by follow cues and by triggers, so the two cannot
    /// drift into replicating differently.
    async fn run_synced_command(&mut self, path: Path, args: serde_json::Value) {
        if self.apply_set(path.clone(), args.clone(), Lifecycle::Synced).await.is_ok() {
            self.state_version += 1;
            self.record_write(&path, Lifecycle::Synced);
            if let Some(sync) = &self.sync {
                sync.broadcast_synced(path, args, self.clock.clone()).await;
            }
        }
    }

    // ── Triggers ──────────────────────────────────────────────────────────────

    /// Evaluate the trigger rules against whatever came in since the last tick.
    ///
    /// Only the leader fires. A trigger's action is a write to replicated state, so
    /// every node running the same rule would apply the same change several times —
    /// and the input that caused it only ever reached one node anyway.
    async fn triggers_tick(&mut self) {
        let inputs = std::mem::take(&mut self.input_events);
        if inputs.is_empty() && !self.triggers.has_work() {
            return;
        }
        if self.state.is_follower() {
            return;
        }

        let triggers: Vec<pult_schema::types::trigger::Trigger> = self.read_collection("triggers");
        if triggers.is_empty() {
            return;
        }
        let effects =
            self.triggers.tick(tokio::time::Instant::now().into_std(), &triggers, &inputs);

        for effect in effects {
            match effect {
                TriggerEffect::SetPending { trigger_id, pending } => {
                    let path = entity_field_path("triggers", trigger_id, "pending");
                    let value = serde_json::Value::Bool(pending);
                    self.write_synced(path, value).await;
                }
                TriggerEffect::Fire { trigger_id, action } => {
                    self.run_trigger_action(action).await;
                    let path = entity_field_path("triggers", trigger_id, "last_fired_at");
                    let value = serde_json::to_value(chrono::Utc::now()).unwrap_or_default();
                    self.write_synced(path, value).await;
                }
            }
        }
    }

    async fn run_trigger_action(&mut self, action: pult_schema::types::trigger::TriggerAction) {
        use pult_schema::types::trigger::TriggerAction;
        match action {
            TriggerAction::GoNext { sequence_id } => {
                let path = entity_field_path("sequences", sequence_id, "goNext");
                self.run_synced_command(path, serde_json::json!({})).await;
            }
            TriggerAction::GoToCue { sequence_id, cue_id } => {
                let path = entity_field_path("sequences", sequence_id, "goToCue");
                self.run_synced_command(path, serde_json::json!({ "cueId": cue_id })).await;
            }
            TriggerAction::SetParameter { fixture_id, parameter, value } => {
                let key = crate::model::playback::parameter_key(&parameter);
                let value = serde_json::to_value(value).unwrap_or_default();
                // A running fade writing the same key will win the next tick. Last
                // writer takes it, which is a design question and not a bug to fix
                // in passing.
                if let Err(e) = self.set_live_value(fixture_id, key, value).await {
                    debug!("[triggers] set parameter on {fixture_id}: {e}");
                }
            }
        }
    }

    /// Write one replicated field and tell everyone who needs to know.
    async fn write_synced(&mut self, path: Path, value: serde_json::Value) {
        if self.apply_set(path.clone(), value.clone(), Lifecycle::Synced).await.is_err() {
            return;
        }
        self.state_version += 1;
        self.record_write(&path, Lifecycle::Synced);
        self.log_local_write(&path, &value, Lifecycle::Synced).await;
        self.broadcast_after_set(&path, value.clone());
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, value, self.clock.clone()).await;
        }
    }

    /// Hand the current patch to the output plugins.
    ///
    /// Sent on every tick that did any work, including a patch edit that moved no
    /// light, because a re-addressed fixture changes the wire without changing a
    /// single level. Plugins decide for themselves what is worth transmitting.
    async fn push_output(&mut self, moved: Vec<Uuid>) {
        let Some(output) = &self.output else { return };
        let fixtures: Vec<pult_schema::types::fixture::Fixture> = self.read_collection("fixtures");
        if fixtures.is_empty() {
            return;
        }
        let fixture_types = self.read_collection("fixture_types");
        output.push(fixtures, fixture_types, moved);
    }

    /// Merge one key into a fixture's live values and replicate the whole map.
    ///
    /// SYNCED rather than LOCAL: nothing else on the network can work this value out
    /// for itself, because it came off a wire attached to this node.
    async fn set_live_value(
        &mut self,
        fixture_id: Uuid,
        key: String,
        value: serde_json::Value,
    ) -> Result<(), BackendError> {
        let path = entity_field_path("fixtures", fixture_id, "live_values");
        let mut values = self
            .state
            .entity("fixtures", fixture_id)
            .and_then(|entity| entity.get("live_values"))
            .cloned()
            .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
        let previous = values.get(&key).cloned();
        set_field(&mut values, &key, value.clone());

        // Queued for the next trigger tick rather than read back from the state
        // there, so a press and a release between two ticks are both seen.
        if let Ok(current) = serde_json::from_value(value) {
            self.input_events.push(InputEvent {
                fixture_id,
                key: key.clone(),
                previous: previous.and_then(|p| serde_json::from_value(p).ok()),
                current,
            });
        }

        self.apply_set(path.clone(), values.clone(), Lifecycle::Synced).await?;
        self.record_write(&path, Lifecycle::Synced);
        self.log_local_write(&path, &values, Lifecycle::Synced).await;
        self.broadcast_after_set(&path, values.clone());
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, values, self.clock.clone()).await;
        }
        // The output side has to see it now: an input can arrive between two ticks,
        // and a relay that follows a button should not wait for the next one.
        self.push_output(vec![fixture_id]).await;
        Ok(())
    }

    /// Log a write this node made itself.
    async fn log_local_write(
        &self,
        path: &Path,
        value: &serde_json::Value,
        lifecycle: Lifecycle,
    ) {
        if lifecycle == Lifecycle::Local {
            return;
        }
        let op = Operation {
            id: Uuid::new_v4(),
            node_id: self.node_id,
            seq: self.clock.0.get(&self.node_id).copied().unwrap_or(0),
            clock: self.clock.clone(),
            lifecycle,
            path: path.clone(),
            value: value.clone(),
            timestamp: chrono::Utc::now(),
        };
        self.log_operation(&op).await;
    }

    /// Read one collection out of the state as typed entities.
    fn read_collection<T: serde::de::DeserializeOwned>(&self, table: &str) -> Vec<T> {
        self.state
            .get_by_path(&vec![PathSegment::Key(table.into())])
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Apply a write and tell the frontends, without sending it to peers.
    async fn apply_local(&mut self, path: Path, value: serde_json::Value) {
        match self.apply_set(path.clone(), value.clone(), Lifecycle::Local).await {
            Ok(()) => self.broadcast_after_set(&path, value),
            Err(e) => debug!("[playback] {path:?}: {e}"),
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

        // LOCAL paths are replaced whole by the manager that owns them, and are not
        // registered entities. Nothing writes a field of one from the outside.
        if self.state.local.contains_key(head) {
            return match rest {
                [] => {
                    self.state.local.insert(head.clone(), value);
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
        self.persist_order(meta, table).await;
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
        self.persist_order(meta, table).await;
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

    /// Write a collection's display order to the showfile.
    ///
    /// Order is show data, so it goes to disk alongside the entities. A failure here
    /// is logged rather than failing the write: losing the order of a list is not a
    /// reason to reject the fixture that was just patched.
    async fn persist_order(&self, meta: &'static EntityMeta, table: &str) {
        if meta.is_singleton || meta.upsert_one.is_none() {
            return;
        }
        if let Err(e) = order::save(&self.pool, table, self.state.ids(table)).await {
            warn!("[engine] could not save {table} order: {e}");
        }
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
        self.record_write(&path, Lifecycle::Synced);
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, args, self.clock.clone()).await;
        }
        Ok(serde_json::Value::Null)
    }

    async fn apply_peer_operation(&mut self, op: Operation) {
        // Merge regardless of whether the operation wins: we have still learned what
        // the sending node knew, and our next write has to be causally after it.
        self.clock.merge(&op.clock);

        if !self.accepts(&op.path, &op.clock, op.node_id) {
            debug!("[sync] dropping superseded write to {:?} from {}", op.path, op.node_id);
            return;
        }
        if self.apply_set(op.path.clone(), op.value.clone(), op.lifecycle).await.is_ok() {
            self.log_operation(&op).await;
            self.path_clocks.insert(op.path.clone(), (op.clock, op.node_id));
            self.broadcast_after_set(&op.path, op.value);
        } else {
            warn!("failed to apply peer operation: {:?}", op.path);
        }
    }

    /// Operations the holder of `known` has not seen, or None when replaying them
    /// would cost more than sending the whole show.
    ///
    /// A node that has never heard from anyone has an empty clock and would be sent
    /// every operation ever recorded, which is strictly worse than a snapshot.
    async fn operations_since(&self, known: &VectorClock) -> Option<Vec<Operation>> {
        if known.0.is_empty() {
            return None;
        }
        let missing = oplog::since(&self.pool, known).await.ok()?;
        let total = oplog::len(&self.pool).await.unwrap_or(0);
        // Replaying most of the log is not catch-up, it is a slow snapshot.
        if total > 0 && missing.len() as u64 * 2 > total {
            return None;
        }
        Some(missing)
    }

    /// Should an incoming write replace what this node already has at that path?
    ///
    /// Vector clocks order writes that are causally related. Writes that are not
    /// related are genuinely simultaneous, and something has to break the tie the
    /// same way on every node or the show ends up different on each of them. The
    /// higher node id wins: arbitrary, but identical everywhere.
    fn accepts(&self, path: &Path, clock: &VectorClock, node: NodeId) -> bool {
        let Some((known_clock, known_node)) = self.path_clocks.get(path) else {
            return true;
        };
        if known_clock.happens_before(clock) {
            true
        } else if clock.happens_before(known_clock) {
            false
        } else {
            node > *known_node
        }
    }

    /// Note that this node wrote a replicated path, so a peer's concurrent write to
    /// the same path can be ordered against it.
    fn record_write(&mut self, path: &Path, lifecycle: Lifecycle) {
        if lifecycle == Lifecycle::Local {
            return;
        }
        self.path_clocks.insert(path.clone(), (self.clock.clone(), self.node_id));
    }

    /// Note a replicated write in the operation log, so a peer that reconnects can be
    /// told what it missed. Local writes never leave this node and are not logged.
    async fn log_operation(&self, op: &Operation) {
        if op.lifecycle == Lifecycle::Local {
            return;
        }
        if let Err(e) = oplog::append(&self.pool, op).await {
            warn!("[engine] could not write to the oplog: {e}");
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
        let saved_order = match order::load_all(&self.pool).await {
            Ok(order) => order,
            Err(e) => {
                warn!("[engine] could not load collection order: {e}");
                Default::default()
            }
        };
        match serde_json::from_value::<ShowState>(serde_json::Value::Object(state_map)) {
            Ok(mut state) => {
                state.set_order(saved_order);
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
            // LOCAL state belongs to this node: its session, and what it can see on
            // its own network segment. A leader's copy of either means nothing here.
            state.local = std::mem::take(&mut self.state.local);
            self.state = state;
            // The snapshot replaces every value, so what we knew about individual
            // paths no longer describes anything.
            self.path_clocks.clear();
        }
        self.save_to_showfile().await;
        for meta in EntityMeta::all_with_tables() {
            if let Some(table) = meta.table_name {
                self.persist_order(meta, table).await;
            }
        }
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

/// `[table, id, field]`.
fn entity_field_path(table: &str, id: Uuid, field: &str) -> Path {
    vec![
        PathSegment::Key(table.into()),
        PathSegment::Id(id),
        PathSegment::Key(field.into()),
    ]
}

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
