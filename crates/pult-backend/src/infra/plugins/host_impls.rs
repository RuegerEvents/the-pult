//! The host side of the WIT contract: what a guest's imports actually do.
//!
//! Everything a plugin may touch flows through here, so this is also where the
//! manifest's permissions are enforced — not in the guest, which is untrusted,
//! and not in the manager, which never sees individual calls.

use std::collections::HashMap;

use pult_schema::{
    commands::CommandRegistration,
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    registry::EntityMeta,
    types::PluginDatum,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{
    WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView,
};

use crate::{
    api::rpcs::{self, LocalRpcDeps},
    engine::{EngineHandle, UpdateBroadcast},
    infra::plugins::manifest::{DataPermission, Permissions, StoreScope, StoreSection},
    infra::plugins::station_store::StationStore,
    infra::plugins::runtime::pult::plugin::{data, introspection, logging, peers, store, types},
    infra::plugins::{InstanceMsg, PluginCommand},
};

/// How deep a chain of plugins calling plugins may get. Cycles are refused
/// outright; this catches an honest chain that is really a loop in disguise.
const MAX_CALL_DEPTH: usize = 8;

/// The `T` in this plugin's `Store<T>`: everything its host imports reach.
pub struct PluginCtx {
    pub plugin_id: String,
    pub permissions: Permissions,
    /// Plugins this one may `call-plugin` into, from the manifest.
    pub deps: Vec<String>,
    /// The stores this plugin declared, as the manifest spelled them.
    ///
    /// Held here rather than looked up per call so that the answer to "may this
    /// guest touch this store, and where does it live" comes from the manifest
    /// the instance was started with — never from the call. A guest can spell
    /// no name that reaches another plugin's data, because the plugin id in the
    /// key is this one's and is not a parameter.
    pub stores: Vec<StoreSection>,
    /// This machine's half of that: persistent, never replicated, never in a
    /// showfile.
    pub station_store: StationStore,
    pub engine: EngineHandle,
    pub broadcast: UpdateBroadcast,
    pub rpc_deps: LocalRpcDeps,
    /// Back to the manager, for calls into other plugins.
    pub manager: mpsc::Sender<PluginCommand>,
    /// Into this plugin's own mailbox, for subscription deliveries.
    pub self_tx: mpsc::UnboundedSender<InstanceMsg>,

    /// Who is above us in the call currently being handled — set by the
    /// instance actor around each guest call, so `call-plugin` can refuse a
    /// cycle without asking anyone.
    pub chain: Vec<String>,
    /// The caller context of the call being handled; travels along on
    /// `call-plugin` so a delegated command still knows whose selection it is.
    pub ctx: Value,
    /// Who asked, parsed once from `ctx.userId`, so the plugin's writes are
    /// attributed and the Oops key covers them.
    pub user: Option<Uuid>,
    /// One gesture per inbound call: a command that fans out over a selection
    /// undoes as one act, the way a drag does.
    pub gesture: Option<Uuid>,

    pub next_token: u64,
    pub subs: HashMap<u64, tokio::task::AbortHandle>,

    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub http: WasiHttpCtx,
    pub http_hooks: AllowlistHooks,
}

impl Drop for PluginCtx {
    fn drop(&mut self) {
        for (_, task) in self.subs.drain() {
            task.abort();
        }
    }
}

impl WasiView for PluginCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl WasiHttpView for PluginCtx {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            hooks: &mut self.http_hooks,
            table: &mut self.table,
            ctx: &mut self.http,
        }
    }
}

// ── Outbound HTTP, gated ──────────────────────────────────────────────────────

/// Hooks whose defaults are the crate's own behaviour.
struct DefaultHooks;
impl WasiHttpHooks for DefaultHooks {}

/// The manifest's `http` allowlist, applied to every outgoing request. An
/// entry matches its host, with or without a port (`"localhost"` allows
/// `localhost:11434` too; `"localhost:11434"` allows only that).
pub struct AllowlistHooks {
    pub plugin_id: String,
    pub allow: Vec<String>,
}

impl AllowlistHooks {
    fn permits(&self, host: &str, port: Option<u16>) -> bool {
        self.allow.iter().any(|entry| {
            entry == host
                || port.is_some_and(|p| *entry == format!("{host}:{p}"))
        })
    }
}

