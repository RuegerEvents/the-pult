//! The WASM plugin runtime.
//!
//! Plugins reach a station two ways, and the difference matters.
//!
//! **From the show.** The `plugin_packages` roster is PERSISTED, so it
//! replicates and reloads; each row names a bundle by its sha256, and the bytes
//! live in the asset store beside the stage plans. A station reconciles what it
//! runs against that roster whenever it changes, fetching a bundle it lacks from
//! a peer and unpacking it into a cache keyed by digest. This is how one install
//! equips a whole rig.
//!
//! **From the disk.** Directories named by `--plugins`, watched and hot
//! reloaded. A plugin found on disk *overrides* a roster row with the same id,
//! on that station only — otherwise a developer editing a plugin on a station
//! joined to a session would silently be running the show's copy instead.
//!
//! The manager loads them in dependency order, keeps the LOCAL `plugins` state
//! telling every frontend what is running, routes calls to them, and reloads
//! one when its files change — a reload is a fresh instance, the way the
//! node-sim applies a config by stopping the node and starting a new one.
//!
//! Two rules from the runtime's first version still hold and shape everything
//! added here: **the manager never awaits guest code**, and it never awaits
//! anything slow inside its event loop. A bundle fetch is an HTTP request to
//! another station, so it runs on its own task and reports back as a message.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::plugin::{
        PluginInfo, PluginPermissions, PluginStatus, PluginsState, SurfaceInfo, WebPanelInfo,
    },
};
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

pub mod bundle;
pub mod cache;
pub mod manifest;

mod assets;
mod host_impls;
mod instance;
mod roster;
mod runtime;
mod station_store;
mod watcher;

pub use assets::routes as asset_routes;
pub use instance::InstanceMsg;

use crate::api::rpcs::LocalRpcDeps;
use crate::engine::{EngineHandle, UpdateBroadcast};
use instance::{InstanceDeps, InstanceHandle, InstanceMsg as Msg};
use manifest::PluginManifest;

/// After a crash, how long a plugin waits before a call may revive it. Stops a
/// guest that traps on every subscription tick from restarting at 40 Hz.
const RESTART_COOLDOWN: Duration = Duration::from_secs(10);

