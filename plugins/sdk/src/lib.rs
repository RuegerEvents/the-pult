//! The plugin SDK: `wit/pult-plugin.wit`, wrapped for comfort.
//!
//! A plugin is one type implementing [`PultPlugin`], registered with
//! [`plugin_main!`]. The raw bindings stay public underneath for anything the
//! wrappers don't cover; the wrappers exist so a plugin works in
//! `serde_json::Value` instead of JSON strings, and never touches the
//! `export!` plumbing.
//!
//! ```ignore
//! use pult_plugin_sdk::{self as sdk, PultPlugin};
//!
//! struct Hello;
//!
//! impl PultPlugin for Hello {
//!     fn init(_config: serde_json::Value) -> Result<Self, String> {
//!         sdk::log_info!("hello from a plugin");
//!         Ok(Hello)
//!     }
//!     fn handle(&mut self, method: &str, _args: serde_json::Value, _ctx: serde_json::Value)
//!         -> Result<serde_json::Value, String>
//!     {
//!         Err(format!("no method called {method:?}"))
//!     }
//! }
//!
//! sdk::plugin_main!(Hello);
//! ```

use serde_json::Value;

mod generated;

pub mod data;

/// The show's types, as a plugin sees them.
///
/// Generated from `pult-schema` — see [`data`] for what the typed layer is and what
/// it deliberately is not.
pub mod schema {
    pub use crate::generated::schema::*;
}

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
    pub_export_macro: true,
    default_bindings_module: "pult_plugin_sdk",
});

/// What a plugin is. One instance lives for the life of the WASM instance;
/// hot reload drops it and `init`s a fresh one.
pub trait PultPlugin: Sized + 'static {
    /// The manifest's `[config]` merged with `config.toml`. An `Err` marks the
    /// plugin failed on the console, with this string as the reason.
    fn init(config: Value) -> Result<Self, String>;

    /// Every inbound call: surface traffic (`surface.exec`, `surface.complete`,
    /// `surface.help`), WebSocket `plugin.<id>.<method>` calls, and other
    /// plugins' `call_plugin`. `ctx` is the caller's context or `Value::Null`.
    fn handle(&mut self, method: &str, args: Value, ctx: Value) -> Result<Value, String>;

    /// A value under one of this plugin's subscriptions changed.
    fn on_update(&mut self, _token: u64, _path: &[String], _value: Value) {}

    /// About to be dropped — reload or shutdown. Nothing survives.
    fn shutdown(&mut self) {}
}

/// The host, in `Value`s.
pub mod host {
    use super::Value;
    use super::pult::plugin::{data, introspection, peers};

    fn parse(text: String) -> Value {
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn text(value: &Value) -> String {
        serde_json::to_string(value).unwrap_or_else(|_| "null".into())
    }

    /// Read a path. `Value::Null` where nothing is.
    pub fn get(path: &[&str]) -> Result<Value, String> {
        let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        data::get(&path).map(parse)
    }

    /// Write a path. Needs `data = "read-write"` in the manifest.
    pub fn set(path: &[&str], value: &Value) -> Result<(), String> {
        let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        data::set(&path, &text(value))
    }

    /// Invoke an entity command (`"sequences.goNext"`) or a station RPC
    /// (`"session.join"`). Needs `commands = true` in the manifest.
    pub fn call(method: &str, args: &Value) -> Result<Value, String> {
        data::call(method, &text(args)).map(parse)
    }

    /// Subscribe to a slash-joined pattern; updates arrive at
    /// [`super::PultPlugin::on_update`] carrying the returned token.
    pub fn subscribe(pattern: &str) -> u64 {
        data::subscribe(pattern)
    }

    pub fn unsubscribe(token: u64) {
        data::unsubscribe(token);
    }

    /// Call another plugin. It must be named in this plugin's
    /// `[dependencies]`; the current caller context travels along.
    pub fn call_plugin(plugin: &str, method: &str, args: &Value) -> Result<Value, String> {
        peers::call_plugin(plugin, method, &text(args)).map(parse)
    }

    /// The entity registry: every collection, its fields and lifecycles.
    pub fn entities() -> Value {
        parse(introspection::entities())
    }

    /// Every registered entity command, with `argsSchema` and `doc`.
    pub fn commands() -> Value {
        parse(introspection::commands())
    }

    /// Every station RPC reachable through [`call`].
    pub fn rpcs() -> Value {
        parse(introspection::rpcs())
    }
}

/// What this plugin remembers between runs.
///
/// Two homes, and the manifest decides which is which. A `scope = "show"` store
/// is in the showfile and on every console in the session; a
/// `scope = "station"` store is on this machine only and never travels in a
/// showfile. The calls are identical either way — a plugin should not have to
/// write the difference twice — so the only place the distinction appears is
/// the `[[stores]]` block an operator can read.
///
/// A store must be declared to be addressable, and a plugin can address no
/// other plugin's:
///
/// ```toml
/// [[stores]]
/// id = "prefs"
/// scope = "station"
/// ```
///
/// ```ignore
/// sdk::store::set("prefs", "provider", &"ollama")?;
/// let provider: Option<String> = sdk::store::get("prefs", "provider")?;
/// ```
///
/// **Never a credential.** A show-scoped store replicates and lands in every
/// backup. Keys belong in the environment passthrough a manifest declares by
/// name, or in this station's `preferences.toml`.
pub mod store {
    use super::Value;
    use super::pult::plugin::store as raw;
    use serde::de::DeserializeOwned;
    use serde::Serialize;