impl WasiHttpHooks for AllowlistHooks {
    fn send_request(
        &mut self,
        request: http::Request<wasmtime_wasi_http::WasiBody>,
        options: Option<wasmtime_wasi_http::RequestOptions>,
        fut: Box<dyn std::future::Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
    ) -> Box<
        dyn std::future::Future<
                Output = wasmtime_wasi_http::Result<(
                    http::Response<wasmtime_wasi_http::WasiBody>,
                    Box<dyn std::future::Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
                )>,
            > + Send,
    > {
        let allowed = request
            .uri()
            .host()
            .is_some_and(|host| self.permits(host, request.uri().port_u16()));
        if !allowed {
            tracing::warn!(
                "[plugin:{}] refused outbound HTTP to {} — not in the manifest's permissions.http",
                self.plugin_id,
                request.uri()
            );
            return Box::new(async { Err(wasmtime_wasi_http::Error::HttpRequestDenied) });
        }
        DefaultHooks.send_request(request, options, fut)
    }
}

// ── Path plumbing ─────────────────────────────────────────────────────────────

/// A path segment as a guest spells it. The same reading order as the wire's
/// untagged serde: a uuid is an id, digits are an index, anything else a key.
pub fn parse_segment(s: &str) -> PathSegment {
    if let Ok(id) = Uuid::parse_str(s) {
        return PathSegment::Id(id);
    }
    if let Ok(n) = s.parse::<usize>() {
        return PathSegment::Index(n);
    }
    PathSegment::Key(s.to_string())
}

pub fn parse_path(segments: &[String]) -> Path {
    segments.iter().map(|s| parse_segment(s)).collect()
}

fn to_json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

// ── Host trait impls ──────────────────────────────────────────────────────────

impl types::Host for PluginCtx {}

impl data::Host for PluginCtx {
    async fn set(
        &mut self,
        path: Vec<String>,
        value: String,
    ) -> wasmtime::Result<Result<(), String>> {
        if self.permissions.data != DataPermission::ReadWrite {
            return Ok(Err("this plugin's manifest does not grant data = \"read-write\"".into()));
        }
        let path = parse_path(&path);
        let value: Value = match serde_json::from_str(&value) {
            Ok(v) => v,
            Err(e) => return Ok(Err(format!("value is not JSON: {e}"))),
        };
        let lifecycle = pult_schema::registry::path_lifecycle(&path);
        let result = match self.user {
            // Attributed, and gathered into this call's gesture, so whatever a
            // command wrote undoes as one act for the operator who asked.
            Some(user) => self.engine.set_as(user, self.gesture, path, lifecycle, value).await,
            None => self.engine.set(path, lifecycle, value).await,
        };
        Ok(result.map_err(|e| e.to_string()))
    }

    async fn get(&mut self, path: Vec<String>) -> wasmtime::Result<Result<String, String>> {
        if self.permissions.data == DataPermission::None {
            return Ok(Err("this plugin's manifest does not grant data access".into()));
        }
        let path = parse_path(&path);
        // Null for a path that holds nothing yet, same as the WebSocket's Get:
        // an empty show is an answer, not an error.
        let value = self.engine.get(path).await.unwrap_or(Value::Null);
        Ok(Ok(to_json_text(&value)))
    }

    async fn subscribe(&mut self, pattern: String) -> wasmtime::Result<u64> {
        if self.permissions.data == DataPermission::None {
            // No data, no updates. Returning a dead token keeps the signature
            // simple; the plugin was told at review time what it may do.
            return Ok(0);
        }
        self.next_token += 1;
        let token = self.next_token;
        let pattern = PathPattern::new(pattern);
        let mut stream = self.broadcast.subscribe_all();
        let tx = self.self_tx.clone();
        let task = tokio::spawn(async move {
            use futures::StreamExt;
            while let Some((path, value)) = stream.next().await {
                if pattern.matches(&path) {
                    if tx.send(InstanceMsg::Update { token, path, value }).is_err() {
                        break;
                    }
                }
            }
        });
        self.subs.insert(token, task.abort_handle());
        Ok(token)
    }

    async fn unsubscribe(&mut self, token: u64) -> wasmtime::Result<()> {
        if let Some(task) = self.subs.remove(&token) {
            task.abort();
        }
        Ok(())
    }

