//! One running plugin: a `Store`, its component, and the mailbox that feeds it.
//!
//! Guests are single-threaded, so every call into one — an RPC, a subscription
//! delivery, a shutdown — goes through this actor and runs alone. Every call
//! gets an epoch deadline first; a guest that blows through it traps, the trap
//! is reported to the manager, and the actor dies rather than limps.

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;
use wasmtime::component::{HasSelf, Linker, ResourceTable};
use wasmtime::Store;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi_http::WasiHttpCtx;

use crate::api::rpcs::LocalRpcDeps;
use crate::engine::{EngineHandle, UpdateBroadcast};

use super::host_impls::{AllowlistHooks, PluginCtx};
use super::manifest::PluginManifest;
use super::runtime::{self, Plugin, CALL_DEADLINE_TICKS};
use super::PluginCommand;

/// A second's worth of ticks: what shutdown gets, which is a courtesy, not a
/// negotiation.
const SHUTDOWN_DEADLINE_TICKS: u64 = 100;

pub enum InstanceMsg {
    Call {
        method: String,
        args: Value,
        ctx: Value,
        chain: Vec<String>,
        reply: oneshot::Sender<Result<Value, String>>,
    },
    Update {
        token: u64,
        path: pult_schema::path::Path,
        value: Value,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct InstanceHandle(pub mpsc::UnboundedSender<InstanceMsg>);

impl InstanceHandle {
    /// Unbounded on purpose: the sender is sometimes a guest call on another
    /// plugin's actor, and an actor blocked on an actor is how deadlocks start.
    pub fn send(&self, msg: InstanceMsg) -> bool {
        self.0.send(msg).is_ok()
    }
}

/// What a plugin instance needs from the station around it.
#[derive(Clone)]
pub struct InstanceDeps {
    pub engine: EngineHandle,
    /// This machine's plugin data, opened once for the station and shared by
    /// every instance — it is one file, and one pool is enough for all of them.
    pub station_store: super::station_store::StationStore,
    pub broadcast: UpdateBroadcast,
    pub rpc_deps: LocalRpcDeps,
    pub manager: mpsc::Sender<PluginCommand>,
}

/// Bring one plugin up on its own task and hand back its mailbox at once.
///
/// Compilation, instantiation and `init` all happen inside the task, which is
/// the load-bearing part: `init` runs guest code, guest code may call other
/// plugins through the manager, and a manager that awaited `init` would be a
/// deadlock with one dependency edge. Readiness travels back as
/// [`PluginCommand::Ready`] or [`PluginCommand::Failed`]; calls sent before
/// then simply queue in the mailbox and run after `init`.
pub fn start(manifest: &PluginManifest, config: serde_json::Value, deps: InstanceDeps) -> InstanceHandle {
    let id = manifest.plugin.id.clone();
    let manifest = manifest.clone();
    let (tx, rx) = mpsc::unbounded_channel::<InstanceMsg>();
    let self_tx = tx.clone();
    let manager = deps.manager.clone();

    tokio::spawn(async move {
        match set_up(&manifest, config, deps, self_tx).await {
            Ok((store, plugin)) => {
                let _ = manager.send(PluginCommand::Ready { id: id.clone() }).await;
                run(id, store, plugin, rx, manager).await;
            }
            Err(reason) => {
                // Dropping rx here closes the mailbox: anything queued or sent
                // later gets its reply channel dropped, which callers read as
                // the plugin being gone.
                let _ = manager.send(PluginCommand::Failed { id, reason }).await;
            }
        }
    });

    InstanceHandle(tx)
}

async fn set_up(
    manifest: &PluginManifest,
    // Already composed from every layer that has an opinion: the manifest's
    // own defaults, the show's roster row, and this station's preferences.
    config: serde_json::Value,
    deps: InstanceDeps,
    self_tx: mpsc::UnboundedSender<InstanceMsg>,
) -> Result<(Store<PluginCtx>, Plugin), String> {
    let id = manifest.plugin.id.clone();
    let component = runtime::load_component(manifest.wasm_path())
        .await
        .map_err(|e| format!("compiling {}: {e}", manifest.plugin.wasm))?;

    let engine = runtime::engine();
    let mut linker = Linker::<PluginCtx>::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|e| e.to_string())?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
        .map_err(|e| e.to_string())?;
    Plugin::add_to_linker::<PluginCtx, HasSelf<PluginCtx>>(&mut linker, |ctx| ctx)
        .map_err(|e| e.to_string())?;

    let mut wasi = WasiCtxBuilder::new();
    // The guest's stdout is the station's log; a plugin's println is a debug
    // aid, not a protocol.
    wasi.inherit_stdout().inherit_stderr();
    for name in &manifest.permissions.env {
        if let Ok(value) = std::env::var(name) {
            wasi.env(name, &value);
        }
    }

    let ctx = PluginCtx {
        plugin_id: id.clone(),
        permissions: manifest.permissions.clone(),
        deps: manifest.dependencies.plugins.clone(),
        stores: manifest.stores.clone(),
        station_store: deps.station_store.clone(),
        engine: deps.engine,
        broadcast: deps.broadcast,
        rpc_deps: deps.rpc_deps,
        manager: deps.manager.clone(),
        self_tx,
        chain: Vec::new(),
        ctx: Value::Null,
        user: None,
        gesture: None,
        next_token: 0,
        subs: Default::default(),
        wasi: wasi.build(),
        table: ResourceTable::new(),
        http: WasiHttpCtx::new(),
        http_hooks: AllowlistHooks {
            plugin_id: id.clone(),
            allow: manifest.permissions.http.clone(),
        },
    };

    let mut store = Store::new(engine, ctx);
    store.set_epoch_deadline(CALL_DEADLINE_TICKS);
    let plugin = Plugin::instantiate_async(&mut store, &component, &linker)
        .await
        .map_err(|e| format!("instantiating: {e}"))?;

    let config_text = serde_json::to_string(&config).unwrap_or_else(|_| "null".into());
    store.set_epoch_deadline(CALL_DEADLINE_TICKS);
    plugin
        .pult_plugin_lifecycle()
        .call_init(&mut store, &config_text)
        .await
        .map_err(|e| format!("init trapped: {e}"))?
        .map_err(|e| format!("init failed: {e}"))?;

    Ok((store, plugin))
}

async fn run(
    id: String,
    mut store: Store<PluginCtx>,
    plugin: Plugin,
    mut rx: mpsc::UnboundedReceiver<InstanceMsg>,
    manager: mpsc::Sender<PluginCommand>,
) {
    let failure: Option<String> = loop {
            let Some(msg) = rx.recv().await else { break None };
            match msg {
                InstanceMsg::Call { method, args, ctx, chain, reply } => {
                    // The call's context lives in the store for its duration:
                    // that is what lets `data.set` attribute writes and
                    // `call-plugin` carry the context on without the guest
                    // handling either.
                    {
                        let data = store.data_mut();
                        data.chain = chain;
                        data.user = ctx
                            .get("userId")
                            .and_then(Value::as_str)
                            .and_then(|s| Uuid::parse_str(s).ok());
                        data.gesture = Some(Uuid::new_v4());
                        data.ctx = ctx.clone();
                    }
                    store.set_epoch_deadline(CALL_DEADLINE_TICKS);
                    let args_text = serde_json::to_string(&args).unwrap_or_else(|_| "null".into());
                    let ctx_text = serde_json::to_string(&ctx).unwrap_or_else(|_| "null".into());
                    let outcome = plugin
                        .pult_plugin_rpc()
                        .call_handle(&mut store, &method, &args_text, &ctx_text)
                        .await;
                    {
                        let data = store.data_mut();
                        data.chain = Vec::new();
                        data.user = None;
                        data.gesture = None;
                        data.ctx = Value::Null;
                    }
                    match outcome {
                        Ok(result) => {
                            let result = result.and_then(|text| {
                                serde_json::from_str(&text)
                                    .map_err(|e| format!("plugin returned invalid JSON: {e}"))
                            });
                            let _ = reply.send(result);
                        }
                        Err(trap) => {
                            let _ = reply.send(Err(format!("plugin trapped: {trap}")));
                            break Some(format!("trapped in {method}: {trap}"));
                        }
                    }
                }
                InstanceMsg::Update { token, path, value } => {
                    let segments: Vec<String> = path.iter().map(|s| s.to_string()).collect();
                    let value_text = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
                    store.set_epoch_deadline(CALL_DEADLINE_TICKS);
                    if let Err(trap) = plugin
                        .pult_plugin_lifecycle()
                        .call_on_update(&mut store, token, &segments, &value_text)
                        .await
                    {
                        break Some(format!("trapped in on-update: {trap}"));
                    }
                }
                InstanceMsg::Shutdown { reply } => {
                    store.set_epoch_deadline(SHUTDOWN_DEADLINE_TICKS);
                    if let Err(e) = plugin.pult_plugin_lifecycle().call_shutdown(&mut store).await {
                        debug!("[plugin:{id}] shutdown trapped: {e}");
                    }
                    let _ = reply.send(());
                    break None;
                }
            }
    };
    if let Some(reason) = failure {
        warn!("[plugin:{id}] {reason}");
        let _ = manager.send(PluginCommand::Failed { id, reason }).await;
    } else {
        debug!("[plugin:{id}] instance stopped");
    }
}
