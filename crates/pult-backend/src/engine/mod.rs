use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::stream::BoxStream;
use pult_schema::{
    commands::CommandRegistration,
    events::operation::{Authorship, NodeId, Operation, VectorClock},
    types::show::{clamp_history_depth, clamp_home_fade_ms, HISTORY_DEPTH_DEFAULT},
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    registry::EntityMeta,
    types::{
        devices::DevicesState,
        fixture::{
            home_value, output_parameters, Fixture, FixtureType, ParameterDirection,
            ParameterKind, ParameterValue,
        },
        output::{OutputCoverage, OutputStatuses},
        plugin::PluginsState, programmer::programmer_entry_id, session::SessionState,
        station::PeerLinks, user::User,
    },
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    error::BackendError,
    infra::{connectors::OutputHandle, showfile::{oplog, order}, sync::SyncHandle},
    model::playback::{parameter_key, Playback, PlaybackEffect, ShowView, TICK},
    model::flows::{FlowEffect, FlowGraph, Flows, InputEvent},
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
    ("output_status", || serde_json::to_value(OutputStatuses::default()).unwrap_or_default()),
    ("output_coverage", || serde_json::to_value(OutputCoverage::default()).unwrap_or_default()),
    ("peers", || serde_json::to_value(PeerLinks::default()).unwrap_or_default()),
    ("plugins", || serde_json::to_value(PluginsState::default()).unwrap_or_default()),
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

/// A JSON value moved by a delta: a `ParameterValue` if that is what it is, and a
/// plain number otherwise.
///
/// Both, because `__by` is a field verb and a field is as likely to be a cue's fade
/// time as a fixture's intensity. Anything else refuses by name rather than being
/// quietly left alone.
fn nudge_json(current: &serde_json::Value, by: f32) -> Result<serde_json::Value, String> {
    if let Ok(value) = serde_json::from_value::<ParameterValue>(current.clone()) {
        let next = value.nudged(by)?;
        return serde_json::to_value(next).map_err(|e| e.to_string());
    }
    match current.as_f64() {
        // An integer field stays one: `fade_in_ms` is milliseconds, and a write of
        // 4500.0 where a whole number is expected is a failed patch rather than a
        // slightly imprecise one.
        Some(n) if current.is_i64() || current.is_u64() => {
            Ok(serde_json::json!((n + by as f64).round() as i64))
        }
        Some(n) => Ok(serde_json::json!(n + by as f64)),
        None => Err("that field is not a number, so there is nothing to move".into()),
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
        /// Who asked and what they were in the middle of. Empty for the engine\'s
        /// own writes, which nobody asked for and nobody can take back.
        authorship: Authorship,
        reply: oneshot::Sender<Result<(), BackendError>>,
    },
    Get {
        path: Path,
        reply: oneshot::Sender<Result<serde_json::Value, BackendError>>,
    },
    /// Take back this user's last change, or put back their last undo.
    Undo {
        user_id: Uuid,
        redo: bool,
        /// The paths it wrote, or empty when there was nothing to reverse.
        reply: oneshot::Sender<Vec<Path>>,
    },
    /// The recent operation log, for the history panel.
    History { limit: u32, reply: oneshot::Sender<Vec<Operation>> },

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
            .send(EngineCommand::Set {
                path,
                value,
                lifecycle,
                authorship: Authorship::none(),
                reply: tx,
            })
            .await
            .map_err(|_| BackendError::ChannelClosed)?;
        rx.await?
    }

    /// Take back a user's last gesture. `redo` puts back their last undo instead.
    ///
    /// Returns the paths it actually wrote — one for an ordinary change, one per
    /// fixture for a fan across a selection, and empty when there was nothing to
    /// take back. Paths rather than operations because that is what moved: a drag is
    /// four hundred operations and one thing changing.
    pub async fn undo(&self, user_id: Uuid, redo: bool) -> Vec<Path> {
        let (tx, rx) = oneshot::channel();
        if self.0.send(EngineCommand::Undo { user_id, redo, reply: tx }).await.is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// The recent operation log, newest first.
    pub async fn history(&self, limit: u32) -> Vec<Operation> {
        let (tx, rx) = oneshot::channel();
        if self.0.send(EngineCommand::History { limit, reply: tx }).await.is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// A write somebody asked for, so it can be taken back.
    ///
    /// `gesture` is the one act it was part of, where the client said so — a drag,
    /// a fan across a selection — and `None` for a write that stands alone.
    pub async fn set_as(
        &self,
        user_id: Uuid,
        gesture: Option<Uuid>,
        path: Path,
        lifecycle: Lifecycle,
        value: serde_json::Value,
    ) -> Result<(), BackendError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(EngineCommand::Set {
                path,
                value,
                lifecycle,
                // `previous` is filled in by the engine, which is the only place
                // that can read the old value without racing another write.
                authorship: Authorship::by(Some(user_id), None).during(gesture),
                reply: tx,
            })
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
    flows: Flows,
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
    /// The `outputs` collection changed and the output side has not been told yet.
    /// Set on the first tick too, so a saved show comes up sending.
    outputs_dirty: bool,
    /// Whether the plugins have been handed a patch with something in it. An
    /// empty show pushes nothing — except once, when the last fixture goes, so
    /// what the plugins know does not outlive it.
    pushed_fixtures: bool,
    /// Something in a graph changed, so the next tick has to look at it.
    flows_dirty: bool,
    /// The fixture parameters some *Watch* node is looking at, so a fade can be
    /// offered to the flow tick without walking every graph on every frame.
    watched: std::collections::HashSet<(Uuid, String)>,
    /// Appends since the log was last cut back.
    ///
    /// In memory rather than on disk: a station restarted often prunes at open and
    /// rarely otherwise, which is the right amount for a station restarted often. The
    /// case this counter is for is the one that never restarts.
    ///
    /// Atomic because the append path takes `&self` — a write is logged from the
    /// same borrow that broadcasts it.
    appends_since_prune: std::sync::atomic::AtomicU32,
    /// Whether a prune is already running, so a second does not start beside it.
    ///
    /// Two concurrent deletes racing on the floor is the one way to get its ordering
    /// wrong, and at a threshold of a thousand appends against a delete that takes
    /// seconds, an overlap is reachable rather than theoretical.
    pruning: Arc<std::sync::atomic::AtomicBool>,
}

/// How many appends between prunes.
///
/// Large enough that pruning is rare against the write rate, small enough that a
/// station left up for a fortnight is bounded while it runs rather than only when it
/// is next opened — which is the case that motivated this at all.
const APPENDS_BETWEEN_PRUNES: u32 = 1_000;

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
            flows: Flows::default(),
            input_events: Vec::new(),
            output: None,
            path_clocks: HashMap::new(),
            state_version: 0,
            playback_seen: 0,
            outputs_dirty: true,
            pushed_fixtures: false,
            flows_dirty: true,
            watched: Default::default(),
            appends_since_prune: Default::default(),
            pruning: Default::default(),
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
                    self.push_output_config().await;
                    self.playback_tick().await;
                    self.flows_tick().await;
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
                EngineCommand::Set { path, value, lifecycle, authorship, reply } => {
                    // A write that says how far, or where something rests, becomes an
                    // ordinary absolute write *here*, before anything below has seen
                    // it. Everything after this line — the read of `previous`, the
                    // apply, the oplog, the broadcast, the sync — is code that has
                    // never heard of `__by` or `__home`, and a peer receives the
                    // number rather than the verb. Which is the whole point: two
                    // stations each adding ten percent to whatever they happened to be
                    // showing would not end up holding the same value.
                    let resolved = match self.resolve_verbs(path, value) {
                        Ok(resolved) => resolved,
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            continue;
                        }
                    };
                    // One verb can be several writes — sending a fixture home is one
                    // per parameter — and an operator who asked for one thing should
                    // get one thing back from Ctrl-Z. So they share a gesture, which
                    // is what undo already groups by.
                    let authorship = match (resolved.len(), authorship.user_id, authorship.gesture) {
                        (n, Some(_), None) if n > 1 => authorship.during(Some(Uuid::new_v4())),
                        _ => authorship,
                    };
                    let mut result = Ok(());
                    for (path, value) in resolved {
                        let mut authorship = authorship.clone();
                        // Read before writing: the oplog is otherwise a list of
                        // destinations with no record of where anything came from, and
                        // replaying that forwards works while running it backwards
                        // does not. Only for a write somebody asked for — the engine's
                        // own, at 40 Hz, would pay for a read nobody will ever undo.
                        authorship.previous =
                            authorship.user_id.map(|_| self.value_before(&path)).unwrap_or(None);
                        result = self.apply_set(path.clone(), value.clone(), lifecycle).await;
                        if result.is_err() {
                            break;
                        }
                        self.state_version += 1;
                        self.record_write(&path, lifecycle);
                        self.log_local_write(&path, &value, lifecycle, &authorship).await;
                        self.broadcast_after_set(&path, value.clone());
                        if lifecycle != Lifecycle::Local {
                            if let Some(sync) = &self.sync {
                                sync.broadcast_synced(path, value, self.clock.clone(), authorship)
                                    .await;
                            }
                        }
                    }
                    let _ = reply.send(result);
                }
                EngineCommand::Undo { user_id, redo, reply } => {
                    let done = self.take_back(user_id, redo).await;
                    let _ = reply.send(done);
                }
                EngineCommand::History { limit, reply } => {
                    // Never further back than the show keeps, whatever was asked for:
                    // a history panel showing changes that Ctrl-Z can no longer reach
                    // is offering to undo things it cannot.
                    let limit = limit.min(self.history_depth());
                    let log = oplog::recent_by_people(&self.pool, limit).await.unwrap_or_default();
                    let _ = reply.send(log);
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
        // For one question — where a parameter rests when nothing is driving it. A
        // handful of rows beside thousands of fixtures.
        let fixture_types: Vec<FixtureType> = self.read_collection("fixture_types");
        let programmer: Vec<pult_schema::types::programmer::ProgrammerValue> =
            self.read_collection("programmer_values");
        let masters: Vec<pult_schema::types::speedmaster::SpeedMaster> =
            self.read_collection("speed_masters");
        let home_fade_ms = self.home_fade_ms();

        // Read once per tick rather than per effect: every station has to place this
        // tick at one instant, and asking the clock twice inside a tick would put two
        // fixtures on the same cue a fraction of a cycle apart.
        let wall_ms = pult_schema::types::sequence::now_ms();

        let effects = {
            let view = ShowView::new(
                &sequences,
                &cues,
                &fixtures,
                &fixture_types,
                &programmer,
                &masters,
                home_fade_ms,
            );
            self.playback.tick(tokio::time::Instant::now().into_std(), wall_ms, &view)
        };

        // A follower takes its cue positions from the leader, so only the leader
        // fires follow cues. Both ends still run their own fades.
        let is_follower = self.state.is_follower();
        let mut moved: Vec<Uuid> = Vec::new();

        for effect in effects {
            match effect {
                PlaybackEffect::SetLiveValues { fixture_id, values } => {
                    self.queue_watched_changes(fixture_id, &values);
                    let path = entity_field_path("fixtures", fixture_id, "live_values");
                    self.apply_local(path, serde_json::to_value(values).unwrap_or_default()).await;
                    moved.push(fixture_id);
                }
                PlaybackEffect::SetLiveEffects { fixture_id, effects } => {
                    let path = entity_field_path("fixtures", fixture_id, "live_effects");
                    self.apply_local(path, serde_json::to_value(effects).unwrap_or_default())
                        .await;
                    moved.push(fixture_id);
                }
                PlaybackEffect::SetLiveFades { fixture_id, fades } => {
                    let path = entity_field_path("fixtures", fixture_id, "live_fades");
                    self.apply_local(path, serde_json::to_value(fades).unwrap_or_default()).await;
                    moved.push(fixture_id);
                }
                PlaybackEffect::SetCueActive { cue_id, is_active } => {
                    let path = entity_field_path("cues", cue_id, "is_active");
                    self.apply_local(path, serde_json::Value::Bool(is_active)).await;
                }
                PlaybackEffect::GoNext { sequence_id, at } => {
                    if is_follower {
                        continue;
                    }
                    let path = entity_field_path("sequences", sequence_id, "goNext");
                    // The instant the follow came due, not the instant this station
                    // got round to acting on it, so every station anchors the cue the
                    // follow fires at the same millisecond.
                    self.run_synced_command(path, serde_json::json!({ "at": at })).await;
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
                sync.broadcast_synced(path, args, self.clock.clone(), Authorship::none()).await;
            }
        }
    }

    /// Tell the flow tick about a fade, but only where something is watching.
    ///
    /// A cue's own output is show state like any other, so a flow ought to be able
    /// to react to it — the alternative is a *Watch* node that offers every driven
    /// parameter and silently never fires for any of them.
    ///
    /// The gate matters: this runs at 40 Hz for every fixture in a fade, and without
    /// it a 500-fixture rig would queue thousands of events a second for a graph
    /// that reads none of them. `watched` is rebuilt only when the graphs change.
    fn queue_watched_changes(
        &mut self,
        fixture_id: Uuid,
        values: &std::collections::HashMap<String, pult_schema::types::fixture::ParameterValue>,
    ) {
        if self.watched.is_empty() {
            return;
        }
        let previous = self
            .state
            .entity("fixtures", fixture_id)
            .and_then(|entity| entity.get("live_values"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        for (key, current) in values {
            if !self.watched.contains(&(fixture_id, key.clone())) {
                continue;
            }
            let before = previous
                .get(key)
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            if before.as_ref() == Some(current) {
                continue;
            }
            self.input_events.push(InputEvent {
                fixture_id,
                key: key.clone(),
                previous: before,
                current: current.clone(),
            });
        }
    }

    /// What every *Watch* node in the show is looking at.
    fn refresh_watched(&mut self) {
        use pult_schema::types::flow::{FlowNodeKind, TriggerSource};

        let nodes: Vec<pult_schema::types::flow::FlowNode> = self.read_collection("flow_nodes");
        self.watched = nodes
            .iter()
            .filter_map(|node| match &node.kind {
                FlowNodeKind::Source(TriggerSource::Parameter { fixture_id, parameter }) => {
                    Some((*fixture_id, crate::model::playback::parameter_key(parameter)))
                }
                _ => None,
            })
            .collect();
    }

    // ── Flows ─────────────────────────────────────────────────────────────────

    /// Evaluate the flow graphs against whatever came in since the last tick.
    ///
    /// Only the leader fires. An action is a write to replicated state, so every
    /// node running the same graph would apply the same change several times — and
    /// the input that caused it only ever reached one node anyway.
    async fn flows_tick(&mut self) {
        let dirty = std::mem::take(&mut self.flows_dirty);
        if dirty {
            self.refresh_watched();
        }
        let inputs = std::mem::take(&mut self.input_events);
        if inputs.is_empty() && !dirty && !self.flows.has_work() {
            return;
        }
        if self.state.is_follower() {
            return;
        }

        let flows: Vec<pult_schema::types::flow::Flow> = self.read_collection("flows");
        if flows.is_empty() {
            return;
        }
        let nodes: Vec<pult_schema::types::flow::FlowNode> = self.read_collection("flow_nodes");
        let edges: Vec<pult_schema::types::flow::FlowEdge> = self.read_collection("flow_edges");
        let graph = FlowGraph { flows: &flows, nodes: &nodes, edges: &edges };
        let effects =
            self.flows.tick(tokio::time::Instant::now().into_std(), &graph, &inputs);

        self.apply_flow_effects(effects).await;
    }

    async fn apply_flow_effects(&mut self, effects: Vec<FlowEffect>) {
        for effect in effects {
            match effect {
                FlowEffect::SetActive { node_id, active } => {
                    let path = entity_field_path("flow_nodes", node_id, "active");
                    self.write_synced(path, serde_json::Value::Bool(active)).await;
                }
                FlowEffect::Fire { node_id, action } => {
                    self.run_flow_action(action).await;
                    let path = entity_field_path("flow_nodes", node_id, "last_fired_at");
                    let value = serde_json::to_value(chrono::Utc::now()).unwrap_or_default();
                    self.write_synced(path, value).await;
                }
            }
        }
    }

    async fn run_flow_action(&mut self, action: pult_schema::types::flow::TriggerAction) {
        use pult_schema::types::flow::TriggerAction;
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
                    debug!("[flows] set parameter on {fixture_id}: {e}");
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
        self.log_local_write(&path, &value, Lifecycle::Synced, &Authorship::none()).await;
        self.broadcast_after_set(&path, value.clone());
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, value, self.clock.clone(), Authorship::none()).await;
        }
    }

    /// Hand the configured outputs to the output side, when they have changed.
    ///
    /// The manager reconciles rather than rebuilds, so sending this on every edit is
    /// cheap; sending it on every tick would not be, which is what the flag is for.
    async fn push_output_config(&mut self) {
        if !self.outputs_dirty {
            return;
        }
        self.outputs_dirty = false;
        let Some(output) = &self.output else { return };
        output.configure(self.read_collection("outputs"));
    }

    /// Hand the current patch to the output plugins.
    ///
    /// Sent on every tick that did any work, including a patch edit that moved no
    /// light, because a re-addressed fixture changes the wire without changing a
    /// single level. Plugins decide for themselves what is worth transmitting.
    async fn push_output(&mut self, moved: Vec<Uuid>) {
        let Some(output) = &self.output else { return };
        let fixtures: Vec<pult_schema::types::fixture::Fixture> = self.read_collection("fixtures");
        if fixtures.is_empty() && !self.pushed_fixtures {
            return;
        }
        // The one empty patch that follows the last fixture being unpatched is
        // worth sending: a plugin keeping state per fixture — what it last sent, or
        // which fixtures nothing reaches — has to hear that there are none left.
        self.pushed_fixtures = !fixtures.is_empty();
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
        self.log_local_write(&path, &values, Lifecycle::Synced, &Authorship::none()).await;
        self.broadcast_after_set(&path, values.clone());
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, values, self.clock.clone(), Authorship::none()).await;
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
        authorship: &Authorship,
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
            user_id: authorship.user_id,
            previous: authorship.previous.clone(),
            undoes: authorship.undoes,
            gesture: authorship.gesture,
        };
        self.log_operation(&op).await;
    }

    /// How far back this show keeps its history, in changes.
    ///
    /// Read from the show on every press rather than cached, because it is ordinary
    /// replicated show data: a second console changing it should take effect here
    /// without this one being restarted.
    fn history_depth(&self) -> u32 {
        let depth = self
            .state
            .get_by_path(&vec![
                PathSegment::Key("show".into()),
                PathSegment::Key("history_depth".into()),
            ])
            .and_then(|v| v.as_u64())
            .unwrap_or(HISTORY_DEPTH_DEFAULT as u64);
        clamp_history_depth(depth.try_into().unwrap_or(u32::MAX))
    }

    /// How long this show takes to let a parameter go, in milliseconds.
    ///
    /// Read from the show on every tick for the same reason as the depth above: a
    /// second console changing it should take effect here, and both stations then
    /// fade home over the same time rather than each over its own.
    fn home_fade_ms(&self) -> u32 {
        let ms = self
            .state
            .get_by_path(&vec![
                PathSegment::Key("show".into()),
                PathSegment::Key("home_fade_ms".into()),
            ])
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        clamp_home_fade_ms(ms.try_into().unwrap_or(u32::MAX))
    }

    /// What is at a path right now, for the oplog to remember.
    ///
    /// A create has nothing before it and a delete has the whole entity, and both
    /// read correctly here: `get_by_path` on a `__create` path finds nothing, and on
    /// a `__delete` path finds the entity about to go. So one read covers all three
    /// shapes of write and the undo code does not have to ask what kind it is.
    fn value_before(&self, path: &Path) -> Option<serde_json::Value> {
        let target = match path.as_slice() {
            // `[table, "__create"]` — nothing is there yet.
            [PathSegment::Key(_), PathSegment::Key(action)] if action == "__create" => {
                return Some(serde_json::Value::Null)
            }
            // `[table, id, "__delete"]` — the entity itself is what is lost.
            [table @ PathSegment::Key(_), id, PathSegment::Key(action)] if action == "__delete" => {
                vec![table.clone(), id.clone()]
            }
            _ => path.clone(),
        };
        Some(self.state.get_by_path(&target).unwrap_or(serde_json::Value::Null))
    }

    /// Take back this user's most recent change, or put back their most recent undo.
    ///
    /// The undo is written like any other change and logged pointing at what it
    /// reversed, so it replicates to peers, reaches the same user's other client,
    /// and turns up in the history as itself.
    async fn take_back(&mut self, user_id: Uuid, redo: bool) -> Vec<Path> {
        let Ok(log) = oplog::recent_by_people(&self.pool, self.history_depth()).await else {
            return Vec::new();
        };
        let run = if redo {
            undo::next_to_redo(&log, user_id)
        } else {
            undo::next_to_undo(&log, user_id)
        };
        let Some(head) = run.first() else { return Vec::new() };

        // Every write of the reversal points at the gesture it reverses and shares a
        // gesture of its own, so putting a drag back is one Ctrl-Shift-Z rather than
        // one per path — and so the next undo sees the reversal as the single act it
        // was.
        let reverses = undo::gesture_key(head);
        let authorship =
            Authorship::by(Some(user_id), None).during(Some(Uuid::new_v4())).reversing(reverses);
        let inverses = undo::inverses_of_run(&run);
        let mut moved = Vec::new();
        for inverse in inverses {
            let lifecycle = pult_schema::registry::path_lifecycle(&inverse.path);
            // Before the write, or it reads back what it just put there and the undo
            // becomes its own inverse.
            let mut written = authorship.clone();
            written.previous = self.value_before(&inverse.path);
            if self.apply_set(inverse.path.clone(), inverse.value.clone(), lifecycle).await.is_err()
            {
                // One path of a gesture failing does not make the rest wrong. A
                // fixture deleted by somebody else since is the ordinary case, and
                // abandoning the other nineteen faders over it would be worse.
                continue;
            }
            moved.push(inverse.path.clone());
            self.state_version += 1;
            self.record_write(&inverse.path, lifecycle);
            self.log_local_write(&inverse.path, &inverse.value, lifecycle, &written).await;
            self.broadcast_after_set(&inverse.path, inverse.value.clone());
            if lifecycle != Lifecycle::Local {
                if let Some(sync) = &self.sync {
                    sync.broadcast_synced(
                        inverse.path,
                        inverse.value,
                        self.clock.clone(),
                        written,
                    )
                    .await;
                }
            }
        }

        moved
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

    /// Turn a write that says how far, or where something rests, into ordinary
    /// absolute writes — or hand back what came in.
    ///
    /// Four shapes carry a verb:
    ///
    /// ```text
    /// [table, ref, field, "__by"]      <delta>
    /// ["programmer_values", "__by"]    { "fixtureId", "parameterKind", "by" }
    /// ["programmer_values", "__home"]  { "fixtureId", "parameterKind"? }
    /// ["fixtures", "__set_home"]       { "fixtureId", "parameterKind"? }
    /// ```
    ///
    /// The first is the primitive: relative to what that field says now. The second
    /// exists because the programmer's ordinary case is *not* already holding the
    /// key — `at +10` on a light nobody has touched has no row to name — and because
    /// what it has to be relative to is then what playback is showing rather than a
    /// row that does not exist. The third is the same act with a destination the
    /// station knows and the caller does not have to: what that parameter rests at.
    /// The fourth is that one backwards — where it rests becomes where it is now.
    ///
    /// Those three shapes are the only places the engine names a collection for a
    /// reason of its own; it costs nothing that matters, since adding a collection
    /// still needs no edit here.
    ///
    /// A verb can be several writes: `__home` without a `parameterKind` is every
    /// output parameter of the fixture, which is what lets a caller ask for home
    /// without first reading what the fixture has.
    ///
    /// Pure with respect to the show: this only reads. The writes it describes
    /// happen in `apply_set` like any other.
    fn resolve_verbs(
        &self,
        path: Path,
        value: serde_json::Value,
    ) -> Result<Vec<(Path, serde_json::Value)>, BackendError> {
        let by = || -> Result<f32, BackendError> {
            value
                .as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| BackendError::InvalidValue {
                    path: path.clone(),
                    reason: "a relative write takes a number to move by".into(),
                })
        };

        match path.as_slice() {
            // The programmer, which may have to take the key to nudge it.
            [PathSegment::Key(table), PathSegment::Key(verb)]
                if table == "programmer_values" && verb == "__by" =>
            {
                self.nudge_programmer(&path, &value).map(|write| vec![write])
            }
            // The programmer again, sent back to where the rig rests.
            [PathSegment::Key(table), PathSegment::Key(verb)]
                if table == "programmer_values" && verb == "__home" =>
            {
                self.home_programmer(&path, &value)
            }
            // The other direction: where it rests becomes wherever it is now.
            [PathSegment::Key(table), PathSegment::Key(verb)]
                if table == "fixtures" && verb == "__set_home" =>
            {
                self.set_home_from_output(&path, &value)
            }
            // Any field of any row: relative to what it says now.
            [table @ PathSegment::Key(_), seg, field @ PathSegment::Key(_), PathSegment::Key(verb)]
                if verb == "__by" =>
            {
                let target = vec![table.clone(), seg.clone(), field.clone()];
                let current = self
                    .state
                    .get_by_path(&target)
                    .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
                let next = nudge_json(&current, by()?).map_err(|reason| {
                    BackendError::InvalidValue { path: path.clone(), reason }
                })?;
                Ok(vec![(target, next)])
            }
            // Only a fixture has an output to take a resting place from.
            [.., PathSegment::Key(verb)] if verb == "__set_home" => {
                Err(BackendError::InvalidValue {
                    path: path.clone(),
                    reason: "only a fixture's parameters rest anywhere; \
                             write to [\"fixtures\", \"__set_home\"]"
                        .into(),
                })
            }
            // `__by` on a create, or on a whole row, means nothing — and doing
            // something almost-right with it would be worse than saying so.
            [.., PathSegment::Key(verb)] if verb == "__by" => Err(BackendError::InvalidValue {
                path: path.clone(),
                reason: "a relative write names one field, or the programmer".into(),
            }),
            // Home is a fact about a fixture's parameters, so there is nowhere else
            // for it to mean anything. A cue does not rest anywhere.
            [.., PathSegment::Key(verb)] if verb == "__home" => Err(BackendError::InvalidValue {
                path: path.clone(),
                reason: "only a fixture's parameters have somewhere to rest; \
                         write to [\"programmer_values\", \"__home\"]"
                    .into(),
            }),
            _ => Ok(vec![(path, value)]),
        }
    }

    /// `["programmer_values", "__by"]` with `{ fixtureId, parameterKind, by }`.
    ///
    /// Relative to what is showing, which is task 14's stack read rather than
    /// re-implemented: the programmer's own value where it holds the key, and the
    /// fixture's live value where it does not. A key held as a running shape refuses
    /// — nudging a shape means moving its offset, which is a different thing wearing
    /// the same word.
    fn nudge_programmer(
        &self,
        path: &Path,
        args: &serde_json::Value,
    ) -> Result<(Path, serde_json::Value), BackendError> {
        let bad = |reason: &str| BackendError::InvalidValue {
            path: path.clone(),
            reason: reason.to_string(),
        };

        let fixture_id = args
            .get("fixtureId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| bad("a relative programmer write needs a fixtureId"))?
            .to_string();
        let kind: pult_schema::types::fixture::ParameterKind = args
            .get("parameterKind")
            .cloned()
            .and_then(|k| serde_json::from_value(k).ok())
            .ok_or_else(|| bad("a relative programmer write needs a parameterKind"))?;
        let by = args
            .get("by")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| bad("a relative programmer write needs a number to move by"))?
            as f32;

        let key = parameter_key(&kind);
        let entry_id: Uuid = programmer_entry_id(&fixture_id, &key)
            .parse()
            .map_err(|_| bad("the derived programmer id is not a uuid"))?;
        let row = self
            .state
            .get_by_path(&vec![
                PathSegment::Key("programmer_values".into()),
                PathSegment::Id(entry_id),
            ])
            .filter(|v| !v.is_null());

        if let Some(row) = row {
            if row.get("effect").is_some_and(|e| !e.is_null()) {
                return Err(bad("that parameter is running a shape; clear it before nudging it"));
            }
            let current: ParameterValue = serde_json::from_value(
                row.get("value").cloned().unwrap_or(serde_json::Value::Null),
            )
            .map_err(|e| bad(&format!("the held value does not parse: {e}")))?;
            let next = current.nudged(by).map_err(|reason| bad(&reason))?;
            return Ok((
                vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Id(entry_id),
                    PathSegment::Key("value".into()),
                ],
                serde_json::to_value(next)?,
            ));
        }

        // Nothing held, so the programmer takes the key — starting from what
        // playback is showing, or from what the fixture type says the parameter
        // rests at when nothing has ever driven it.
        let fixture_uuid: Uuid =
            fixture_id.parse().map_err(|_| bad("that fixtureId is not a uuid"))?;
        let showing = self
            .state
            .get_by_path(&vec![
                PathSegment::Key("fixtures".into()),
                PathSegment::Id(fixture_uuid),
                PathSegment::Key("live_values".into()),
                PathSegment::Key(key.clone()),
            ])
            .filter(|v| !v.is_null())
            .map(|showing| {
                serde_json::from_value::<ParameterValue>(showing)
                    .map_err(|e| bad(&format!("the live value does not parse: {e}")))
            })
            .transpose()?
            .or_else(|| self.home_value_of(fixture_uuid, &kind))
            .ok_or_else(|| bad("that fixture has no such parameter"))?;
        let current = showing;
        let next = current.nudged(by).map_err(|reason| bad(&reason))?;

        Ok((
            vec![
                PathSegment::Key("programmer_values".into()),
                PathSegment::Key("__create".into()),
            ],
            serde_json::json!({
                "id": entry_id,
                "fixture_id": fixture_id,
                "parameter_kind": kind,
                "value": next,
                "locked": false,
            }),
        ))
    }

    /// Where a parameter rests when nothing is driving it, for a key nothing has
    /// ever driven.
    ///
    /// The resolution is the schema's: this fixture's own override if it has one, and
    /// what its type declares otherwise — where the type's answer is derived from
    /// what the device said about its own ports, so it is the node's answer rather
    /// than the console's guess.
    fn home_value_of(&self, fixture_id: Uuid, kind: &ParameterKind) -> Option<ParameterValue> {
        let (fixture, fixture_type) = self.fixture_and_type(fixture_id)?;
        home_value(&fixture, &fixture_type, kind)
    }

    /// One fixture and the type it was patched as, as the schema's own structs.
    ///
    /// Read out of the state tree and parsed rather than picked at as JSON, because
    /// everything asked of them here — an override, a parameter's direction, its
    /// default — is a question the schema already answers.
    fn fixture_and_type(&self, fixture_id: Uuid) -> Option<(Fixture, FixtureType)> {
        let fixture = self
            .state
            .get_by_path(&vec![
                PathSegment::Key("fixtures".into()),
                PathSegment::Id(fixture_id),
            ])
            .filter(|v| !v.is_null())?;
        let fixture: Fixture = serde_json::from_value(fixture).ok()?;
        let fixture_type = self
            .state
            .get_by_path(&vec![
                PathSegment::Key("fixture_types".into()),
                PathSegment::Id(fixture.fixture_type_id),
            ])
            .filter(|v| !v.is_null())?;
        let fixture_type: FixtureType = serde_json::from_value(fixture_type).ok()?;
        Some((fixture, fixture_type))
    }

    /// `["programmer_values", "__home"]` with `{ fixtureId, parameterKind? }`.
    ///
    /// The programmer takes each parameter at the value it rests at. A destination
    /// like any other by the time anything records it — which is what lets a client
    /// that can set a level ask for home without being able to read the rig, and is
    /// the same argument `__by` made for "a bit darker".
    ///
    /// Without a `parameterKind` this is every parameter an operator can set on that
    /// fixture: enumerating them is the station's job, so that no client has to hold
    /// a copy of what a fixture has.
    ///
    /// A parked value is left where it was parked. Parking is exactly the ask that a
    /// value survive being taken away, and home takes values away.
    fn home_programmer(
        &self,
        path: &Path,
        args: &serde_json::Value,
    ) -> Result<Vec<(Path, serde_json::Value)>, BackendError> {
        let bad = |reason: &str| BackendError::InvalidValue {
            path: path.clone(),
            reason: reason.to_string(),
        };

        let fixture_id = args
            .get("fixtureId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| bad("sending something home needs a fixtureId"))?
            .to_string();
        let fixture_uuid: Uuid =
            fixture_id.parse().map_err(|_| bad("that fixtureId is not a uuid"))?;
        let (fixture, fixture_type) = self
            .fixture_and_type(fixture_uuid)
            .ok_or_else(|| bad("no fixture of that id is patched here"))?;

        // One parameter when named, and everything an operator can set when not.
        let named = match args.get("parameterKind") {
            Some(k) if !k.is_null() => Some(
                serde_json::from_value::<ParameterKind>(k.clone())
                    .map_err(|e| bad(&format!("that is not a parameter kind: {e}")))?,
            ),
            _ => None,
        };
        let kinds: Vec<ParameterKind> = match &named {
            Some(kind) => {
                let definition = fixture_type
                    .parameters
                    .iter()
                    .find(|p| p.kind == *kind)
                    .ok_or_else(|| bad("that fixture has no such parameter"))?;
                if definition.direction != ParameterDirection::Output {
                    return Err(bad(
                        "that parameter is one the device writes and the show reads; \
                         there is nothing to send home",
                    ));
                }
                vec![kind.clone()]
            }
            None => output_parameters(&fixture_type).map(|p| p.kind.clone()).collect(),
        };

        let mut writes = Vec::new();
        for kind in kinds {
            let key = parameter_key(&kind);
            let Some(value) = home_value(&fixture, &fixture_type, &kind) else {
                continue;
            };
            let entry_id: Uuid = programmer_entry_id(&fixture_id, &key)
                .parse()
                .map_err(|_| bad("the derived programmer id is not a uuid"))?;
            let row = self
                .state
                .get_by_path(&vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Id(entry_id),
                ])
                .filter(|v| !v.is_null());

            if let Some(row) = row {
                if row.get("locked").is_some_and(|l| l == &serde_json::Value::Bool(true)) {
                    // Named on its own, being told is better than being ignored.
                    // Swept up with the rest of a fixture, it is the parking working.
                    if named.is_some() {
                        return Err(bad("that value is parked; unpark it before sending it home"));
                    }
                    continue;
                }
                writes.push((
                    vec![
                        PathSegment::Key("programmer_values".into()),
                        PathSegment::Id(entry_id),
                        PathSegment::Key("value".into()),
                    ],
                    serde_json::to_value(&value)?,
                ));
                // A key held as a running shape stops being one: it was asked to rest.
                if row.get("effect").is_some_and(|e| !e.is_null()) {
                    writes.push((
                        vec![
                            PathSegment::Key("programmer_values".into()),
                            PathSegment::Id(entry_id),
                            PathSegment::Key("effect".into()),
                        ],
                        serde_json::Value::Null,
                    ));
                }
                continue;
            }

            writes.push((
                vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Key("__create".into()),
                ],
                serde_json::json!({
                    "id": entry_id,
                    "fixture_id": fixture_id,
                    "parameter_kind": kind,
                    "value": value,
                    "locked": false,
                }),
            ));
        }

        Ok(writes)
    }

    /// `["fixtures", "__set_home"]` with `{ fixtureId, parameterKind? }`.
    ///
    /// The opposite act to `__home`, and the one an operator actually performs on a
    /// house light: rather than sending a parameter to where it rests, it makes where
    /// it rests be wherever the parameter is now. Aim the light, look at it, keep it.
    ///
    /// A verb rather than an ordinary write to `home_values` because the value being
    /// stored is one only the station holds. A browser could read `live_values` and
    /// write the map itself; the command line and a plugin with no data access could
    /// not, and the whole argument `__by` and `__home` made was that a caller able to
    /// act should not have to be a caller able to read the rig.
    ///
    /// One write, of the whole map. `home_values` is a single JSON column, so that is
    /// the shape of the field — and it means taking a fixture's whole output is one
    /// thing to undo rather than one per parameter.
    fn set_home_from_output(
        &self,
        path: &Path,
        args: &serde_json::Value,
    ) -> Result<Vec<(Path, serde_json::Value)>, BackendError> {
        let bad = |reason: &str| BackendError::InvalidValue {
            path: path.clone(),
            reason: reason.to_string(),
        };

        let fixture_id = args
            .get("fixtureId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| bad("taking a home value needs a fixtureId"))?;
        let fixture_uuid: Uuid =
            fixture_id.parse().map_err(|_| bad("that fixtureId is not a uuid"))?;
        let (fixture, fixture_type) = self
            .fixture_and_type(fixture_uuid)
            .ok_or_else(|| bad("no fixture of that id is patched here"))?;

        // One parameter when named, and everything an operator can set when not — the
        // same two shapes `__home` takes, so that the pair reads as a pair.
        let named = match args.get("parameterKind") {
            Some(k) if !k.is_null() => Some(
                serde_json::from_value::<ParameterKind>(k.clone())
                    .map_err(|e| bad(&format!("that is not a parameter kind: {e}")))?,
            ),
            _ => None,
        };
        let kinds: Vec<ParameterKind> = match &named {
            Some(kind) => {
                let definition = fixture_type
                    .parameters
                    .iter()
                    .find(|p| p.kind == *kind)
                    .ok_or_else(|| bad("that fixture has no such parameter"))?;
                if definition.direction != ParameterDirection::Output {
                    return Err(bad(
                        "that parameter is one the device writes and the show reads; \
                         it does not rest anywhere",
                    ));
                }
                vec![kind.clone()]
            }
            None => output_parameters(&fixture_type).map(|p| p.kind.clone()).collect(),
        };

        let mut home = fixture.home_values.clone();
        let mut took = false;
        for kind in kinds {
            let key = parameter_key(&kind);
            let Some(value) = fixture.live_values.get(&key) else {
                // Named on its own, being told is better than being ignored — an
                // operator who asked for one parameter is looking at that parameter.
                if named.is_some() {
                    return Err(bad(
                        "that parameter is not putting anything out, so there is \
                         nothing to take",
                    ));
                }
                continue;
            };
            home.insert(key, value.clone());
            took = true;
        }
        // Nothing on stage to take. Not an error: a fixture that has never been driven
        // is an ordinary state, and writing the map back unchanged would put a change
        // that changed nothing into the history panel.
        if !took {
            return Ok(Vec::new());
        }

        Ok(vec![(
            vec![
                PathSegment::Key("fixtures".into()),
                PathSegment::Id(fixture_uuid),
                PathSegment::Key("home_values".into()),
            ],
            serde_json::to_value(home)?,
        )])
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
        if table == "outputs" {
            self.outputs_dirty = true;
        }
        // A button press is a write to `flow_nodes`, and the flow tick is otherwise
        // only woken by an input or a running delay. Named here for the same reason
        // `outputs` is: the alternative is polling every graph forty times a second
        // for something that almost never changes.
        if table == "flow_nodes" {
            self.flows_dirty = true;
        }

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
            sync.broadcast_synced(path, args, self.clock.clone(), Authorship::none()).await;
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
        // Anything below a prune floor is gone, so the rows that survive are not the
        // whole answer to what this peer missed. Handing them over would report
        // success and lose the writes that were cut, which is the one way pruning can
        // corrupt a session rather than merely cost it a snapshot.
        if oplog::behind_the_floor(known, &oplog::floor(&self.pool).await.ok()?) {
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
            return;
        }
        // Every so often, rather than on a timer: what should drive the work is how
        // much has been written, and a timer would wake to do nothing on an idle show
        // while still landing mid-burst on a busy one.
        use std::sync::atomic::Ordering;
        let n = self.appends_since_prune.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= APPENDS_BETWEEN_PRUNES {
            self.appends_since_prune.store(0, Ordering::Relaxed);
            self.prune_the_log();
        }
    }

    /// Broadcast the right value for a completed set operation.
    /// __create/__delete paths broadcast the updated parent collection so frontends
    /// subscribed to e.g. "cues" see the change without re-fetching.
    /// All other paths broadcast the path/value pair as-is.
    fn broadcast_after_set(&self, path: &Path, value: serde_json::Value) {
        // A create, a delete, or a field of a singleton: send the whole thing.
        //
        // A subscriber watching `show` is watching the show, and a pattern is matched
        // against the path a write names — so a field write to `show/name` reaches
        // nobody who asked for `show`. Collections already answer this by sending the
        // collection back; a singleton is the same problem with one row.
        //
        // Entities are deliberately not treated this way. A field write there is
        // `fixtures/<id>/live_values` at forty a second during a fade, and sending the
        // whole rig each time is the thing `subscribeDeep` exists to avoid.
        let whole = match path.as_slice() {
            [PathSegment::Key(k), PathSegment::Key(a)] if a == "__create" => Some(k.as_str()),
            [PathSegment::Key(k), _, PathSegment::Key(a)] if a == "__delete" => Some(k.as_str()),
            [PathSegment::Key(k), PathSegment::Key(_)]
                if EntityMeta::by_table(k).is_some_and(|m| m.is_singleton) =>
            {
                Some(k.as_str())
            }
            _ => None,
        };
        if let Some(key) = whole {
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
                self.outputs_dirty = true;
                // Whole graphs arrived at once, so what they watch has to be worked
                // out again before the next fade is offered to them.
                self.flows_dirty = true;
            }
            Err(e) => warn!("[engine] load_from_showfile: ShowState deserialization failed: {e}"),
        }
        self.seed_default_user().await;
        // A showfile that has been round a long tech week arrives past both
        // retentions, and this is the largest cut it will ever take.
        self.prune_the_log();
    }

    /// Bring the log back within its retentions, off the actor's own loop.
    ///
    /// **Spawned rather than awaited.** The engine is one actor, and this is the only
    /// place in it that issues a `DELETE` over what can be a million rows. Awaiting
    /// that here would be a stalled tick — output stopping while the disk works —
    /// which is a far worse failure than a log that is briefly still too long.
    ///
    /// Nothing waits on the result and nothing needs to: pruning is idempotent, and a
    /// prune that fails or is interrupted leaves a log that is merely longer than it
    /// should be, which the next one fixes.
    fn prune_the_log(&self) {
        use std::sync::atomic::Ordering;
        if self.pruning.swap(true, Ordering::SeqCst) {
            return; // One is already running; a second would race it on the floor.
        }
        let (pool, running) = (self.pool.clone(), self.pruning.clone());
        let depth = self.history_depth();
        let retention =
            chrono::Duration::minutes(crate::infra::preferences::load().oplog_retention_minutes
                as i64);
        tokio::spawn(async move {
            match oplog::prune(&pool, depth, retention).await {
                Ok(0) => {}
                Ok(cut) => debug!("[oplog] pruned {cut} operations"),
                Err(e) => warn!("[oplog] could not prune: {e}"),
            }
            running.store(false, Ordering::SeqCst);
        });
    }

    /// Give the show an operator, if it does not have one already.
    ///
    /// Undo is per person and an unattributed write can never be taken back, so a show
    /// with no users at all is a show where the first thing anybody does is permanent.
    /// Seeding here rather than in the first browser to connect is what covers a
    /// station running headless: plugins and station RPCs write too, and they are
    /// somebody.
    ///
    /// **Only when absent.** `create_entity` does not check whether the id is already
    /// there, so an unconditional create at every load would rewrite the row on every
    /// start — and on a second station, replicate "Operator" over a name somebody
    /// chose. The check is the guard, not an optimisation.
    ///
    /// Unattributed, like the engine's other writes: nobody asked for it. An operator
    /// pressing Ctrl-Z on a fresh show should reach their own first change, not the
    /// console's act of inventing them.
    async fn seed_default_user(&mut self) {
        if self.state.entity("users", User::DEFAULT_ID).is_some() {
            return;
        }
        let Ok(value) = serde_json::to_value(User::default_user()) else { return };
        let path: Path =
            vec![PathSegment::Key("users".into()), PathSegment::Key("__create".into())];
        if let Err(e) = self.apply_set(path.clone(), value.clone(), Lifecycle::Persisted).await {
            warn!("[engine] could not seed the default user: {e}");
            return;
        }
        self.record_write(&path, Lifecycle::Persisted);
        self.log_local_write(&path, &value, Lifecycle::Persisted, &Authorship::none()).await;
        self.broadcast_after_set(&path, value.clone());
        if let Some(sync) = &self.sync {
            sync.broadcast_synced(path, value, self.clock.clone(), Authorship::none()).await;
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
            self.outputs_dirty = true;
            self.flows_dirty = true;
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

pub mod undo;



#[cfg(test)]
mod tests;