    async fn call(
        &mut self,
        method: String,
        args: String,
    ) -> wasmtime::Result<Result<String, String>> {
        if !self.permissions.commands {
            return Ok(Err("this plugin's manifest does not grant commands = true".into()));
        }
        let args: Value = match serde_json::from_str(&args) {
            Ok(v) => v,
            Err(e) => return Ok(Err(format!("args are not JSON: {e}"))),
        };
        let result = if rpcs::is_local_rpc(&method) {
            rpcs::dispatch(&method, args, &self.rpc_deps).await
        } else {
            self.engine.call(method, args).await.map_err(|e| e.to_string())
        };
        Ok(result.map(|v| to_json_text(&v)))
    }
}

impl introspection::Host for PluginCtx {
    async fn entities(&mut self) -> wasmtime::Result<String> {
        let entities: Vec<Value> = inventory::iter::<EntityMeta>()
            .map(|meta| {
                let fields: Vec<Value> = (meta.field_lifecycles)()
                    .iter()
                    .map(|(name, lifecycle)| {
                        json!({ "name": name, "lifecycle": lifecycle_name(*lifecycle) })
                    })
                    .collect();
                json!({
                    "entityName": meta.entity_name,
                    "tableName": meta.table_name,
                    "isSingleton": meta.is_singleton,
                    "primaryKey": meta.primary_key,
                    "fields": fields,
                })
            })
            .collect();
        Ok(to_json_text(&Value::Array(entities)))
    }

    async fn commands(&mut self) -> wasmtime::Result<String> {
        let commands: Vec<Value> = inventory::iter::<CommandRegistration>()
            .map(|cmd| {
                json!({
                    "table": (cmd.entity_table)(),
                    "name": cmd.command_name,
                    "argsTs": cmd.args_ts,
                    "argsSchema": cmd.args_schema,
                    "doc": cmd.doc,
                })
            })
            .collect();
        Ok(to_json_text(&Value::Array(commands)))
    }

    async fn rpcs(&mut self) -> wasmtime::Result<String> {
        let rpcs: Vec<Value> = rpcs::LOCAL_RPCS
            .iter()
            .map(|meta| {
                let schema: Value =
                    serde_json::from_str(meta.args_schema).unwrap_or(Value::Array(vec![]));
                json!({ "method": meta.method, "argsSchema": schema, "doc": meta.doc })
            })
            .collect();
        Ok(to_json_text(&Value::Array(rpcs)))
    }
}

impl peers::Host for PluginCtx {
    async fn call_plugin(
        &mut self,
        plugin: String,
        method: String,
        args: String,
    ) -> wasmtime::Result<Result<String, String>> {
        if !self.deps.iter().any(|d| *d == plugin) {
            return Ok(Err(format!(
                "{:?} is not in this plugin's [dependencies]",
                plugin
            )));
        }
        if plugin == self.plugin_id || self.chain.iter().any(|c| *c == plugin) {
            return Ok(Err(format!("call cycle: {} is already on the call chain", plugin)));
        }
        if self.chain.len() + 1 >= MAX_CALL_DEPTH {
            return Ok(Err("plugin call chain too deep".into()));
        }
        let args: Value = match serde_json::from_str(&args) {
            Ok(v) => v,
            Err(e) => return Ok(Err(format!("args are not JSON: {e}"))),
        };
        let mut chain = self.chain.clone();
        chain.push(self.plugin_id.clone());
        let (reply_tx, reply_rx) = oneshot::channel();
        let sent = self
            .manager
            .send(PluginCommand::Call {
                plugin,
                method,
                args,
                // The caller's context travels with the call, so a delegated
                // command still acts on the same selection for the same user.
                ctx: self.ctx.clone(),
                chain,
                reply: reply_tx,
            })
            .await;
        if sent.is_err() {
            return Ok(Err("the plugin runtime is shutting down".into()));
        }
        match reply_rx.await {
            Ok(result) => Ok(result.map(|v| to_json_text(&v))),
            Err(_) => Ok(Err("the called plugin went away mid-call".into())),
        }
    }
}