pub enum PluginCommand {
    /// Invoke one plugin's `rpc.handle`.
    Call {
        plugin: String,
        method: String,
        args: Value,
        ctx: Value,
        /// Plugins already on the call stack, for cycle refusal.
        chain: Vec<String>,
        reply: oneshot::Sender<Result<Value, String>>,
    },
    /// Something under this directory changed; load it fresh.
    Reload { dir: PathBuf },
    /// An instance finished its `init` and is answering. Reported by its own
    /// actor — the manager never waits on guest code, because guest code may
    /// call back into the manager.
    Ready { id: String },
    /// An instance died — in `init`, or mid-flight. Reported by its own actor.
    Failed { id: String, reason: String },
    /// Where a plugin's web assets live, for the HTTP route.
    AssetRoot {
        id: String,
        reply: oneshot::Sender<Option<PathBuf>>,
    },
    /// The show's plugin roster changed. Carries nothing: the roster is read
    /// fresh from the engine, so two changes arriving close together settle on
    /// the same answer rather than racing to apply their own snapshots.
    RosterChanged,
    /// A bundle fetch finished. Reported by the task that ran it, because
    /// fetching is an HTTP request to another station and the event loop must
    /// not be inside one.
    Fetched {
        sha256: String,
        /// `Err` names why, for the status the operator reads.
        result: Result<(), String>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct PluginsHandle(pub mpsc::Sender<PluginCommand>);

impl PluginsHandle {
    /// A call arriving from outside the runtime — the WebSocket, mostly. Args
    /// may carry the caller's context as `{ "payload": ..., "ctx": ... }`;
    /// anything else is payload with no context.
    pub async fn call(
        &self,
        plugin: String,
        method: String,
        args: Value,
    ) -> Result<Value, String> {
        let (args, ctx) = split_ctx(args);
        let (tx, rx) = oneshot::channel();
        self.0
            .send(PluginCommand::Call {
                plugin,
                method,
                args,
                ctx,
                chain: Vec::new(),
                reply: tx,
            })
            .await
            .map_err(|_| "the plugin runtime is not running".to_string())?;
        rx.await.map_err(|_| "the plugin went away mid-call".to_string())?
    }

    pub async fn asset_root(&self, id: String) -> Option<PathBuf> {
        let (tx, rx) = oneshot::channel();
        self.0.send(PluginCommand::AssetRoot { id, reply: tx }).await.ok()?;
        rx.await.ok().flatten()
    }
}

/// `{ payload, ctx }` split apart, or the whole value as payload.
fn split_ctx(args: Value) -> (Value, Value) {
    match args {
        Value::Object(mut map) if map.contains_key("payload") => {
            let payload = map.remove("payload").unwrap_or(Value::Null);
            let ctx = map.remove("ctx").unwrap_or(Value::Null);
            (payload, ctx)
        }
        other => (other, Value::Null),
    }
}

/// One plugin as the manager holds it.
struct Loaded {
    manifest: PluginManifest,
    info: PluginInfo,
    instance: Option<InstanceHandle>,
    /// When the last (re)start happened, for the crash cooldown.
    started_at: Instant,
    /// The digest it came from, or `None` for one loaded off a plugin
    /// directory. This is what the roster diff keys on.
    sha256: Option<String>,
    /// The composed configuration this instance was handed in `init`. Kept so
    /// the reconcile can notice when it no longer matches what the layers say.
    config: Value,
}

pub struct PluginManager {
    dirs: Vec<PathBuf>,
    engine: EngineHandle,
    /// The showfile, for reaching the asset store a bundle lives in.
    pool: Option<Arc<SqlitePool>>,
    broadcast: UpdateBroadcast,
    deps: InstanceDeps,
    plugins: BTreeMap<String, Loaded>,
    /// Digests currently being fetched, so two roster changes arriving during
    /// one download do not start a second one for the same bytes.
    fetching: BTreeSet<String>,
    rx: mpsc::Receiver<PluginCommand>,
    tx: mpsc::Sender<PluginCommand>,
}

impl PluginManager {
    pub fn new(
        engine: EngineHandle,
        broadcast: UpdateBroadcast,
        rpc_deps: LocalRpcDeps,
        dirs: Vec<PathBuf>,
        pool: Option<Arc<SqlitePool>>,
    ) -> (Self, PluginsHandle) {
        // Canonical from the start: `--plugins plugins` is a fine thing to
        // type, but the watcher reports absolute paths, and matching a change
        // back to its plugin only works when everything speaks the same one.
        let dirs = dirs
            .into_iter()
            .map(|dir| dir.canonicalize().unwrap_or(dir))
            .collect();
        let (tx, rx) = mpsc::channel(64);
        let deps = InstanceDeps {
            engine: engine.clone(),
            // Opened in `run`, which is async and is where the station actually
            // starts. Until then every store reads empty, which is also the
            // answer for a station whose file will not open at all.
            station_store: station_store::StationStore::none(),
            broadcast: broadcast.clone(),
            rpc_deps,
            manager: tx.clone(),
        };
        (
            Self {
                dirs,
                engine,
                pool,
                broadcast,
                deps,
                plugins: BTreeMap::new(),
                fetching: BTreeSet::new(),
                rx,
                tx: tx.clone(),
            },
            PluginsHandle(tx),
        )
    }

    pub async fn run(mut self) {
        // What the plugins on this machine remember. Opened before any of them
        // runs, and never a reason to fail: a station that cannot open it logs
        // once and carries on with the stores reading empty.
        self.deps.station_store = station_store::StationStore::open().await;

        // The roster is watched whether or not any directory is configured: a
        // station with no `--plugins` at all still runs what the show carries,
        // which is the ordinary case for everything that is not a dev checkout.
        let _roster_watch = self.watch_roster();

        let _watcher = if self.dirs.is_empty() {
            info!("[plugin] no plugin directories configured");
            None
        } else {
            Some(watcher::spawn(self.dirs.clone(), self.tx.clone()))
        };

        self.load_all().await;
        // What the show already asks for, before anything has changed.
        self.reconcile().await;
        self.event_loop().await;
    }

    /// Turn writes under `plugin_packages` into one message to ourselves.
    ///
    /// A create or a delete broadcasts the collection and a field write
    /// broadcasts the field, so matching the first segment catches both. The
    /// message carries nothing: the roster is read fresh when it is handled, so
    /// a burst of writes settles on one answer instead of racing.
    fn watch_roster(&self) -> tokio::task::JoinHandle<()> {
        let mut stream = self.broadcast.subscribe_all();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some((path, _)) = stream.next().await {
                let is_roster = matches!(
                    path.first(),
                    Some(PathSegment::Key(key)) if key == "plugin_packages"
                );
                if is_roster && tx.send(PluginCommand::RosterChanged).await.is_err() {
                    break;
                }
            }
        })
    }

