//! The WASM plugin runtime.
//!
//! Which plugins exist is a fact about this station's disk: directories named
//! by `--plugins`, each holding a `pult-plugin.toml` and a component. The
//! manager loads them in dependency order, keeps the LOCAL `plugins` state
//! telling every frontend what is running, routes calls to them, and reloads
//! one when its files change — a reload is a fresh instance, the way the
//! node-sim applies a config by stopping the node and starting a new one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::plugin::{PluginInfo, PluginStatus, PluginsState, SurfaceInfo, WebPanelInfo},
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

pub mod bundle;
pub mod manifest;

mod assets;
mod host_impls;
mod instance;
mod runtime;
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
}

pub struct PluginManager {
    dirs: Vec<PathBuf>,
    engine: EngineHandle,
    deps: InstanceDeps,
    plugins: BTreeMap<String, Loaded>,
    rx: mpsc::Receiver<PluginCommand>,
    tx: mpsc::Sender<PluginCommand>,
}

impl PluginManager {
    pub fn new(
        engine: EngineHandle,
        broadcast: UpdateBroadcast,
        rpc_deps: LocalRpcDeps,
        dirs: Vec<PathBuf>,
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
            broadcast,
            rpc_deps,
            manager: tx.clone(),
        };
        (
            Self {
                dirs,
                engine,
                deps,
                plugins: BTreeMap::new(),
                rx,
                tx: tx.clone(),
            },
            PluginsHandle(tx),
        )
    }

    pub async fn run(mut self) {
        if self.dirs.is_empty() {
            // Nothing to do, but stay alive: the handle is in AppState and a
            // call should answer "no such plugin", not "runtime gone".
            info!("[plugin] no plugin directories configured");
        } else {
            let _watcher = watcher::spawn(self.dirs.clone(), self.tx.clone());
            self.load_all().await;
            // The watcher thread lives as long as the manager loop below.
            self.event_loop().await;
            return;
        }
        self.event_loop().await;
    }

    async fn event_loop(&mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                PluginCommand::Call { plugin, method, args, ctx, chain, reply } => {
                    self.route_call(plugin, method, args, ctx, chain, reply).await;
                }
                PluginCommand::Reload { dir } => {
                    self.reload_dir(dir).await;
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
            self.start_plugin(m);
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
                api: manifest::API_VERSION.into(),
                wasm: String::new(),
            },
            surfaces: Vec::new(),
            panels: Vec::new(),
            permissions: Default::default(),
            dependencies: Default::default(),
            config: Default::default(),
        };
        let info = info_for(&manifest_stub, PluginStatus::Failed(reason));
        self.plugins.insert(
            id,
            Loaded { manifest: manifest_stub, info, instance: None, started_at: Instant::now() },
        );
    }

    /// Start (or restart) one plugin whose manifest is already validated. The
    /// instance comes up on its own task; Running or Failed arrives later as a
    /// message. A dependency merely has to be *loading* — calls to it queue in
    /// its mailbox until its `init` is done, so the order the mailboxes were
    /// created in is the only sequencing anybody needs.
    fn start_plugin(&mut self, manifest: PluginManifest) {
        let id = manifest.plugin.id.clone();

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
                Loaded { manifest, info, instance: None, started_at: Instant::now() },
            );
            return;
        }

        info!("[plugin:{id}] loading {}", manifest.wasm_path().display());
        let handle = instance::start(&manifest, self.deps.clone());
        let info = info_for(&manifest, PluginStatus::Loading);
        self.plugins.insert(
            id,
            Loaded { manifest, info, instance: Some(handle), started_at: Instant::now() },
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
            let manifest = self.plugins.get(&plugin).map(|l| l.manifest.clone());
            if let Some(manifest) = manifest {
                self.start_plugin(manifest);
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
                self.start_plugin(manifest);
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
            "[plugin]\nid = \"one\"\nname = \"One\"\nversion = \"0\"\napi = \"0.1\"\nwasm = \"one.wasm\"\n",
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