impl PluginCtx {
    /// The store this call names, if the manifest declared it.
    ///
    /// Every store call starts here, before anything is read or written, so an
    /// undeclared name never reaches storage of either kind.
    fn declared(&self, store: &str) -> Result<StoreSection, String> {
        self.stores
            .iter()
            .find(|s| s.id == store)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "this plugin's manifest declares no store {store:?} \
                     (declare it with a [[stores]] block)"
                )
            })
    }

    /// The `value` field of the row a key lives at. Worked out by the host from
    /// its own plugin id, so a guest cannot spell a path into anyone else's data.
    fn datum_path(&self, store: &str, key: &str) -> Path {
        vec![
            PathSegment::Key("plugin_data".into()),
            PathSegment::Id(PluginDatum::id_for(&self.plugin_id, store, key)),
            PathSegment::Key("value".into()),
        ]
    }

    /// Every row of one of this plugin's show-scoped stores.
    async fn show_store_rows(&mut self, store: &str) -> Vec<PluginDatum> {
        let all = self
            .engine
            .get(vec![PathSegment::Key("plugin_data".into())])
            .await
            .unwrap_or(Value::Null);
        serde_json::from_value::<Vec<PluginDatum>>(all)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.plugin_id == self.plugin_id && d.store == store)
            .collect()
    }

    async fn show_store_get(&mut self, store: &str, key: &str) -> Result<Value, String> {
        // Null for a key that holds nothing, the way `data.get` answers for a
        // path that holds nothing: a cache's first run is not a failure.
        Ok(self.engine.get(self.datum_path(store, key)).await.unwrap_or(Value::Null))
    }

    /// Write one key of a show-scoped store, through the engine's ordinary
    /// write path so replication, persistence and catch-up all follow.
    ///
    /// **Whether the write is attributed is the whole of the undo behaviour.**
    /// `Operation::is_undoable` already requires a user, and the History panel
    /// reads `WHERE user_id IS NOT NULL`, so an unattributed write is
    /// non-undoable and invisible to the history with nothing taught about
    /// plugins. A store the manifest declared `undoable` is attributed to
    /// whoever is behind the call instead, and undoes like any other edit.
    ///
    /// The gesture is kept either way: coalescing keys on it, not on the user,
    /// so a plugin writing one key repeatedly inside one call still collapses.
    async fn show_store_set(
        &mut self,
        store: &StoreSection,
        key: &str,
        value: Value,
    ) -> Result<(), String> {
        let rows = self.show_store_rows(&store.id).await;
        if let Err(reason) = within_quota(store, &rows, key, &value) {
            return Err(reason);
        }

        let id = PluginDatum::id_for(&self.plugin_id, &store.id, key);
        let existing = rows.iter().any(|d| d.id == id);
        // An operator only exists behind a call they made. A timer, `init` or a
        // chain nobody started has none, so those stay unattributed however the
        // store is declared — attributing them would put a stranger's act at the
        // top of somebody's undo stack.
        let user = if store.undoable { self.user } else { None };

        let (path, value) = if existing {
            (self.datum_path(&store.id, key), value)
        } else {
            // A new key is a create, which carries the whole entity — and the
            // id with it, which is what makes the row the same on every station.
            let datum = PluginDatum {
                id,
                plugin_id: self.plugin_id.clone(),
                store: store.id.clone(),
                key: key.to_string(),
                value,
            };
            (
                vec![
                    PathSegment::Key("plugin_data".into()),
                    PathSegment::Key("__create".into()),
                ],
                serde_json::to_value(datum).unwrap_or(Value::Null),
            )
        };

        let result = match user {
            Some(user) => {
                self.engine.set_as(user, self.gesture, path, Lifecycle::Persisted, value).await
            }
            None => self.engine.set(path, Lifecycle::Persisted, value).await,
        };
        result.map_err(|e| e.to_string())
    }

    async fn show_store_delete(&mut self, store: &StoreSection, key: &str) -> Result<(), String> {
        let id = PluginDatum::id_for(&self.plugin_id, &store.id, key);
        if !self.show_store_rows(&store.id).await.iter().any(|d| d.id == id) {
            // Forgetting what was never there is not an error.
            return Ok(());
        }
        let path = vec![
            PathSegment::Key("plugin_data".into()),
            PathSegment::Id(id),
            PathSegment::Key("__delete".into()),
        ];
        let user = if store.undoable { self.user } else { None };
        let result = match user {
            Some(user) => {
                self.engine
                    .set_as(user, self.gesture, path, Lifecycle::Persisted, Value::Null)
                    .await
            }
            None => self.engine.set(path, Lifecycle::Persisted, Value::Null).await,
        };
        result.map_err(|e| e.to_string())
    }

    async fn show_store_keys(&mut self, store: &str, prefix: &str) -> Result<Vec<String>, String> {
        let mut keys: Vec<String> = self
            .show_store_rows(store)
            .await
            .into_iter()
            .map(|d| d.key)
            .filter(|k| k.starts_with(prefix))
            .collect();
        keys.sort();
        Ok(keys)
    }
}