    async fn event_loop(&mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                PluginCommand::Call { plugin, method, args, ctx, chain, reply } => {
                    self.route_call(plugin, method, args, ctx, chain, reply).await;
                }
                PluginCommand::Reload { dir } => {
                    self.reload_dir(dir).await;
                    // A directory plugin appearing, changing id, or going away
                    // changes which roster rows this station overrides — and a
                    // reload rebuilds the row from its manifest, so the flag
                    // has to be worked out again rather than carried over.
                    self.reconcile().await;
                    self.publish().await;
                }
                PluginCommand::Ready { id } => {
                    if let Some(loaded) = self.plugins.get_mut(&id) {
                        info!("[plugin:{id}] running");
                        loaded.info.status = PluginStatus::Running;
                        self.publish().await;
                    }
                }
                PluginCommand::Failed { id, reason } => {
                    if let Some(loaded) = self.plugins.get_mut(&id) {
                        warn!("[plugin:{id}] failed: {reason}");
                        loaded.instance = None;
                        loaded.info.status = PluginStatus::Failed(reason);
                        self.publish().await;
                    }
                }
                PluginCommand::AssetRoot { id, reply } => {
                    let root = self
                        .plugins
                        .get(&id)
                        .map(|loaded| loaded.manifest.dir.join("assets"));
                    let _ = reply.send(root);
                }
                PluginCommand::RosterChanged => {
                    self.reconcile().await;
                }
                PluginCommand::Fetched { sha256, result } => {
                    self.fetching.remove(&sha256);
                    match result {
                        Ok(()) => {
                            // The bytes are here, but the placeholder row that
                            // said "fetching" already carries this digest — so
                            // the diff would see a match and decide there was
                            // nothing to do. Drop the placeholders first; the
                            // reconcile then finds the plugin missing, and
                            // starts it from a store that now has the bundle.
                            self.plugins.retain(|_, loaded| {
                                !(loaded.sha256.as_deref() == Some(sha256.as_str())
                                    && loaded.instance.is_none()
                                    && matches!(loaded.info.status, PluginStatus::Fetching))
                            });
                            self.reconcile().await;
                        }
                        Err(reason) => {
                            // Name the failure against every plugin that was
                            // waiting on these bytes, since that is what the
                            // operator is looking at.
                            for loaded in self.plugins.values_mut() {
                                if loaded.sha256.as_deref() == Some(sha256.as_str())
                                    && matches!(loaded.info.status, PluginStatus::Fetching)
                                {
                                    warn!("[plugin:{}] {reason}", loaded.info.id);
                                    loaded.info.status = PluginStatus::Failed(reason.clone());
                                }
                            }
                            self.publish().await;
                        }
                    }
                }
                PluginCommand::Shutdown => break,
            }
        }
        // Give every guest its shutdown call before the station goes.
        for (_, loaded) in self.plugins.iter_mut() {
            if let Some(instance) = loaded.instance.take() {
                let (tx, rx) = oneshot::channel();
                if instance.send(Msg::Shutdown { reply: tx }) {
                    let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
                }
            }
        }
        info!("[plugin] stopped");
    }

    /// First load: everything found in the configured directories, in
    /// dependency order.
    async fn load_all(&mut self) {
        let mut manifests = Vec::new();
        for (dir, parsed) in discover(&self.dirs) {
            match parsed {
                Ok(manifest) => manifests.push(manifest),
                Err(reason) => {
                    // A directory that fails to parse still deserves a row the
                    // operator can see, named as well as it can be.
                    let id = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| dir.display().to_string());
                    warn!("[plugin] {}: {reason}", dir.display());
                    self.insert_failed(id, dir, reason);
                }
            }
        }
        let (ordered, failed) = manifest::load_order(manifests);
        for (id, reason) in failed {
            warn!("[plugin:{id}] not loaded: {reason}");
            self.insert_failed(id.clone(), PathBuf::new(), reason);
        }
        for m in ordered {
            let config = self.config_for(&m, &Value::Null);
            self.start_plugin(m, config);
        }
        self.publish().await;
    }

    fn insert_failed(&mut self, id: String, dir: PathBuf, reason: String) {
        let manifest_stub = PluginManifest {
            dir,
            plugin: manifest::PluginSection {
                id: id.clone(),
                name: id.clone(),
                version: String::new(),
                api: manifest::API_VERSION.to_string(),
                wasm: String::new(),
            },
            surfaces: Vec::new(),
            panels: Vec::new(),
            permissions: Default::default(),
            dependencies: Default::default(),
            stores: Vec::new(),
            config: Default::default(),
        };
        let info = info_for(&manifest_stub, PluginStatus::Failed(reason));
        self.plugins.insert(
            id,
            Loaded { manifest: manifest_stub, info, instance: None, started_at: Instant::now(), sha256: None, config: Value::Null },
        );
    }

    /// Start (or restart) one plugin whose manifest is already validated. The
    /// instance comes up on its own task; Running or Failed arrives later as a
    /// message. A dependency merely has to be *loading* — calls to it queue in
    /// its mailbox until its `init` is done, so the order the mailboxes were
    /// created in is the only sequencing anybody needs.
    fn start_plugin(&mut self, manifest: PluginManifest, config: Value) {
        let id = manifest.plugin.id.clone();
        // Where this one came from survives a restart. Without it, reviving a
        // crashed carried plugin would leave the row looking like a directory
        // plugin, and the next reconcile would take that as an override.
        let carried = self.plugins.get(&id).and_then(|loaded| loaded.sha256.clone());

        let unmet: Vec<&String> = manifest
            .dependencies
            .plugins
            .iter()
            .filter(|dep| {
                !self
                    .plugins
                    .get(dep.as_str())
                    .is_some_and(|p| !matches!(p.info.status, PluginStatus::Failed(_)))
            })
            .collect();
        if !unmet.is_empty() {
            let reason = format!("depends on {:?}, which is not running", unmet);
            let info = info_for(&manifest, PluginStatus::Failed(reason.clone()));
            warn!("[plugin:{id}] {reason}");
            self.plugins.insert(
                id,
                Loaded { manifest, info, instance: None, started_at: Instant::now(), sha256: carried, config: config.clone() },
            );
            return;
        }

        info!("[plugin:{id}] loading {}", manifest.wasm_path().display());
        let handle = instance::start(&manifest, config.clone(), self.deps.clone());
        let info = info_for(&manifest, PluginStatus::Loading);
        self.plugins.insert(
            id,
            Loaded { manifest, info, instance: Some(handle), started_at: Instant::now(), sha256: carried, config: config.clone() },
        );
    }

    async fn route_call(
        &mut self,
        plugin: String,
        method: String,
        args: Value,
        ctx: Value,
        chain: Vec<String>,
        reply: oneshot::Sender<Result<Value, String>>,
    ) {
        // A crashed plugin gets one fresh start per cooldown, on demand: the
        // operator's next command is the retry button.
        let needs_revival = self.plugins.get(&plugin).is_some_and(|loaded| {
            loaded.instance.is_none()
                && matches!(loaded.info.status, PluginStatus::Failed(_))
                && loaded.started_at.elapsed() >= RESTART_COOLDOWN
                && !loaded.manifest.plugin.wasm.is_empty()
        });
        if needs_revival {
            let held = self
                .plugins
                .get(&plugin)
                .map(|l| (l.manifest.clone(), l.config.clone()));
            if let Some((manifest, config)) = held {
                // The same configuration it had. Recomposing here would let a
                // crash quietly pick up an edit nobody asked to apply yet.
                self.start_plugin(manifest, config);
                self.publish().await;
            }
        }

        let Some(loaded) = self.plugins.get(&plugin) else {
            let _ = reply.send(Err(format!("no plugin called {plugin:?} is loaded")));
            return;
        };
        match &loaded.instance {
            Some(instance) => {
                // Forwarded, never awaited: the manager routing calls while a
                // guest is mid-call is what lets plugins call each other.
                if !instance.send(Msg::Call { method, args, ctx, chain, reply }) {
                    // The actor is gone but Failed has not arrived yet; the
                    // reply channel closing tells the caller enough.
                }
            }
            None => {
                let reason = match &loaded.info.status {
                    PluginStatus::Failed(reason) => format!("{plugin} is failed: {reason}"),
                    _ => format!("{plugin} is not running"),
                };
                let _ = reply.send(Err(reason));
            }
        }
    }

    /// A directory changed: stop whatever ran from it and start what is there
    /// now. Dependents keep running — they reach this plugin by id through the
    /// manager, not through a held reference.
    async fn reload_dir(&mut self, dir: PathBuf) {
        let old_id = self
            .plugins
            .iter()
            .find(|(_, loaded)| loaded.manifest.dir == dir)
            .map(|(id, _)| id.clone());
        if let Some(id) = &old_id {
            if let Some(loaded) = self.plugins.get_mut(id) {
                if let Some(instance) = loaded.instance.take() {
                    info!("[plugin:{id}] reloading");
                    let (tx, rx) = oneshot::channel();
                    if instance.send(Msg::Shutdown { reply: tx }) {
                        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
                    }
                }
            }
        }

        let manifest_path = dir.join("pult-plugin.toml");
        if old_id.is_none() && !manifest_path.is_file() {
            // Not a plugin and never was one: build artifacts and editor
            // droppings under a watched root land here, and are nobody's news.
            return;
        }
        let parsed = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("reading {}: {e}", manifest_path.display()))
            .and_then(|text| PluginManifest::parse(&dir, &text));
        match parsed {
            Ok(manifest) => {
                // The directory may now hold a different id; the old row goes.
                if let Some(id) = old_id {
                    if id != manifest.plugin.id {
                        self.plugins.remove(&id);
                    }
                }
                let config = self.config_for(&manifest, &Value::Null);
                self.start_plugin(manifest, config);
            }
            Err(reason) => {
                warn!("[plugin] {}: {reason}", dir.display());
                match old_id {
                    Some(id) => {
                        if let Some(loaded) = self.plugins.get_mut(&id) {
                            loaded.info.status = PluginStatus::Failed(reason);
                        }
                    }
                    None => {
                        let id = dir
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| dir.display().to_string());
                        self.insert_failed(id, dir, reason);
                    }
                }
            }
        }
    }

    /// Make what runs match what the show asks for.
    ///
    /// Read the roster, diff it against what is running, and apply. Idempotent
    /// on purpose: a burst of writes produces a burst of messages, and each one
    /// simply arrives at the same answer as the last.
    async fn reconcile(&mut self) {
        let roster = self.roster().await;

        // Ids this station loads from a directory. They are the developer's,
        // and the roster has no say about them.
        let overridden: Vec<String> = self
            .plugins
            .values()
            .filter(|loaded| loaded.sha256.is_none() && !loaded.manifest.plugin.wasm.is_empty())
            .map(|loaded| loaded.info.id.clone())
            .collect();

        let running: BTreeMap<String, roster::Running> = self
            .plugins
            .iter()
            .map(|(id, loaded)| {
                (
                    id.clone(),
                    roster::Running {
                        sha256: loaded.sha256.clone(),
                        config: loaded.config.clone(),
                    },
                )
            })
            .collect();

        // Mark the overrides *before* anything short-circuits. When the only
        // roster row is one this station overrides there is nothing to do and
        // no plan to make — which is exactly the case an operator most needs
        // told about, since otherwise a console quietly running something else
        // looks identical to the ones running what the show carries.
        let mut changed = false;
        for (id, loaded) in self.plugins.iter_mut() {
            let overridden_now =
                overridden.contains(id) && roster.iter().any(|p| &p.plugin_id == id);
            if loaded.info.overridden_by_disk != overridden_now {
                if overridden_now {
                    info!("[plugin:{id}] running the copy on disk, not the one the show carries");
                }
                loaded.info.overridden_by_disk = overridden_now;
                changed = true;
            }
        }

        // What each carried plugin's configuration composes to now. Only a
        // plugin whose manifest this station has read can be composed for; one
        // still being fetched has no defaults to merge under yet, and it will
        // be composed when it starts.
        let station_prefs = crate::infra::preferences::load();
        let desired_config: BTreeMap<String, Value> = roster
            .iter()
            .filter_map(|package| {
                let loaded = self.plugins.get(&package.plugin_id)?;
                if loaded.manifest.plugin.wasm.is_empty() {
                    return None;
                }
                Some((
                    package.plugin_id.clone(),
                    manifest::compose_config(
                        &loaded.manifest,
                        &package.config,
                        &station_prefs.plugin_config(&package.plugin_id),
                    ),
                ))
            })
            .collect();

        let actions = roster::plan(&roster, &running, &overridden, &desired_config);
        if actions.is_empty() {
            if changed {
                self.publish().await;
            }
            return;
        }

        for action in actions {
            match action {
                roster::Action::Publish { plugin_id } => {
                    // Only the label moved. Whatever is running keeps running —
                    // restarting a plugin because somebody fixed a typo in its
                    // name is exactly what task 9 taught outputs not to do.
                    if let (Some(package), Some(loaded)) =
                        (find(&roster, &plugin_id), self.plugins.get_mut(&plugin_id))
                    {
                        if loaded.info.name != package.name {
                            loaded.info.name = package.name.clone();
                            changed = true;
                        }
                    }
                }
                roster::Action::Stop { plugin_id } => {
                    self.stop(&plugin_id).await;
                    self.plugins.remove(&plugin_id);
                    changed = true;
                }
                roster::Action::Restart { plugin_id, sha256 } => {
                    // Same bytes, different configuration. A plugin is handed
                    // its config in `init` and never again, so the only way to
                    // change it is a fresh instance.
                    info!("[plugin:{plugin_id}] restarting: its configuration changed");
                    self.stop(&plugin_id).await;
                    self.plugins.remove(&plugin_id);
                    self.start_carried(&roster, &plugin_id, &sha256).await;
                    changed = true;
                }
                roster::Action::Replace { plugin_id, sha256 } => {
                    self.stop(&plugin_id).await;
                    self.plugins.remove(&plugin_id);
                    self.start_carried(&roster, &plugin_id, &sha256).await;
                    changed = true;
                }
                roster::Action::Start { plugin_id, sha256 } => {
                    self.start_carried(&roster, &plugin_id, &sha256).await;
                    changed = true;
                }
            }
        }

        if changed {
            self.publish().await;
        }
    }

    /// One plugin's configuration, composed from every layer that has an
    /// opinion about it.
    ///
    /// `show` is the roster row's configuration, or null for a plugin loaded
    /// from a directory — that one is deliberately not the show's, so the
    /// show's settings for an id it happens to share are not its business
    /// either. This station's preferences apply to both, since they are about
    /// this machine rather than about where the plugin came from.
    fn config_for(&self, manifest: &PluginManifest, show: &Value) -> Value {
        let station = crate::infra::preferences::load().plugin_config(&manifest.plugin.id);
        manifest::compose_config(manifest, show, &station)
    }

    /// The show's plugin roster, or nothing if it cannot be read.
    async fn roster(&self) -> Vec<pult_schema::types::plugin::PluginPackage> {
        let path = vec![PathSegment::Key("plugin_packages".into())];
        let Ok(value) = self.engine.get(path).await else { return Vec::new() };
        serde_json::from_value(value).unwrap_or_default()
    }

    /// Start a plugin the show carries, fetching its bundle if this station has
    /// never seen it.
    async fn start_carried(
        &mut self,
        roster: &[pult_schema::types::plugin::PluginPackage],
        plugin_id: &str,
        sha256: &str,
    ) {
        let Some(package) = find(roster, plugin_id) else { return };

        let Some(dir) = cache::dir_for(sha256) else {
            self.insert_carried_failure(
                package,
                format!("{sha256:?} is not a digest, so there is nowhere to unpack it"),
            );
            return;
        };

        if !cache::holds(sha256) {
            // Two ways to not have it: the bytes are not in the store yet, or
            // they are and nothing has unpacked them. Only the first is a fetch.
            match self.bundle_bytes(sha256).await {
                Some(bytes) => {
                    if let Err(e) = bundle::extract(&bytes, &dir) {
                        self.insert_carried_failure(package, format!("{e:#}"));
                        return;
                    }
                }
                None => {
                    self.begin_fetch(package, sha256);
                    return;
                }
            }
        }

        let manifest_path = dir.join(bundle::MANIFEST_NAME);
        let parsed = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("reading the unpacked manifest: {e}"))
            .and_then(|text| PluginManifest::parse(&dir, &text));
        match parsed {
            Ok(manifest) => {
                // The bundle's own id has to be the one the roster promised, or
                // a row could start a plugin that is not the plugin it names.
                if manifest.plugin.id != package.plugin_id {
                    self.insert_carried_failure(
                        package,
                        format!(
                            "the bundle at this digest is {:?}, not {:?}",
                            manifest.plugin.id, package.plugin_id
                        ),
                    );
                    return;
                }
                let config = self.config_for(&manifest, &package.config);
                self.start_plugin(manifest, config);
                if let Some(loaded) = self.plugins.get_mut(plugin_id) {
                    loaded.sha256 = Some(sha256.to_string());
                    // Published as well as held: the digest is how a panel tells
                    // a plugin the show carries from one somebody is editing.
                    loaded.info.sha256 = Some(sha256.to_string());
                }
            }
            Err(reason) => self.insert_carried_failure(package, reason),
        }
    }

    /// The bundle's bytes from the local store, without going to the network.
    async fn bundle_bytes(&self, sha256: &str) -> Option<Vec<u8>> {
        let pool = self.pool.as_ref()?;
        crate::infra::assets::get(pool, sha256).await.ok().flatten().map(|asset| asset.bytes)
    }

    /// Ask the other stations for a bundle, on a task of its own.
    ///
    /// Never in the event loop. This is an HTTP request with a ten-second
    /// timeout to a machine that may not answer, and the loop it would block is
    /// the one that routes every plugin call in the station.
    fn begin_fetch(&mut self, package: &pult_schema::types::plugin::PluginPackage, sha256: &str) {
        self.insert_carried_status(package, PluginStatus::Fetching);

        if !self.fetching.insert(sha256.to_string()) {
            return; // already on its way
        }
        let (Some(pool), sha) = (self.pool.clone(), sha256.to_string()) else {
            return;
        };
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        info!("[plugin:{}] fetching its bundle from a peer", package.plugin_id);
        tokio::spawn(async move {
            let peers = peer_addresses(&engine).await;
            let result = match crate::infra::assets::fetch_from_peers(&pool, &sha, &peers).await {
                // What comes back is hashed before it is stored, in the asset
                // store itself — so nothing here has to trust a peer's answer.
                Ok(Some(_)) => Ok(()),
                Ok(None) => Err(format!(
                    "no station in this session has the bundle {}",
                    &sha[..sha.len().min(12)]
                )),
                Err(e) => Err(format!("fetching the bundle failed: {e}")),
            };
            let _ = tx.send(PluginCommand::Fetched { sha256: sha, result }).await;
        });
    }

    fn insert_carried_failure(
        &mut self,
        package: &pult_schema::types::plugin::PluginPackage,
        reason: String,
    ) {
        warn!("[plugin:{}] {reason}", package.plugin_id);
        self.insert_carried_status(package, PluginStatus::Failed(reason));
    }

    /// A row for a carried plugin that is not running, so the panel has
    /// something to show while it is fetched — or something to explain if it
    /// never will be.
    fn insert_carried_status(
        &mut self,
        package: &pult_schema::types::plugin::PluginPackage,
        status: PluginStatus,
    ) {
        let info = PluginInfo {
            id: package.plugin_id.clone(),
            name: package.name.clone(),
            version: package.version.clone(),
            status,
            surfaces: Vec::new(),
            panels: Vec::new(),
            sha256: Some(package.sha256.clone()),
            overridden_by_disk: false,
            permissions: Default::default(),
        };
        let manifest = stub_manifest(&package.plugin_id);
        self.plugins.insert(
            package.plugin_id.clone(),
            Loaded {
                manifest,
                info,
                instance: None,
                started_at: Instant::now(),
                sha256: Some(package.sha256.clone()),
                config: Value::Null,
            },
        );
    }

    /// Give a plugin's guest its `shutdown` and drop the instance.
    async fn stop(&mut self, plugin_id: &str) {
        if let Some(loaded) = self.plugins.get_mut(plugin_id) {
            if let Some(instance) = loaded.instance.take() {
                info!("[plugin:{plugin_id}] stopping");
                let (tx, rx) = oneshot::channel();
                if instance.send(Msg::Shutdown { reply: tx }) {
                    let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
                }
            }
        }
    }

    /// Tell every frontend what this station is running.
    async fn publish(&self) {
        let state = PluginsState {
            plugins: self.plugins.values().map(|loaded| loaded.info.clone()).collect(),
        };
        let value = serde_json::to_value(&state).unwrap_or_default();
        let path = vec![PathSegment::Key("plugins".into())];
        if let Err(e) = self.engine.set(path, Lifecycle::Local, value).await {
            warn!("[plugin] could not publish state: {e}");
        }
    }
}

