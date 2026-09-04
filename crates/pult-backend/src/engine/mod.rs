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
        client::ClientStatsMap,
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
    model::playback::{parameter_key, Playback, PlaybackEffect, ShowView},
    model::flows::{FlowEffect, FlowGraph, Flows, InputEvent},
};

/// How often a collection may be broadcast whole while a burst of writes is arriving.
///
/// A create has to send the collection, and the collection is the whole rig. At five
/// thousand fixtures that is an expensive thing to serialise, and doing it per created
/// row is what made patching a rig cost 89 seconds against 3 for everything else on the
/// path. Fifty milliseconds is twenty a second: faster than anybody reads a list filling
/// up, and 1/64th of what an import used to pay.
///
/// It is a *ceiling on a burst*, not a delay on a write. An idle console has not flushed
/// for far longer than this, so one write still goes out at once.
const COLLECTION_FLUSH_EVERY: std::time::Duration = std::time::Duration::from_millis(50);

/// What one write does to a saved version.
enum VersionTouched {
    Nothing,
    Taken(Uuid),
    Dropped(Uuid),
}

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
    ("clients", || serde_json::to_value(ClientStatsMap::default()).unwrap_or_default()),
    ("plugins", || serde_json::to_value(PluginsState::default()).unwrap_or_default()),
    // Which saved versions this station holds a snapshot for. LOCAL because it is a
    // fact about this machine's disk: a `versions` row replicates and the file it
    // names does not, so a station that joined after a version was taken has the row
    // and no file — and the panel can only say "not on this station" because the
    // station says which ones are.
    (crate::infra::showfile::versions::VERSIONS_HERE, || serde_json::json!([])),
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
    /// Merge one key into what a fixture's devices have reported.
    ///
    /// A device writing a sensor reading cannot read-modify-write from outside the
    /// actor: two ports reporting in the same millisecond would each write back a
    /// map missing the other's key. Merging inside the actor is the whole point.
    SetSensedValue {
        fixture_id: Uuid,
        key: String,
        value: serde_json::Value,
        reply: oneshot::Sender<Result<(), BackendError>>,
    },
    /// How many driven parameters the flow sampler is keeping a sample of.
    ///
    /// Test-only, and the one thing about the sampler that cannot be observed from
    /// outside: what it costs is the size of this set, and the property worth pinning
    /// down is that the set follows the *Watch* nodes rather than the rig.
    #[cfg(test)]
    SampledParameters { reply: oneshot::Sender<usize> },
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
    /// The same engine, reached through one source class's own queue.
    ///
    /// Which is what keeps a plugin in a write loop from crowding out an operator:
    /// the flood fills the plugin's queue and the router still gives the operator's
    /// its turns. See `engine::admission`.
    pub fn for_source(admission: &admission::Admission, source: admission::Source) -> Self {
        EngineHandle(admission.sender(source))
    }

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

    /// Merge one key into what a fixture's devices have reported, replicating the
    /// result.
    ///
    /// Unlike what is driving a parameter, which every node derives for itself from
    /// cue state, an input only exists on the node the device is talking to. It has to
    /// be sent.
    pub async fn set_sensed_value(
        &self,
        fixture_id: Uuid,
        key: String,
        value: serde_json::Value,
    ) -> Result<(), BackendError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(EngineCommand::SetSensedValue { fixture_id, key, value, reply: tx })
            .await
            .map_err(|_| BackendError::ChannelClosed)?;
        rx.await?
    }

    /// How many driven parameters the flow sampler is keeping a sample of.
    #[cfg(test)]
    pub async fn sampled_parameters(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        if self.0.send(EngineCommand::SampledParameters { reply: tx }).await.is_err() {
            return 0;
        }
        rx.await.unwrap_or(0)
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
    /// The disk, off this actor. Every persisted write goes here and the reply waits
    /// for the group commit rather than for its own fsync — see `engine::writer`.
    writer: writer::WriteHandle,
    /// Durability receipts for the writes this command has enqueued.
    ///
    /// The actor does not wait for the disk; it collects what it is owed and hands the
    /// lot to a task that answers the caller when they land. So an acknowledged write
    /// is still on the disk, and the *next* command is no longer behind this one's
    /// fsync — which is what lets the writer group anything at all.
    pending_writes: Vec<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    /// Collections whose whole contents a subscriber still has to be sent.
    ///
    /// A create or a delete has to broadcast the *collection*, because a subscriber
    /// watching `fixtures` is watching the collection and a pattern matched against
    /// `fixtures/__create` reaches nobody. Doing that per row meant deep-cloning every
    /// fixture as JSON once per created fixture — 89 seconds to patch five thousand,
    /// where the whole rest of the write path was three. So the collection is marked
    /// here and sent once the command queue is empty: one broadcast per burst instead
    /// of one per row, which is the rule `push_output` already follows and for the same
    /// reason.
    dirty_collections: std::collections::BTreeSet<String>,
    /// When a collection was last broadcast whole.
    ///
    /// An empty queue turned out to be a poor test on its own: a client with sixty-four
    /// writes in flight empties it between almost every one, so "flush when idle" still
    /// flushed per row. This bounds it in time instead — a burst sends the collection at
    /// a readable rate rather than a per-row one, and an idle console still sends a
    /// single write's collection at once, because the last flush is long past.
    flushed_at: std::time::Instant,
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
    /// How many times each collection has been written, so a consumer can ask whether
    /// anything **it reads** has moved rather than whether anything at all has.
    ///
    /// This was one counter over the whole show, and the difference is not academic.
    /// A station writes its own `stations` row every two seconds and its output status
    /// every second — diagnostics that cannot reach a lamp — and each of those made the
    /// engine re-read the rig and hand the connectors an identical patch. At five
    /// thousand fixtures that push costs 116 ms inside the output loop, so an idle
    /// console spent a sixth of every second rebuilding a picture nothing had changed,
    /// and drew at 30 Hz where `Frames::DMX` asks for 40.
    collection_versions: HashMap<String, u64>,
    /// Writes that name no collection, or that are too broad to attribute to one: a
    /// showfile loaded, a snapshot applied, a registered command run. Counted into
    /// every answer `version_of` gives, so the fallback is always "everybody moved".
    everything_version: u64,
    playback_seen: u64,
    /// The version the output side was last handed. Its own counter, because the
    /// output side has to hear about a change playback rightly ignores — a fixture
    /// patched into a dark house moves no light and still moves the wire.
    pushed_version: u64,
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
    /// What each of those was last seen at, which is what makes an edge visible.
    ///
    /// The one place in the console that still keeps a driven value, and it is as
    /// large as the *Watch* nodes in the show rather than as large as the rig.
    watch_samples: HashMap<(Uuid, String), pult_schema::types::fixture::ParameterValue>,
    /// When they were last looked at.
    watch_sampled_at: std::time::Instant,
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
    /// What to call this show if the file has no `show` row. From the bundle's
    /// manifest; `None` for a station with no bundle open, which then seeds nothing.
    seed_name: Option<String>,
    /// What turns a `versions` row into a file on this station's disk. `None` for a
    /// station with no bundle open, which has nowhere to put one.
    checkpointer: Option<crate::infra::showfile::versions::Checkpointer>,
}