    /// Read a key and deserialize it. `None` where nothing is stored — which is
    /// an answer rather than an error, since a cache's first run has none.
    pub fn get<T: DeserializeOwned>(store: &str, key: &str) -> Result<Option<T>, String> {
        let text = raw::get(store, key)?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value).map(Some).map_err(|e| e.to_string())
    }

    /// Read a key without deserializing, for data whose shape is not known
    /// ahead of time.
    pub fn get_value(store: &str, key: &str) -> Result<Value, String> {
        raw::get(store, key).map(|text| serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Write a key. Refused, leaving the store as it was, if the store is not
    /// declared or the write would take it past its quota.
    pub fn set<T: Serialize>(store: &str, key: &str, value: &T) -> Result<(), String> {
        let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
        raw::set(store, key, &text)
    }

    /// Forget a key. Forgetting one that was never there is not an error.
    pub fn delete(store: &str, key: &str) -> Result<(), String> {
        raw::delete(store, key)
    }

    /// The keys beginning with `prefix`, in order. `""` lists the store.
    pub fn keys(store: &str, prefix: &str) -> Result<Vec<String>, String> {
        raw::keys(store, prefix)
    }

    /// Be told when a key of this store changes without this plugin doing it.
    ///
    /// Updates arrive at [`super::PultPlugin::on_update`] carrying the returned
    /// token, with `path` as `[store, key]` and `value` the new one —
    /// `Value::Null` where the key was forgotten.
    ///
    /// Worth doing whenever a plugin *caches* what it stored. A show-scoped
    /// store is show data: an operator's Ctrl-Z can take a write back, and
    /// another station's copy of this plugin can write the same key. Neither
    /// reaches a value being held in a struct field, and a plugin that never
    /// reads the key again would never find out.
    ///
    /// A station-scoped store returns 0: nothing but this plugin writes it.
    pub fn subscribe(store: &str) -> u64 {
        raw::subscribe(store)
    }

    pub fn unsubscribe(token: u64) {
        raw::unsubscribe(token);
    }
}

pub use pult::plugin::logging::{log, LogLevel};

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log($crate::LogLevel::Info, &format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log($crate::LogLevel::Warn, &format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log($crate::LogLevel::Error, &format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::log($crate::LogLevel::Debug, &format!($($arg)*)) };
}

/// The surface protocol: the JSON shapes the built-in console and bar panels
/// speak over `surface.exec` / `surface.complete` / `surface.help`. Typed here
/// so a plugin and the frontend agree by construction.
pub mod surface {
    use serde::{Deserialize, Serialize};

    /// `surface.exec` args.
    #[derive(Debug, Deserialize)]
    pub struct ExecRequest {
        pub line: String,
    }

    /// One line of scrollback.
    #[derive(Debug, Serialize)]
    pub struct OutputLine {
        /// `"result"`, `"info"` or `"error"` — the surface styles them.
        pub kind: String,
        pub text: String,
    }

    /// Where in the input an error sits, in byte offsets.
    #[derive(Debug, Serialize)]
    pub struct ErrorSpan {
        pub start: usize,
        pub end: usize,
    }

    /// `surface.exec` result.
    #[derive(Debug, Default, Serialize)]
    pub struct ExecResponse {
        pub lines: Vec<OutputLine>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub error: Option<ExecError>,
        /// Things the surface should do in the browser, e.g. change the
        /// selection. Absent means none.
        ///
        /// The selection effect takes either shape, and `query` wins where a
        /// surface understands both:
        ///
        /// - `{"selection": {"fixtureIds": […]}}` — these fixtures, as a list.
        /// - `{"selection": {"query": <SelectionQuery>}}` — this *question*, so
        ///   the browser's selection goes on following the rig. That is what
        ///   recalling a saved group leaves, and it is why a surface that
        ///   selects a group should hand back the group's query rather than
        ///   what the query happens to pick out today.
        ///
        /// Deliberately untyped JSON: a surface ignores an effect it does not
        /// understand, which is what lets a new one arrive without every plugin
        /// and every surface being rebuilt together.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub effects: Option<serde_json::Value>,
    }

    #[derive(Debug, Serialize)]
    pub struct ExecError {
        pub message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub span: Option<ErrorSpan>,
        /// What would have been accepted, for the message under the caret.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub expected: Vec<String>,
    }

    /// `surface.complete` args. `cursor` is a byte offset into `line`.
    #[derive(Debug, Deserialize)]
    pub struct CompleteRequest {
        pub line: String,
        pub cursor: usize,
    }

    #[derive(Debug, Serialize)]
    pub struct CompletionItem {
        /// What is inserted.
        pub text: String,
        /// Shown dimmed beside it: a title, a type, a hint.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub detail: Option<String>,
    }

    /// `surface.complete` result. The surface replaces
    /// `line[replace_from..cursor]` with the chosen item's text.
    #[derive(Debug, Serialize)]
    pub struct CompleteResponse {
        pub items: Vec<CompletionItem>,
        #[serde(rename = "replaceFrom")]
        pub replace_from: usize,
    }

    /// `surface.help` args.
    #[derive(Debug, Deserialize)]
    pub struct HelpRequest {
        #[serde(default)]
        pub topic: Option<String>,
    }

    /// `surface.help` result: plain text with blank-line paragraphs.
    #[derive(Debug, Serialize)]
    pub struct HelpResponse {
        pub text: String,
    }
}

/// One line of surface scrollback, briefly.
pub fn output_line(kind: &str, text: impl Into<String>) -> surface::OutputLine {
    surface::OutputLine { kind: kind.to_string(), text: text.into() }
}

/// Wire a [`PultPlugin`] type into the component's exports. Call once, at the
/// bottom of the plugin's `lib.rs`.
#[macro_export]
macro_rules! plugin_main {
    ($ty:ty) => {
        mod __pult_plugin_glue {
            use super::*;

            pub struct Component;

            // One guest is one thread; the mutex is for the borrow checker,
            // not for concurrency.
            static INSTANCE: ::std::sync::Mutex<Option<$ty>> = ::std::sync::Mutex::new(None);

            fn parse(text: &str) -> ::serde_json::Value {
                ::serde_json::from_str(text).unwrap_or(::serde_json::Value::Null)
            }

            impl $crate::exports::pult::plugin::lifecycle::Guest for Component {
                fn init(config: String) -> Result<(), String> {
                    let plugin = <$ty as $crate::PultPlugin>::init(parse(&config))?;
                    *INSTANCE.lock().unwrap() = Some(plugin);
                    Ok(())
                }

                fn shutdown() {
                    if let Some(mut plugin) = INSTANCE.lock().unwrap().take() {
                        $crate::PultPlugin::shutdown(&mut plugin);
                    }
                }

                fn on_update(token: u64, path: Vec<String>, value: String) {
                    if let Some(plugin) = INSTANCE.lock().unwrap().as_mut() {
                        $crate::PultPlugin::on_update(plugin, token, &path, parse(&value));
                    }
                }
            }

            impl $crate::exports::pult::plugin::rpc::Guest for Component {
                fn handle(method: String, args: String, ctx: String) -> Result<String, String> {
                    let mut guard = INSTANCE.lock().unwrap();
                    let plugin = guard.as_mut().ok_or("plugin not initialised")?;
                    let result =
                        $crate::PultPlugin::handle(plugin, &method, parse(&args), parse(&ctx))?;
                    Ok(result.to_string())
                }
            }

            $crate::export!(Component with_types_in $crate);
        }
    };
}