impl PluginCtx {
    async fn station_store_get(&mut self, store: &str, key: &str) -> Result<Value, String> {
        Ok(self.station_store.get(&self.plugin_id, store, key).await)
    }

    async fn station_store_set(
        &mut self,
        store: &StoreSection,
        key: &str,
        value: Value,
    ) -> Result<(), String> {
        let rows = self.station_store.rows(&self.plugin_id, &store.id).await;
        // The quota reads the same rows either way, so it is one function over
        // `(key, value)` pairs rather than two that could drift apart.
        let as_data: Vec<PluginDatum> = rows
            .into_iter()
            .map(|(key, value)| PluginDatum {
                id: PluginDatum::id_for(&self.plugin_id, &store.id, &key),
                plugin_id: self.plugin_id.clone(),
                store: store.id.clone(),
                key,
                value,
            })
            .collect();
        within_quota(store, &as_data, key, &value)?;
        self.station_store.set(&self.plugin_id, &store.id, key, &value).await
    }

    async fn station_store_delete(&mut self, store: &str, key: &str) -> Result<(), String> {
        self.station_store.delete(&self.plugin_id, store, key).await
    }

    async fn station_store_keys(&mut self, store: &str, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self
            .station_store
            .rows(&self.plugin_id, store)
            .await
            .into_iter()
            .map(|(key, _)| key)
            .filter(|k| k.starts_with(prefix))
            .collect())
    }
}

/// Would this write take the store past what it may hold?
///
/// Checked before the write, so a refused one leaves the store exactly as it
/// was. Replacing a key's value spends only the difference, which is why the
/// key being replaced is subtracted rather than the whole store being summed.
fn within_quota(
    store: &StoreSection,
    rows: &[PluginDatum],
    key: &str,
    value: &Value,
) -> Result<(), String> {
    let replacing = rows.iter().find(|d| d.key == key);
    if replacing.is_none() && rows.len() as u64 >= store.max_keys {
        return Err(format!(
            "store {:?} already holds its {} keys",
            store.id, store.max_keys
        ));
    }
    let size_of = |v: &Value| serde_json::to_string(v).map(|s| s.len() as u64).unwrap_or(0);
    let used: u64 = rows.iter().map(|d| size_of(&d.value)).sum();
    let after = used - replacing.map(|d| size_of(&d.value)).unwrap_or(0) + size_of(value);
    if after > store.max_bytes {
        return Err(format!(
            "store {:?} holds {} bytes and may hold {}; this write would make it {after}",
            store.id, used, store.max_bytes
        ));
    }
    Ok(())
}