/// The roster row for one plugin id.
fn find<'a>(
    roster: &'a [pult_schema::types::plugin::PluginPackage],
    plugin_id: &str,
) -> Option<&'a pult_schema::types::plugin::PluginPackage> {
    roster.iter().find(|p| p.plugin_id == plugin_id)
}

/// Where the other stations serve HTTP, which is where a bundle can be had.
async fn peer_addresses(engine: &EngineHandle) -> Vec<String> {
    let path = vec![PathSegment::Key("stations".into())];
    let Ok(value) = engine.get(path).await else { return Vec::new() };
    let Ok(stations) = serde_json::from_value::<Vec<pult_schema::types::station::Station>>(value)
    else {
        return Vec::new();
    };
    stations.into_iter().map(|station| station.http_addr).collect()
}

/// Enough of a manifest to carry a name and a failure, for a plugin whose real
/// one this station cannot read yet.
fn stub_manifest(id: &str) -> PluginManifest {
    PluginManifest {
        dir: PathBuf::new(),
        plugin: manifest::PluginSection {
            id: id.to_string(),
            name: id.to_string(),
            version: String::new(),
            api: String::new(),
            wasm: String::new(),
        },
        surfaces: Vec::new(),
        panels: Vec::new(),
        permissions: Default::default(),
        dependencies: Default::default(),
        stores: Vec::new(),
        config: Default::default(),
    }
}

