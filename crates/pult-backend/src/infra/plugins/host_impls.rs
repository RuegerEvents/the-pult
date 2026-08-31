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
    infra::plugins::manifest::{DataPermission, Permissions},
    infra::plugins::runtime::pult::plugin::{data, introspection, logging, peers, types},
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