/// How many appends between prunes.
///
/// Large enough that pruning is rare against the write rate, small enough that a
/// station left up for a fortnight is bounded while it runs rather than only when it
/// is next opened — which is the case that motivated this at all.
const APPENDS_BETWEEN_PRUNES: u32 = 1_000;

/// What `push_output` hands the connectors, and so the only writes that can change
/// what leaves this station.
///
/// Named here rather than deduced, because `push_output` reads exactly these three
/// and this is the question of whether reading them again would say anything new. A
/// collection missing from the list is a rig that stops updating, so anything added
/// to that read belongs here too.
const OUTPUT_COLLECTIONS: &[&str] = &["fixtures", "fixture_types", "programmer_values"];

/// What `playback_pass` reads. `show` is in it for `home_fade_ms`, which decides how
/// long a release takes.
const PLAYBACK_COLLECTIONS: &[&str] = &[
    "sequences",
    "cues",
    "fixtures",
    "fixture_types",
    "programmer_values",
    "speed_masters",
    "show",
];

/// How often a watched parameter is looked at.
///
/// 40 Hz, which is what the engine used to run its whole tick at and is finer than an
/// operator can see a threshold cross. The cost of it is one evaluation per watched
/// parameter, so it is a rate a show sets by how much it watches rather than by how
/// large its rig is.
const WATCH_SAMPLE: std::time::Duration = std::time::Duration::from_millis(25);

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

    /// The same, with the writer given its own pool.
    ///
    /// `start` passes a second pool to the showfile so a group commit does not sit in
    /// front of a peer's catch-up read. A test passes none and the writer shares this
    /// one, which is right for an in-memory show: every `sqlite::memory:` connection
    /// is its own database, so a second pool there would be a second, empty show.
    pub fn new_with_write_pool(
        node_id: NodeId,
        rx: mpsc::Receiver<EngineCommand>,
        pool: Arc<SqlitePool>,
        write_pool: Option<Arc<SqlitePool>>,
        sync: Option<SyncHandle>,
    ) -> (Self, UpdateBroadcast) {
        let writer = writer::start(write_pool.unwrap_or_else(|| pool.clone()));
        Self::build(node_id, rx, pool, writer, sync)
    }

    pub fn new_with_rx(
        node_id: NodeId,
        rx: mpsc::Receiver<EngineCommand>,
        pool: Arc<SqlitePool>,
        sync: Option<SyncHandle>,
    ) -> (Self, UpdateBroadcast) {
        let writer = writer::start(pool.clone());
        Self::build(node_id, rx, pool, writer, sync)
    }

    fn build(
        node_id: NodeId,
        rx: mpsc::Receiver<EngineCommand>,
        pool: Arc<SqlitePool>,
        writer: writer::WriteHandle,
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
            writer,
            pending_writes: Vec::new(),
            dirty_collections: Default::default(),
            flushed_at: std::time::Instant::now() - COLLECTION_FLUSH_EVERY,
            sync,
            commands: build_command_table(),
            playback: Playback::default(),
            flows: Flows::default(),
            input_events: Vec::new(),
            output: None,
            path_clocks: HashMap::new(),
            collection_versions: HashMap::new(),
            everything_version: 0,
            playback_seen: 0,
            pushed_version: 0,
            outputs_dirty: true,
            pushed_fixtures: false,
            flows_dirty: true,
            watched: Default::default(),
            watch_samples: HashMap::new(),
            watch_sampled_at: std::time::Instant::now(),
            appends_since_prune: Default::default(),
            pruning: Default::default(),
            seed_name: None,
            checkpointer: None,
        };
        (engine, broadcast)
    }

    /// Attach an output plugin manager. Call before `run`.
    pub fn set_output(&mut self, output: OutputHandle) {
        self.output = Some(output);
    }

    /// What to call this show if the file has no `show` row yet. Call before `run`.
    ///
    /// The bundle's `bundle.toml` knows the name the operator typed when they made
    /// the folder, and nothing else does — a fresh `show.db` has no row at all.
    pub fn set_seed_name(&mut self, name: impl Into<String>) {
        self.seed_name = Some(name.into());
    }

    /// Attach the thing that copies the show when a version is taken. Call before
    /// `run`.
    pub fn set_checkpointer(
        &mut self,
        checkpointer: crate::infra::showfile::versions::Checkpointer,
    ) {
        self.checkpointer = Some(checkpointer);
    }

    pub async fn run(mut self) {
        loop {
            // What the engine does on its own, and when. There is no rate here: a fade
            // in progress is nothing for the engine to do, because nobody is storing
            // what it is worth. What is left is a follow cue coming due, a *Watch* node
            // wanting a sample, and work left over from the last command — so a station
            // running a show with neither of those sleeps until somebody speaks to it.
            // Anything a create or a delete made stale goes out now, before this
            // loop blocks — but only once there is nothing left queued, so a burst
            // costs one broadcast rather than one per row. `is_empty` is the whole
            // test: while a rig is being patched the queue is never empty, and the
            // moment it is, the last state is the one worth sending.
            if !self.dirty_collections.is_empty()
                && self.rx.is_empty()
                && self.flushed_at.elapsed() >= COLLECTION_FLUSH_EVERY
            {
                self.flush_collections();
            }

            // A collection still owed a broadcast has to be able to wake this loop, or
            // the ceiling above becomes a hole: the last write of a burst marks the
            // collection, the flush is not due yet, and the loop then blocks on
            // whatever the show happens to want next — which on an idle station is a
            // long time and on a settled one is never. So the sleep is shortened to
            // whatever is left of the interval.
            let owed = (!self.dirty_collections.is_empty())
                .then(|| COLLECTION_FLUSH_EVERY.saturating_sub(self.flushed_at.elapsed()));
            let until = match owed {
                Some(left) => self.next_wake().min(left),
                None => self.next_wake(),
            };
            let wake = tokio::time::sleep(until);

            // `biased` so a burst of commands — opening a showfile, a peer catching us
            // up — drains before any of it is acted on, rather than each write dragging
            // a pass and an output push behind it.
            let cmd = tokio::select! {
                biased;
                cmd = self.rx.recv() => cmd,
                _ = wake => {
                    // Possibly the only reason this woke.
                    if !self.dirty_collections.is_empty() {
                        self.flush_collections();
                    }
                    self.push_output_config().await;
                    let moved = self.playback_pass().await;
                    // The output side hears about the show whether or not playback had
                    // anything to say: a re-addressed or newly patched fixture changes
                    // the wire without changing a single level.
                    let rig = self.version_of(OUTPUT_COLLECTIONS);
                    if self.pushed_version != rig || !moved.is_empty() {
                        self.pushed_version = rig;
                        self.push_output(moved).await;
                    }
                    self.sample_watched();
                    self.flows_tick().await;
                    continue;
                }
            };
            let Some(cmd) = cmd else { break };
            match cmd {
                EngineCommand::Stop => break,
                EngineCommand::LoadFromShowfile => {
                    self.load_from_showfile().await;
                    self.touch_everything();
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
                    let resolved = match self.resolve_verbs(path, value, authorship.user_id) {
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
                        // Read before the write, because a delete says which version
                        // it is deleting only by naming it — and by the time the write
                        // has been applied, the row it named is gone.
                        let touched_versions = self.version_touched_by(&path, &value);
                        result = self.apply_set(path.clone(), value.clone(), lifecycle).await;
                        if result.is_err() {
                            break;
                        }
                        self.touch(&path);
                        self.record_write(&path, lifecycle);
                        // A version's row has landed in memory; the file follows it
                        // once the row is on the disk. Here rather than in the verb
                        // above, because a version arriving from a peer, or a delete
                        // arriving from an undo, has to reach the same place.
                        self.checkpoint(touched_versions).await;
                        self.log_local_write(&path, &value, lifecycle, &authorship).await;
                        self.broadcast_after_set(&path, value.clone());
                        if lifecycle != Lifecycle::Local {
                            if let Some(sync) = &self.sync {
                                sync.broadcast_synced(path, value, self.clock.clone(), authorship)
                                    .await;
                            }
                        }
                    }
                    // The caller waits for the disk; the actor does not. Everything
                    // this command enqueued is handed to a task that answers when it
                    // is durable, so the acknowledgement means exactly what it always
                    // meant while the next command is already being read. That is what
                    // gives the writer a group to commit: before this, the actor sat
                    // on each fsync and the writer never held more than one.
                    let receipts = std::mem::take(&mut self.pending_writes);
                    if receipts.is_empty() {
                        let _ = reply.send(result);
                    } else {
                        tokio::spawn(async move {
                            let mut outcome = result;
                            for receipt in receipts {
                                let landed = match receipt.await {
                                    Ok(landed) => landed,
                                    Err(_) => Err("the showfile writer stopped".to_string()),
                                };
                                // The first failure is the one reported; a later one is
                                // the same commit failing again.
                                if outcome.is_ok() {
                                    if let Err(e) = landed {
                                        outcome = Err(BackendError::Showfile(anyhow::anyhow!(e)));
                                    }
                                }
                            }
                            let _ = reply.send(outcome);
                        });
                    }
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
                    // A registered command names a method rather than a path, and what
                    // it goes on to write is its own business, so this is the case the
                    // fallback exists for. It is operator-paced and costs nothing.
                    self.touch_everything();
                    let _ = reply.send(result);
                }
                EngineCommand::SetSensedValue { fixture_id, key, value, reply } => {
                    let result = self.set_sensed_value(fixture_id, key, value).await;
                    if result.is_ok() {
                        self.touch_table("fixtures");
                    }
                    let _ = reply.send(result);
                }
                #[cfg(test)]
                EngineCommand::SampledParameters { reply } => {
                    let _ = reply.send(self.watch_samples.len());
                }
                EngineCommand::ApplyPeerOperation(op) => {
                    self.apply_peer_operation(op).await;
                }
                EngineCommand::ApplyOperationBatch(operations) => {
                    let count = operations.len();
                    for op in operations {
                        self.apply_peer_operation(op).await;
                    }
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
                    self.touch_everything();
                }
            }
        }
    }

    // ── What has moved ────────────────────────────────────────────────────────

    /// Record that the collection a path names has been written.
    ///
    /// The first segment of every entity path is the table, which is the same thing
    /// `broadcast_after_set` reads out of it. A path that names nothing — there are a
    /// few, and a registered command's is one — counts as everything having moved,
    /// which is what the single counter this replaced always assumed.
    fn touch(&mut self, path: &Path) {
        match path.first() {
            Some(PathSegment::Key(table)) => self.touch_table(table),
            _ => self.everything_version += 1,
        }
    }

    /// The same, for a caller that holds the table rather than a path.
    fn touch_table(&mut self, table: &str) {
        match self.collection_versions.get_mut(table) {
            Some(version) => *version += 1,
            None => {
                self.collection_versions.insert(table.to_string(), 1);
            }
        }
    }

    /// Something happened that nobody can attribute to a collection.
    fn touch_everything(&mut self) {
        self.everything_version += 1;
    }

    /// How much the collections a caller reads have moved, as one number.
    ///
    /// A sum rather than a set of counters, because every part of it only ever goes
    /// up: two readings differ exactly when one of the named collections was written
    /// between them, which is the whole of what a consumer is asking.
    fn version_of(&self, collections: &[&str]) -> u64 {
        collections
            .iter()
            .filter_map(|table| self.collection_versions.get(*table))
            .sum::<u64>()
            + self.everything_version
    }

    // ── Waking up ─────────────────────────────────────────────────────────────

    /// How long until the engine has something of its own to do.
    ///
    /// Zero while anything is outstanding, which is what makes a burst of writes turn
    /// into one pass: the loop polls the command channel first, so this only comes
    /// round when there is nothing left to drain.
    fn next_wake(&self) -> std::time::Duration {
        if self.outputs_dirty
            || self.version_of(PLAYBACK_COLLECTIONS) != self.playback_seen
            || self.version_of(OUTPUT_COLLECTIONS) != self.pushed_version
            || self.flows_dirty
            || !self.input_events.is_empty()
        {
            return std::time::Duration::ZERO;
        }

        // Far enough away to be a sleep and not a rate. A station with a show up, no
        // follow pending and nothing watched genuinely has nothing to do until a
        // command arrives, and this is how it says so.
        let mut wait = std::time::Duration::from_secs(3600);
        if let Some(due) = self.playback.next_deadline() {
            let now = pult_schema::types::sequence::now_ms();
            wait = wait.min(std::time::Duration::from_millis(due.saturating_sub(now)));
        }
        if !self.watched.is_empty() {
            wait = wait.min(WATCH_SAMPLE.saturating_sub(self.watch_sampled_at.elapsed()));
        }
        // A held pulse in a flow graph is a deadline like a follow cue, not a reason
        // to keep looking.
        if let Some(due) = self.flows.next_deadline() {
            wait = wait.min(due.saturating_duration_since(std::time::Instant::now()));
        }
        wait
    }

    // ── Playback ──────────────────────────────────────────────────────────────

    /// Work out what is driving the rig, and publish it.
    ///
    /// Run when the show changed or a follow came due, and at no other time. What it
    /// publishes are descriptions — the fades and the shapes, anchored in console time
    /// — never values: a fade in progress produces nothing here at all, because the
    /// connector drawing it and the browser showing it each work out what it is worth
    /// for the moment they are asking about.
    ///
    /// It lands with LOCAL lifecycle. Every station derives the same descriptions from
    /// the same replicated cue state, so fanning them out would be sending each console
    /// a slower copy of what it has already computed.
    async fn playback_pass(&mut self) -> Vec<Uuid> {
        let follow_due = self
            .playback
            .next_deadline()
            .is_some_and(|due| due <= pult_schema::types::sequence::now_ms());
        if !follow_due && self.version_of(PLAYBACK_COLLECTIONS) == self.playback_seen {
            return Vec::new();
        }
        self.playback_seen = self.version_of(PLAYBACK_COLLECTIONS);

        let sequences: Vec<pult_schema::types::sequence::Sequence> = self.read_collection("sequences");
        let programmer: Vec<pult_schema::types::programmer::ProgrammerValue> =
            self.read_collection("programmer_values");

        // Nothing is on, nothing is held, and nothing is remembered — so there is
        // nothing this pass could publish, and no reason to read the rig to find that
        // out. Which matters because a pass now runs on every change to the show
        // rather than at a rate: patching a rig into a dark house would otherwise walk
        // the whole of it once per fixture.
        if self.playback.is_idle()
            && programmer.is_empty()
            && sequences.iter().all(|s| s.active_cue_index.is_none())
        {
            return Vec::new();
        }

        let cues: Vec<pult_schema::types::cue::Cue> = self.read_collection("cues");
        let fixtures: Vec<pult_schema::types::fixture::Fixture> = self.read_collection("fixtures");
        // For one question — where a parameter rests when nothing is driving it. A
        // handful of rows beside thousands of fixtures.
        let fixture_types: Vec<FixtureType> = self.read_collection("fixture_types");
        let masters: Vec<pult_schema::types::speedmaster::SpeedMaster> =
            self.read_collection("speed_masters");
        let home_fade_ms = self.home_fade_ms();

        // Read once per pass rather than per effect: every station has to place this
        // pass at one instant, and asking the clock twice inside one would put two
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
            self.playback.pass(wall_ms, &view)
        };

        // A follower takes its cue positions from the leader, so only the leader
        // fires follow cues. Both ends still run their own fades.
        let is_follower = self.state.is_follower();
        let mut moved: Vec<Uuid> = Vec::new();

        for effect in effects {
            match effect {
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

        moved
    }

    /// Run a registered command and replicate the result.
    ///
    /// The engine's own way of pressing Go: everything a `Call` from a frontend does,
    /// minus the reply. Shared by follow cues and by triggers, so the two cannot
    /// drift into replicating differently.
    async fn run_synced_command(&mut self, path: Path, args: serde_json::Value) {
        if self.apply_set(path.clone(), args.clone(), Lifecycle::Synced).await.is_ok() {
            // Everything, for the reason a `Call` is everything: the path is the
            // command's, and what running it wrote is not in it.
            self.touch_everything();
            self.record_write(&path, Lifecycle::Synced);
            if let Some(sync) = &self.sync {
                sync.broadcast_synced(path, args, self.clock.clone(), Authorship::none()).await;
            }
        }
    }

    /// Sample the driven parameters some *Watch* node is looking at.
    ///
    /// Edge detection is the one thing that cannot be done from a function. A *Watch*
    /// node asks "when this parameter crosses a threshold", and answering that means
    /// having looked at it twice — so where everything else in the console stopped
    /// materialising values, this keeps a sample, and only of what is actually
    /// watched.
    ///
    /// Which is the whole difference from what it replaces. This used to be handed the
    /// values of every fixture the engine had just written, forty times a second, and
    /// throw away the ones nothing was looking at; a rig of two thousand paid for a
    /// graph watching one lamp. Now the set decides the work: nothing watched, nothing
    /// sampled, and one parameter watched costs one evaluation a sample.
    fn sample_watched(&mut self) {
        if self.watched.is_empty() {
            self.watch_samples.clear();
            return;
        }
        if self.watch_sampled_at.elapsed() < WATCH_SAMPLE {
            return;
        }
        self.watch_sampled_at = std::time::Instant::now();

        let now_ms = pult_schema::types::sequence::now_ms();
        let programmer: Vec<pult_schema::types::programmer::ProgrammerValue> =
            self.read_collection("programmer_values");
        let held = pult_schema::types::fixture::HeldByProgrammer::of(&programmer);

        let watching: Vec<(Uuid, String)> = self.watched.iter().cloned().collect();
        for at in watching {
            let Some(fixture) = self
                .state
                .entity("fixtures", at.0)
                .and_then(|row| serde_json::from_value::<Fixture>(row.clone()).ok())
            else {
                continue;
            };
            let fixture_type = self.fixture_type_of(&fixture);
            let driving = pult_schema::types::fixture::driving(
                &fixture,
                fixture_type.as_ref(),
                held.get(at.0, &at.1),
                &at.1,
            );
            // A parameter nothing is driving is one a device reports, and those queue
            // their own events as they arrive. Sampling home values would fire a graph
            // once at startup for every lamp in the rig.
            if !driving.is_driven() {
                self.watch_samples.remove(&at);
                continue;
            }
            let Some(current) = pult_render::value_at(&driving, now_ms) else { continue };
            let previous = self.watch_samples.insert(at.clone(), current.clone());
            if previous.as_ref() == Some(&current) {
                continue;
            }
            self.input_events.push(InputEvent {
                fixture_id: at.0,
                key: at.1,
                previous,
                current,
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
                // A drive, not a stored value: it takes the key from whatever fade or
                // effect had it and parks the parameter there, so a connector and a
                // browser see the same thing every other layer produces. A cue taken
                // afterwards takes the key back. Last writer wins, which is a design
                // question and not a bug to fix in passing.
                let key = crate::model::playback::parameter_key(&parameter);
                self.playback.set_parameter(
                    fixture_id,
                    key,
                    value,
                    pult_schema::types::sequence::now_ms(),
                );
                // The pass that publishes it has already run this time round the loop,
                // so ask for another rather than leaving the drive until something else
                // changes the show.
                self.playback_seen = self.playback_seen.wrapping_sub(1);
            }
        }
    }

    /// Write one replicated field and tell everyone who needs to know.
    async fn write_synced(&mut self, path: Path, value: serde_json::Value) {
        if self.apply_set(path.clone(), value.clone(), Lifecycle::Synced).await.is_err() {
            return;
        }
        self.touch(&path);
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

    /// Hand the output plugins the current picture of what is driving the rig.
    ///
    /// Sent when the **show** changes — a cue taken, a fade started, a fixture patched,
    /// an operator taking a fader — and not when a value moves, which after this change
    /// the engine never separately learns. A connector holds this and draws its own
    /// frames from it, so a three-second fade is one push rather than a hundred and
    /// twenty.
    ///
    /// The programmer goes with it because it is a layer over playback that only the
    /// show knows about, and a connector that could not see it would put a cue on the
    /// wire while an operator had hold of the fader.
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
        let programmer = self.read_collection("programmer_values");
        output.push(fixtures, fixture_types, programmer, moved);
    }

    /// Merge one key into what a fixture's devices have reported, and replicate the
    /// whole map.
    ///
    /// SYNCED rather than LOCAL: nothing else on the network can work this value out
    /// for itself, because it came off a wire attached to this node. Which is also why
    /// this is the one value the console still stores — it was told it rather than
    /// deciding it, so there is no function to evaluate.
    pub(crate) async fn set_sensed_value(
        &mut self,
        fixture_id: Uuid,
        key: String,
        value: serde_json::Value,
    ) -> Result<(), BackendError> {
        let path = entity_field_path("fixtures", fixture_id, "sensed_values");
        let mut values = self
            .state
            .entity("fixtures", fixture_id)
            .and_then(|entity| entity.get("sensed_values"))
            .cloned()
            .ok_or_else(|| BackendError::PathNotFound(path.clone()))?;
        let previous = values.get(&key).cloned();
        set_field(&mut values, &key, value.clone());

        // Queued for the next flow pass rather than read back from the state there, so
        // a press and a release between two passes are both seen.
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
        // The output side has to see it now: an input can arrive between two passes,
        // and a relay that follows a button should not wait for the next one.
        self.push_output(vec![fixture_id]).await;
        Ok(())
    }

    /// Log a write this node made itself.
    async fn log_local_write(
        &mut self,
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
            self.touch(&inverse.path);
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
        author: Option<Uuid>,
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
            // Save. The row is built here rather than in the browser, because two of
            // the four fields are the engine's own: the show's clock, and the moment
            // the station reached the write. Turned into an ordinary `__create`, so
            // history, the showfile and every peer see a create like any other — and
            // Ctrl-Z after an accidental Save deletes the row, which takes the file
            // with it.
            [PathSegment::Key(table), PathSegment::Key(verb)]
                if table == "versions" && verb == "__checkpoint" =>
            {
                let version = pult_schema::types::Version {
                    id: Uuid::new_v4(),
                    name: value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .filter(|name| !name.trim().is_empty()),
                    created_at: chrono::Utc::now(),
                    user_id: author,
                    automatic: value
                        .get("automatic")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    clock: self.clock.clone(),
                };
                let created = vec![
                    PathSegment::Key("versions".into()),
                    PathSegment::Key("__create".into()),
                ];
                let row = serde_json::to_value(&version).map_err(|e| {
                    BackendError::InvalidValue { path: path.clone(), reason: e.to_string() }
                })?;
                Ok(vec![(created, row)])
            }
            // Nothing else has a version to take.
            [.., PathSegment::Key(verb)] if verb == "__checkpoint" => {
                Err(BackendError::InvalidValue {
                    path: path.clone(),
                    reason: "only a show has versions; write to \
                             [\"versions\", \"__checkpoint\"]"
                        .into(),
                })
            }
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

        // Nothing held, so the programmer takes the key — starting from what playback
        // is showing at this instant, which since nothing stores that any more means
        // evaluating the fade or shape driving it, and falling through to where the
        // parameter rests when nothing ever has.
        let fixture_uuid: Uuid =
            fixture_id.parse().map_err(|_| bad("that fixtureId is not a uuid"))?;
        // The type is optional here, deliberately: a fixture patched to a type this
        // station has not received is still being driven by whatever is on it, and a
        // nudge should move that rather than refuse.
        let showing = self
            .fixture_row(fixture_uuid)
            .and_then(|fixture| {
                let fixture_type = self.fixture_type_of(&fixture);
                let driving = pult_schema::types::fixture::driving(
                    &fixture,
                    fixture_type.as_ref(),
                    None, // nothing is held: that is the branch above
                    &key,
                );
                pult_render::value_at(&driving, pult_schema::types::sequence::now_ms())
            })
            .or_else(|| self.home_value_of(fixture_uuid, &kind))
            .ok_or_else(|| bad("that fixture has no such parameter"))?;
        let next = showing.nudged(by).map_err(|reason| bad(&reason))?;

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
    /// One patched fixture, if this station has it.
    fn fixture_row(&self, fixture_id: Uuid) -> Option<Fixture> {
        self.state
            .get_by_path(&vec![
                PathSegment::Key("fixtures".into()),
                PathSegment::Id(fixture_id),
            ])
            .filter(|v| !v.is_null())
            .and_then(|row| serde_json::from_value(row).ok())
    }

    /// The type a fixture is patched as, where this station holds the row.
    ///
    /// Optional because it need not: a fixture patched to a type that has not
    /// replicated yet still has its own home overrides, and a house light should not
    /// go dark waiting for a row.
    fn fixture_type_of(&self, fixture: &Fixture) -> Option<FixtureType> {
        self.state
            .get_by_path(&vec![
                PathSegment::Key("fixture_types".into()),
                PathSegment::Id(fixture.fixture_type_id),
            ])
            .filter(|v| !v.is_null())
            .and_then(|row| serde_json::from_value(row).ok())
    }

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
    /// A verb rather than an ordinary write to `home_values` because working out what
    /// a parameter is putting out means holding the whole stack — the fade, the shape
    /// over it, the programmer over that — and evaluating it for this instant. A
    /// browser can do that; the command line and a plugin with no data access cannot,
    /// and the whole argument `__by` and `__home` made was that a caller able to act
    /// should not have to be a caller able to read the rig.
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

        // Evaluated at one instant for the whole fixture, so a mover caught mid-fade
        // keeps the pose it was actually in rather than a pan from one millisecond and
        // a tilt from the next.
        let now_ms = pult_schema::types::sequence::now_ms();
        let programmer: Vec<pult_schema::types::programmer::ProgrammerValue> =
            self.read_collection("programmer_values");
        let held = pult_schema::types::fixture::HeldByProgrammer::of(&programmer);

        let mut home = fixture.home_values.clone();
        let mut took = false;
        for kind in kinds {
            let key = parameter_key(&kind);
            let driving = pult_schema::types::fixture::driving(
                &fixture,
                Some(&fixture_type),
                held.get(fixture_uuid, &key),
                &key,
            );
            // Only where something is actually driving it. A parameter resting at its
            // home value has nothing to take: taking it would write back what is
            // already the answer, and put a change that changed nothing into history.
            let value = driving.is_driven().then(|| pult_render::value_at(&driving, now_ms)).flatten();
            let Some(value) = value else {
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
            home.insert(key, value);
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
        // Appended rather than rewritten. A create puts one id at the end, and asking
        // for the whole collection's order back is what made importing a rig cost
        // O(n²) — see `order::append`.
        self.append_order(meta, table, id).await;
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
    /// One created entity's place at the end of its collection.
    async fn append_order(&mut self, meta: &'static EntityMeta, table: &str, id: Uuid) {
        if meta.is_singleton || meta.upsert_one.is_none() {
            return;
        }
        if let Some(receipt) = self
            .writer
            .submit(vec![writer::WriteJob::OrderAppend { table: table.to_string(), id }])
            .await
        {
            self.pending_writes.push(receipt);
        }
    }

    async fn persist_order(&mut self, meta: &'static EntityMeta, table: &str) {
        if meta.is_singleton || meta.upsert_one.is_none() {
            return;
        }
        // Through the writer like everything else, so the order lands behind the
        // entity write it belongs to rather than racing it on another task.
        let ids = self.state.ids(table).to_vec();
        if let Some(receipt) = self
            .writer
            .submit(vec![writer::WriteJob::Order { table: table.to_string(), ids }])
            .await
        {
            self.pending_writes.push(receipt);
        }
    }

    async fn persist(
        &mut self,
        meta: &'static EntityMeta,
        entity: &serde_json::Value,
    ) -> Result<(), BackendError> {
        if meta.upsert_one.is_none() {
            return Ok(());
        }
        // Enqueued, not awaited. The receipt is collected and the caller is answered
        // when it lands, so an acknowledged write is still durable — but the actor
        // moves on, which is the only way a burst of writes can share one commit.
        //
        // The consequence to know: memory takes the value before the disk has
        // confirmed it, where this used to be the other way round. A disk that refuses
        // now means an error reaching the caller *after* the value is in the show
        // rather than instead of it. `persist_order` has always behaved that way, on
        // the grounds that losing a list's order is not a reason to reject the fixture
        // that was just patched.
        if let Some(receipt) =
            self.writer.submit(vec![writer::WriteJob::Upsert { meta, entity: entity.clone() }]).await
        {
            self.pending_writes.push(receipt);
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
        // Read before the write for the reason the local path reads it before the
        // write: a delete names the version it removes, and once the write has landed
        // the row it named is gone.
        let touched_versions = self.version_touched_by(&op.path, &op.value);
        if self.apply_set(op.path.clone(), op.value.clone(), op.lifecycle).await.is_ok() {
            // Here rather than at the call site, because only here is the path known —
            // and a peer's own `stations` row is a write this station must not read as
            // its rig having moved.
            self.touch(&op.path);
            self.log_operation(&op).await;
            self.path_clocks.insert(op.path.clone(), (op.clock, op.node_id));
            self.broadcast_after_set(&op.path, op.value);
            // A peer saved, so this station saves too — its own showfile, which is
            // the only thing it could honestly copy. That is what makes a version a
            // point in the *show's* history rather than in one console's.
            self.checkpoint(touched_versions).await;
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
    async fn log_operation(&mut self, op: &Operation) {
        if op.lifecycle == Lifecycle::Local {
            return;
        }
        // **Awaited, unlike every other write here**, and this is not an oversight.
        //
        // Entity state is read from memory, so a create that has not reached the disk
        // is still fully visible to the next `Get` — which is what makes deferring it
        // safe. The oplog is the exception: undo is a *query over it*, the History
        // panel reads it back, and a peer catching up is served `oplog::since` from
        // SQLite. Defer this one and a user's own Ctrl-Z races their write, which is
        // what seven tests said the moment it was tried.
        //
        // It costs little. This is one INSERT with no collection rewrite behind it,
        // and the expensive write on the create path — the order — is what moved.
        if let Err(e) =
            self.writer.write(vec![writer::WriteJob::Oplog { op: Box::new(op.clone()) }]).await
        {
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
    fn broadcast_after_set(&mut self, path: &Path, value: serde_json::Value) {
        // A create, a delete, or a field of a singleton: send the whole thing.
        //
        // A subscriber watching `show` is watching the show, and a pattern is matched
        // against the path a write names — so a field write to `show/name` reaches
        // nobody who asked for `show`. Collections already answer this by sending the
        // collection back; a singleton is the same problem with one row.
        //
        // Entities are deliberately not treated this way. A field write there is
        // `fixtures/<id>/live_fades` when a cue is taken over a rig of thousands, and
        // sending the whole rig each time is what `subscribeDeep` exists to avoid.
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
            // Marked, not sent. Flushed when the queue drains — see
            // `dirty_collections`, and `flush_collections` below.
            self.dirty_collections.insert(key.to_string());
        } else {
            let _ = self.broadcast.0.send((path.clone(), value));
        }
    }

    /// Send every collection that a create or a delete has made stale.
    ///
    /// Called when the command queue is empty, so a single write is broadcast at once
    /// and a burst of five thousand is broadcast once. A subscriber cannot tell the
    /// difference except in how much less it is sent.
    fn flush_collections(&mut self) {
        self.flushed_at = std::time::Instant::now();
        for key in std::mem::take(&mut self.dirty_collections) {
            let col_path = vec![PathSegment::Key(key)];
            if let Some(col_val) = self.state.get_by_path(&col_path) {
                let _ = self.broadcast.0.send((col_path, col_val));
            }
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
        self.seed_the_show().await;
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

    /// What this write is about to do to a saved version, read *before* it is
    /// applied.
    ///
    /// A delete names the version it is removing and nothing else, so by the time the
    /// write has landed the row is gone and there is nothing left to ask.
    fn version_touched_by(&self, path: &Path, value: &serde_json::Value) -> VersionTouched {
        if self.checkpointer.is_none() {
            return VersionTouched::Nothing;
        }
        match path.as_slice() {
            [PathSegment::Key(table), PathSegment::Key(action)]
                if table == "versions" && action == "__create" =>
            {
                match value.get("id").and_then(|v| v.as_str()).and_then(|id| Uuid::parse_str(id).ok())
                {
                    Some(id) => VersionTouched::Taken(id),
                    None => VersionTouched::Nothing,
                }
            }
            [PathSegment::Key(table), seg, PathSegment::Key(action)]
                if table == "versions" && action == "__delete" =>
            {
                match self.state.resolve_id("versions", seg) {
                    Some(id) => VersionTouched::Dropped(id),
                    None => VersionTouched::Nothing,
                }
            }
            _ => VersionTouched::Nothing,
        }
    }

    /// Copy the show, or throw the copy away.
    ///
    /// Reached by every kind of write, deliberately — an operator saving, a peer's
    /// copy of the same save, an undo deleting one — because what makes a snapshot
    /// right is the row existing on this station, and how the row got here is not
    /// this station's business.
    ///
    /// The copy waits on a **barrier** rather than on the version's own receipt: the
    /// writer's queue is ordered, so a barrier submitted now lands after the upsert
    /// submitted a moment ago, and the snapshot therefore contains its own row.
    /// Getting that backwards would make every restore quietly forget the point it
    /// restored to.
    async fn checkpoint(&mut self, touched: VersionTouched) {
        let Some(checkpointer) = self.checkpointer.clone() else { return };
        match touched {
            VersionTouched::Nothing => {}
            VersionTouched::Taken(id) => {
                if let Some(receipt) = self.writer.submit(vec![writer::WriteJob::Barrier]).await {
                    checkpointer.take(id, receipt);
                }
            }
            VersionTouched::Dropped(id) => checkpointer.forget(id),
        }
    }

    /// Give the show its row, if the file does not have one.
    ///
    /// Here rather than in the first browser to connect, which is where it used to be
    /// — the Show panel had an *Initialize Show* button, so a station running headless
    /// or reached first by a plugin had no show at all, and two browsers pressing it
    /// would have made two. What a new show starts with comes from the bundle's name
    /// and this station's preferences, both of which the engine can read and the page
    /// cannot.
    ///
    /// The preferences are read *once*, here, and are show data from then on: a second
    /// station opening the same show reads the same numbers rather than its own, which
    /// is the whole reason `history_depth` and `home_fade_ms` are in the show.
    ///
    /// Unattributed, like the default user beside it: nobody asked for it, and an
    /// operator pressing Ctrl-Z on a fresh show should reach their own first change.
    async fn seed_the_show(&mut self) {
        use pult_schema::types::show::Show;

        let path: Path = vec![PathSegment::Key("show".into())];
        if self.state.get_by_path(&path).is_some_and(|v| !v.is_null()) {
            return;
        }
        // No name to seed with is a station with no bundle open: an in-memory show
        // that nothing will ever read again, and giving it a row would be inventing a
        // show nobody asked for.
        let Some(name) = self.seed_name.clone() else { return };

        let prefs = crate::infra::preferences::load();
        let show = Show {
            id: uuid::Uuid::new_v4(),
            name,
            created_at: chrono::Utc::now(),
            editing_cue: None,
            history_depth: prefs.history_depth,
            home_fade_ms: prefs.home_fade_ms,
            haze_density: prefs.haze_density,
            haze_turbulence: prefs.haze_turbulence,
        };
        let Ok(value) = serde_json::to_value(&show) else { return };
        if let Err(e) = self.apply_set(path.clone(), value.clone(), Lifecycle::Persisted).await {
            warn!("[engine] could not seed the show: {e}");
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

pub mod admission;
pub mod undo;
pub mod writer;



#[cfg(test)]
mod tests;