fn info_for(manifest: &PluginManifest, status: PluginStatus) -> PluginInfo {
    PluginInfo {
        id: manifest.plugin.id.clone(),
        name: manifest.plugin.name.clone(),
        version: manifest.plugin.version.clone(),
        status,
        surfaces: manifest
            .surfaces
            .iter()
            .map(|s| SurfaceInfo {
                id: s.id.clone(),
                kind: s.kind.as_str().to_string(),
                title: s.title.clone(),
            })
            .collect(),
        panels: manifest
            .panels
            .iter()
            .map(|p| WebPanelInfo {
                id: p.id.clone(),
                title: p.title.clone(),
                element: p.element.clone(),
                script: p.script.clone(),
                fills: p.fills,
            })
            .collect(),
        // Filled in by the reconcile once it knows where this one came from.
        sha256: None,
        overridden_by_disk: false,
        permissions: PluginPermissions {
            data: match manifest.permissions.data {
                manifest::DataPermission::None => "none",
                manifest::DataPermission::Read => "read",
                manifest::DataPermission::ReadWrite => "read-write",
            }
            .to_string(),
            commands: manifest.permissions.commands,
            http: manifest.permissions.http.clone(),
            env: manifest.permissions.env.clone(),
        },
    }
}

/// Every plugin directory under the configured roots. A root that itself holds
/// a `pult-plugin.toml` is a single plugin; otherwise each child directory
/// with one is.
fn discover(roots: &[PathBuf]) -> Vec<(PathBuf, Result<PluginManifest, String>)> {
    let mut found = Vec::new();
    for root in roots {
        if root.join("pult-plugin.toml").is_file() {
            found.push((root.clone(), parse_dir(root)));
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            warn!("[plugin] cannot read {}", root.display());
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if dir.is_dir() && dir.join("pult-plugin.toml").is_file() {
                found.push((dir.clone(), parse_dir(&dir)));
            }
        }
    }
    found
}

fn parse_dir(dir: &std::path::Path) -> Result<PluginManifest, String> {
    let path = dir.join("pult-plugin.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading manifest: {e}"))?;
    PluginManifest::parse(dir, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_finds_plugins_one_level_down_and_at_the_root() {
        let root = tempdir();
        std::fs::create_dir_all(root.join("one")).unwrap();
        std::fs::write(
            root.join("one/pult-plugin.toml"),
            "[plugin]\nid = \"one\"\nname = \"One\"\nversion = \"0\"\napi = \"1.0\"\nwasm = \"one.wasm\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("not-a-plugin")).unwrap();

        let found = discover(&[root.clone()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.as_ref().unwrap().plugin.id, "one");

        // Pointing straight at a plugin directory works too.
        let direct = discover(&[root.join("one")]);
        assert_eq!(direct.len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pult-plugin-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