impl store::Host for PluginCtx {
    async fn get(
        &mut self,
        store: String,
        key: String,
    ) -> wasmtime::Result<Result<String, String>> {
        let declared = match self.declared(&store) {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        let value = match declared.scope {
            StoreScope::Show => self.show_store_get(&store, &key).await,
            StoreScope::Station => self.station_store_get(&store, &key).await,
        };
        Ok(value.map(|v| to_json_text(&v)))
    }

    async fn set(
        &mut self,
        store: String,
        key: String,
        value: String,
    ) -> wasmtime::Result<Result<(), String>> {
        let declared = match self.declared(&store) {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        let value: Value = match serde_json::from_str(&value) {
            Ok(v) => v,
            Err(e) => return Ok(Err(format!("value is not JSON: {e}"))),
        };
        Ok(match declared.scope {
            StoreScope::Show => self.show_store_set(&declared, &key, value).await,
            StoreScope::Station => self.station_store_set(&declared, &key, value).await,
        })
    }

    async fn delete(
        &mut self,
        store: String,
        key: String,
    ) -> wasmtime::Result<Result<(), String>> {
        let declared = match self.declared(&store) {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        Ok(match declared.scope {
            StoreScope::Show => self.show_store_delete(&declared, &key).await,
            StoreScope::Station => self.station_store_delete(&store, &key).await,
        })
    }

    /// Watch one show-scoped store for changes this plugin did not make.
    ///
    /// The work is done off the event loop, in a task per subscription, exactly
    /// like `data.subscribe` — and it is deliberately built on the same
    /// broadcast rather than on a hook in `show_store_set`. A hook would see
    /// only this station's plugin writing; the broadcast sees an undo, a peer's
    /// copy of the same plugin, and a showfile catching up, which are the three
    /// ways a value actually moves out from under a plugin that is holding it.
    ///
    /// The task keeps `id -> key` for the store's rows, because a row is
    /// addressed by a UUIDv5 over `(plugin, store, key)` and that is one-way: a
    /// field write broadcasts `plugin_data/<id>/value` and nothing in the path
    /// says which key it is. Seeded at subscribe and kept up as rows arrive, so
    /// a key being *deleted* — where the row is gone before anyone could read
    /// it — still reports the key it was.
    async fn subscribe(&mut self, store: String) -> wasmtime::Result<u64> {
        let Ok(declared) = self.declared(&store) else {
            tracing::warn!(
                "[plugin:{}] subscribed to store {store:?}, which its manifest does not declare",
                self.plugin_id
            );
            return Ok(0);
        };
        if declared.scope == StoreScope::Station {
            // Nothing to say. This machine's file, and this plugin is the only
            // writer, so every change to it is one the guest just made.
            return Ok(0);
        }

        self.next_token += 1;
        let token = self.next_token;
        let mut known: HashMap<Uuid, String> = self
            .show_store_rows(&store)
            .await
            .into_iter()
            .map(|d| (d.id, d.key))
            .collect();

        let mut stream = self.broadcast.subscribe_all();
        let tx = self.self_tx.clone();
        let plugin_id = self.plugin_id.clone();
        let engine = self.engine.clone();
        let task = tokio::spawn(async move {
            use futures::StreamExt;
            while let Some((path, value)) = stream.next().await {
                let [PathSegment::Key(table), rest @ ..] = path.as_slice() else { continue };
                if table != "plugin_data" {
                    continue;
                }
                // A create or a delete broadcasts the whole collection, so the
                // rows in hand are the answer and the diff against what was
                // known names both what arrived and what went.
                let changed: Vec<(String, Value)> = match rest {
                    [] => {
                        let rows: Vec<PluginDatum> =
                            serde_json::from_value(value).unwrap_or_default();
                        let mut now: HashMap<Uuid, String> = HashMap::new();
                        let mut changed = Vec::new();
                        for row in rows {
                            if row.plugin_id != plugin_id || row.store != store {
                                continue;
                            }
                            if !known.contains_key(&row.id) {
                                changed.push((row.key.clone(), row.value.clone()));
                            }
                            now.insert(row.id, row.key);
                        }
                        for (id, key) in &known {
                            if !now.contains_key(id) {
                                changed.push((key.clone(), Value::Null));
                            }
                        }
                        known = now;
                        changed
                    }
                    // A field write on a row. Only `value` moves a stored key;
                    // nothing else on the row is the plugin's business.
                    [PathSegment::Id(id), PathSegment::Key(field)] if field == "value" => {
                        match known.get(id) {
                            Some(key) => vec![(key.clone(), value)],
                            // A row this task has not seen, which happens when a
                            // create arrived as a snapshot rather than as a
                            // broadcast. Ask once, and remember it.
                            None => {
                                let row = engine
                                    .get(vec![
                                        PathSegment::Key("plugin_data".into()),
                                        PathSegment::Id(*id),
                                    ])
                                    .await
                                    .ok()
                                    .and_then(|v| {
                                        serde_json::from_value::<PluginDatum>(v).ok()
                                    });
                                match row {
                                    Some(row)
                                        if row.plugin_id == plugin_id && row.store == store =>
                                    {
                                        known.insert(row.id, row.key.clone());
                                        vec![(row.key, value)]
                                    }
                                    _ => continue,
                                }
                            }
                        }
                    }
                    _ => continue,
                };

                for (key, value) in changed {
                    let update = InstanceMsg::Update {
                        token,
                        path: vec![
                            PathSegment::Key(store.clone()),
                            PathSegment::Key(key),
                        ],
                        value,
                    };
                    if tx.send(update).is_err() {
                        return;
                    }
                }
            }
        });
        self.subs.insert(token, task.abort_handle());
        Ok(token)
    }

    async fn unsubscribe(&mut self, token: u64) -> wasmtime::Result<()> {
        if let Some(task) = self.subs.remove(&token) {
            task.abort();
        }
        Ok(())
    }

    async fn keys(
        &mut self,
        store: String,
        prefix: String,
    ) -> wasmtime::Result<Result<Vec<String>, String>> {
        let declared = match self.declared(&store) {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        Ok(match declared.scope {
            StoreScope::Show => self.show_store_keys(&store, &prefix).await,
            StoreScope::Station => self.station_store_keys(&store, &prefix).await,
        })
    }
}

impl logging::Host for PluginCtx {
    async fn log(&mut self, level: types::LogLevel, message: String) -> wasmtime::Result<()> {
        let id = &self.plugin_id;
        match level {
            types::LogLevel::Trace => tracing::trace!("[plugin:{id}] {message}"),
            types::LogLevel::Debug => tracing::debug!("[plugin:{id}] {message}"),
            types::LogLevel::Info => tracing::info!("[plugin:{id}] {message}"),
            types::LogLevel::Warn => tracing::warn!("[plugin:{id}] {message}"),
            types::LogLevel::Error => tracing::error!("[plugin:{id}] {message}"),
        }
        Ok(())
    }
}

fn lifecycle_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Local => "local",
        Lifecycle::Synced => "synced",
        Lifecycle::Persisted => "persisted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_read_the_way_the_wire_does() {
        let id = "2f6b535b-9a71-4c39-9d95-6d6ab2f0f639";
        assert_eq!(parse_segment(id), PathSegment::Id(id.parse().unwrap()));
        assert_eq!(parse_segment("5"), PathSegment::Index(5));
        assert_eq!(parse_segment("fade_time"), PathSegment::Key("fade_time".into()));
    }

    fn a_store(max_keys: u64, max_bytes: u64) -> StoreSection {
        StoreSection {
            id: "s".into(),
            scope: StoreScope::Show,
            max_keys,
            max_bytes,
            undoable: false,
        }
    }

    fn rows(pairs: &[(&str, Value)]) -> Vec<PluginDatum> {
        pairs
            .iter()
            .map(|(key, value)| PluginDatum {
                id: PluginDatum::id_for("p", "s", key),
                plugin_id: "p".into(),
                store: "s".into(),
                key: (*key).to_string(),
                value: value.clone(),
            })
            .collect()
    }

    #[test]
    fn the_key_count_is_a_ceiling_on_new_keys_only() {
        let store = a_store(2, 1_000);
        let full = rows(&[("a", json!(1)), ("b", json!(2))]);

        let err = within_quota(&store, &full, "c", &json!(3)).unwrap_err();
        assert!(err.contains("its 2 keys"), "names the limit: {err}");

        // Replacing a key that is already there adds no key, so a full store is
        // still writable — otherwise a cache at its limit could never be updated,
        // only cleared.
        assert!(within_quota(&store, &full, "a", &json!(99)).is_ok());
    }

    #[test]
    fn the_byte_ceiling_counts_what_the_write_would_leave_behind() {
        let store = a_store(100, 40);
        let existing = rows(&[("a", json!("aaaaaaaaaa"))]);

        // A second key has to fit beside the first.
        assert!(within_quota(&store, &existing, "b", &json!("bb")).is_ok());
        let err = within_quota(&store, &existing, "b", &json!("b".repeat(60))).unwrap_err();
        assert!(err.contains("may hold"), "names the limit: {err}");

        // Replacing spends only the difference, so a value that would not fit
        // *beside* the old one still fits *instead* of it.
        assert!(
            within_quota(&store, &existing, "a", &json!("a".repeat(30))).is_ok(),
            "replacing a key does not pay for it twice"
        );
    }

    #[test]
    fn an_empty_store_takes_the_first_thing_written_to_it() {
        let store = a_store(1, 16);
        assert!(within_quota(&store, &[], "first", &json!("x")).is_ok());
    }

    #[test]
    fn the_allowlist_is_a_host_list_not_a_prefix_match() {
        let hooks = AllowlistHooks {
            plugin_id: "test".into(),
            allow: vec!["localhost".into(), "api.example.com:8443".into()],
        };
        assert!(hooks.permits("localhost", None));
        assert!(hooks.permits("localhost", Some(11434)));
        assert!(hooks.permits("api.example.com", Some(8443)));
        assert!(!hooks.permits("api.example.com", Some(443)));
        assert!(!hooks.permits("evil-localhost", None));
        assert!(!hooks.permits("localhost.example.com", None));
    }
}
