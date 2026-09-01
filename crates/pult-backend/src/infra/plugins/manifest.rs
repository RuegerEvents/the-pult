//! What a plugin says about itself: `pult-plugin.toml`.
//!
//! The manifest is the whole of what the host knows before running any guest
//! code — what to load, what the plugin may touch, and what UI it offers — so
//! everything here is validated before a byte of WASM is compiled. Pure data
//! and pure functions: this file compiles and tests without wasmtime.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The `pult:plugin` WIT package version this backend implements.
///
/// **The plugin's `api` is a floor, not a match.** A component's imports are
/// stamped with the package version it was built against — `pult:plugin/data@1.0.0`
/// — and wasmtime resolves those against the host's semver-compatibly, so a guest
/// built against `1.0` instantiates against a `1.1` host that added an interface,
/// while the reverse cannot work: the host has nothing to satisfy an import that
/// did not exist when it was built. That asymmetry *is* the rule below, and the
/// check here exists to say so in words before wasmtime says it in a link error.
///
/// It has to be `1.x`. Under semver a `0.x` minor bump is a breaking change, so
/// wasmtime treats `0.1` and `0.2` as unrelated and every import fails to
/// resolve — which makes an additive change impossible to ship at `0.x` no matter
/// how it is spelled. Verified rather than assumed: `scripts/check-api-compat.sh`.
pub const API_VERSION: ApiVersion = ApiVersion { major: 1, minor: 1 };

/// A `major.minor` the manifest named. A trailing patch is accepted and ignored:
/// the WIT package carries one and an author copying it across should not be
/// punished for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: u32,
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl std::str::FromStr for ApiVersion {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let mut parts = text.split('.');
        let mut number = |what: &str| -> Result<u32, String> {
            parts
                .next()
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| format!("plugin API version {text:?} has no {what}"))
        };
        let major = number("major version")?;
        let minor = number("minor version")?;
        Ok(ApiVersion { major, minor })
    }
}

impl ApiVersion {
    /// Can a station speaking `self` run a plugin built against `plugin`?
    ///
    /// The same question wasmtime asks of the component's imports, asked early
    /// enough to answer an operator instead of a linker.
    pub fn satisfies(self, plugin: ApiVersion) -> bool {
        self.major == plugin.major && self.minor >= plugin.minor
    }
}

/// The most a store may hold. A manifest may ask for less and not for more —
/// a ceiling a plugin could raise would not be a ceiling. The reason to allow
/// *lowering* is that an author who knows their cache should never exceed
/// sixteen keys is telling the operator reading the manifest something true.
pub const DEFAULT_MAX_KEYS: u64 = 1_000;
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;

fn default_max_keys() -> u64 {
    DEFAULT_MAX_KEYS
}
fn default_max_bytes() -> u64 {
    DEFAULT_MAX_BYTES
}

/// One `[[stores]]` block: somewhere this plugin keeps what it remembers.
///
/// Declaring the store is what grants access to it. There is no key under
/// `[permissions]` because a store is the plugin's own namespace — it cannot
/// address another plugin's, and the host derives where the data lives from
/// `(plugin_id, store)` rather than from anything the guest passes. What an
/// operator needs to know is whether this plugin keeps data and whether that
/// data goes into their showfile, and that is exactly what this block says,
/// where the rest of the permissions are read.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSection {
    pub id: String,
    pub scope: StoreScope,
    #[serde(default = "default_max_keys")]
    pub max_keys: u64,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
    /// Whether a write here is the operator's act rather than the plugin's.
    ///
    /// Off by default, which is the safe direction: a plugin caching something
    /// while handling an operator's command would otherwise put an invisible
    /// entry at the top of that operator's undo stack. On for the store a
    /// plugin saves into *because the operator asked it to* — a macro, a
    /// snippet — where a console that answered Ctrl-Z by taking back the
    /// previous edit instead would be silently doing the wrong thing.
    #[serde(default)]
    pub undoable: bool,
}

/// Whose data is this?
///
/// Deliberately not spelled with the `Lifecycle` enum. In this codebase LOCAL
/// means *not persisted* — state a station holds and shares with its own
/// frontends and which does not survive a reload. A station-scoped store is the
/// opposite: persistent and not replicated, a combination `Lifecycle` has no
/// name for. Naming the axis `scope` says the true thing — whose data is this —
/// and leaves `Lifecycle` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreScope {
    /// In the showfile, replicated to every peer, in the backup.
    Show,
    /// On this machine, beside the preferences, never in a showfile.
    Station,
}

impl StoreScope {
    pub fn as_str(self) -> &'static str {
        match self {
            StoreScope::Show => "show",
            StoreScope::Station => "station",
        }
    }
}

/// `pult-plugin.toml`, parsed. `dir` is where it was found, so relative paths
/// in it have somewhere to be relative to.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub dir: PathBuf,
    pub plugin: PluginSection,
    pub surfaces: Vec<SurfaceSection>,
    pub panels: Vec<PanelSection>,
    pub permissions: Permissions,
    pub dependencies: Dependencies,
    pub stores: Vec<StoreSection>,
    /// The `[config]` table as written; merged with `config.toml` at load time.
    pub config: toml::Table,
}

impl PluginManifest {
    /// The store this plugin declared under `id`, or `None`.
    ///
    /// The host resolves every store call through here rather than through
    /// anything the guest passed, which is what makes "a plugin can address no
    /// store it did not declare, and no other plugin's" true by construction
    /// rather than by checking.
    pub fn store(&self, id: &str) -> Option<&StoreSection> {
        self.stores.iter().find(|s| s.id == id)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSection {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The `pult:plugin` WIT version this plugin was built against.
    pub api: String,
    /// The component file, relative to the manifest.
    pub wasm: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceSection {
    pub id: String,
    pub kind: SurfaceKind,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceKind {
    /// Prompt, scrollback, completions, help — a command line.
    Console,
    /// One line and a flyout — an input that answers.
    Bar,
}

impl SurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SurfaceKind::Console => "console",
            SurfaceKind::Bar => "bar",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSection {
    pub id: String,
    pub title: String,
    /// The custom element tag the script defines.
    pub element: String,
    /// Script path under `assets/`, relative to the manifest.
    pub script: String,
    #[serde(default)]
    pub fills: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub data: DataPermission,
    /// May the plugin invoke entity commands and station RPCs?
    #[serde(default)]
    pub commands: bool,
    /// Hosts the plugin may reach over outbound HTTP. Empty means none: a
    /// plugin that talks to the network says so where an operator can read it.
    #[serde(default)]
    pub http: Vec<String>,
    /// Environment variable names passed through to the guest. Names, not
    /// values — the operator's environment is not the plugin's by default.
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataPermission {
    #[default]
    None,
    Read,
    ReadWrite,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
    /// Plugins this one loads after and is allowed to `call-plugin` into.
    #[serde(default)]
    pub plugins: Vec<String>,
}

/// The manifest as it appears in TOML, before the directory is known.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    plugin: PluginSection,
    #[serde(default)]
    surfaces: Vec<SurfaceSection>,
    #[serde(default)]
    panels: Vec<PanelSection>,
    #[serde(default)]
    permissions: Permissions,
    #[serde(default)]
    dependencies: Dependencies,
    #[serde(default)]
    stores: Vec<StoreSection>,
    #[serde(default)]
    config: toml::Table,
}

impl PluginManifest {
    /// Parse and validate one manifest. `dir` is the directory holding it.
    pub fn parse(dir: &Path, text: &str) -> Result<PluginManifest, String> {
        let raw: RawManifest = toml::from_str(text).map_err(|e| e.to_string())?;
        let manifest = PluginManifest {
            dir: dir.to_path_buf(),
            plugin: raw.plugin,
            surfaces: raw.surfaces,
            panels: raw.panels,
            permissions: raw.permissions,
            dependencies: raw.dependencies,
            stores: raw.stores,
            config: raw.config,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), String> {
        let id = &self.plugin.id;
        if id.is_empty()
            || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "plugin id {id:?} must be lowercase letters, digits and hyphens"
            ));
        }
        let api: ApiVersion = self.plugin.api.parse()?;
        if !API_VERSION.satisfies(api) {
            // Two different failures, and an operator can act on the difference:
            // one wants a newer console, the other a plugin rebuilt against this
            // contract.
            let why = if api.major != API_VERSION.major {
                "a different edition of the plugin contract — it needs rebuilding"
            } else {
                "a newer plugin contract than this station has — update the console"
            };
            return Err(format!(
                "built against plugin API {api}, this station speaks {API_VERSION}: {why}"
            ));
        }
        contained_relative_path(&self.plugin.wasm)?;
        let mut seen = HashSet::new();
        for surface in &self.surfaces {
            if !seen.insert(surface.id.as_str()) {
                return Err(format!("duplicate surface id {:?}", surface.id));
            }
        }
        for panel in &self.panels {
            if !seen.insert(panel.id.as_str()) {
                return Err(format!("duplicate panel id {:?}", panel.id));
            }
            contained_relative_path(&panel.script)?;
        }
        if self.dependencies.plugins.iter().any(|dep| dep == id) {
            return Err("a plugin cannot depend on itself".into());
        }
        self.validate_stores()?;
        Ok(())
    }

    /// Stores get their own namespace: a store called `monitor` and a panel
    /// called `monitor` are addressed differently and never meet, so they do
    /// not collide.
    fn validate_stores(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        for store in &self.stores {
            if store.id.is_empty()
                || !store.id.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                })
            {
                return Err(format!(
                    "store id {:?} must be lowercase letters, digits, hyphens and underscores",
                    store.id
                ));
            }
            if !seen.insert(store.id.as_str()) {
                return Err(format!("duplicate store id {:?}", store.id));
            }
            if store.max_keys > DEFAULT_MAX_KEYS {
                return Err(format!(
                    "store {:?} asks for {} keys; {DEFAULT_MAX_KEYS} is the most a store may hold",
                    store.id, store.max_keys
                ));
            }
            if store.max_bytes > DEFAULT_MAX_BYTES {
                return Err(format!(
                    "store {:?} asks for {} bytes; {DEFAULT_MAX_BYTES} is the most a store may hold",
                    store.id, store.max_bytes
                ));
            }
            // Station data never reaches the oplog, so there is nothing there
            // to take back. A manifest saying otherwise has misunderstood
            // something, and is told so rather than quietly ignored.
            if store.undoable && store.scope == StoreScope::Station {
                return Err(format!(
                    "store {:?} is station-scoped and cannot be undoable: \
                     station data is never written to the show's history",
                    store.id
                ));
            }
        }
        Ok(())
    }

    /// The component file's absolute location.
    pub fn wasm_path(&self) -> PathBuf {
        self.dir.join(&self.plugin.wasm)
    }

    /// The `[config]` table with `config.toml` (if present beside the manifest)
    /// deep-merged over it, as the JSON the guest's `init` receives.
    pub fn effective_config(&self) -> serde_json::Value {
        let mut merged = self.config.clone();
        if let Ok(text) = std::fs::read_to_string(self.dir.join("config.toml")) {
            match toml::from_str::<toml::Table>(&text) {
                Ok(overlay) => deep_merge(&mut merged, overlay),
                Err(e) => tracing::warn!(
                    "[plugin] {}: config.toml ignored, not valid TOML: {e}",
                    self.plugin.id
                ),
            }
        }
        serde_json::to_value(merged).unwrap_or(serde_json::Value::Null)
    }
}

/// Reject a path that could name anything outside the plugin's directory. The
/// manifest is data from disk, not from this codebase; it does not get to
/// reach around the tree.
fn contained_relative_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!("path {path:?} must stay inside the plugin directory"));
    }
    Ok(())
}

/// A plugin's configuration, composed of every layer that has an opinion.
///
/// In increasing order of precedence:
///
/// 1. the manifest's own `[config]`, and a `config.toml` beside it,
/// 2. the show-level configuration on the plugin's roster row,
/// 3. this station's overrides from `preferences.toml`.
///
/// Merged key by key rather than replaced wholesale, so a station overriding
/// one value keeps the show's for its siblings — a station that had to restate
/// a whole table to change one line would get them out of step the first time
/// the show's copy changed.
///
/// **Station last, not show last.** The most specific layer wins, and the
/// things a station legitimately overrides — a credential, a local model URL, a
/// machine with different hardware — are exactly the things that must not be
/// written into a showfile. A show-last order would leave a station unable to
/// correct a value the show got wrong for it.
pub fn compose_config(
    manifest: &PluginManifest,
    show: &serde_json::Value,
    station: &serde_json::Value,
) -> serde_json::Value {
    let mut composed = manifest.effective_config();
    deep_merge_json(&mut composed, show);
    deep_merge_json(&mut composed, station);
    composed
}

fn deep_merge_json(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    match (base, overlay) {
        // Null is "said nothing", not "said empty" — a layer that does not
        // mention a plugin must not blank out the layer beneath it.
        (_, serde_json::Value::Null) => {}
        (serde_json::Value::Object(base), serde_json::Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(existing) => deep_merge_json(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

fn deep_merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => deep_merge(b, o),
            (_, v) => {
                base.insert(key, v);
            }
        }
    }
}

/// Order manifests so every plugin loads after the plugins it depends on.
///
/// A dependency that is not present is an error for the plugin that asked for
/// it, not for the load as a whole — the rest of the plugins still come up.
/// Returns the loadable manifests in order, plus (id, reason) for the rest.
pub fn load_order(
    manifests: Vec<PluginManifest>,
) -> (Vec<PluginManifest>, Vec<(String, String)>) {
    let mut by_id: BTreeMap<String, PluginManifest> = BTreeMap::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    for manifest in manifests {
        let id = manifest.plugin.id.clone();
        if by_id.insert(id.clone(), manifest).is_some() {
            failed.push((id.clone(), "another plugin already has this id".into()));
            by_id.remove(&id);
        }
    }

    // Depth-first with a visiting mark, so a cycle is named rather than looped.
    let mut ordered: Vec<PluginManifest> = Vec::new();
    let mut state: HashMap<String, Mark> = HashMap::new();
    let ids: Vec<String> = by_id.keys().cloned().collect();
    for id in &ids {
        visit(id, &mut by_id, &mut state, &mut ordered, &mut failed);
    }
    (ordered, failed)
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Visiting,
    Done,
    Failed,
}

fn visit(
    id: &str,
    by_id: &mut BTreeMap<String, PluginManifest>,
    state: &mut HashMap<String, Mark>,
    ordered: &mut Vec<PluginManifest>,
    failed: &mut Vec<(String, String)>,
) -> Mark {
    match state.get(id) {
        Some(&mark @ (Mark::Done | Mark::Failed)) => return mark,
        Some(Mark::Visiting) => {
            state.insert(id.to_string(), Mark::Failed);
            failed.push((id.to_string(), "dependency cycle".into()));
            return Mark::Failed;
        }
        None => {}
    }
    let Some(deps) = by_id.get(id).map(|m| m.dependencies.plugins.clone()) else {
        return Mark::Failed;
    };
    state.insert(id.to_string(), Mark::Visiting);
    for dep in &deps {
        let mark = if by_id.contains_key(dep) {
            visit(dep, by_id, state, ordered, failed)
        } else {
            Mark::Failed
        };
        if mark != Mark::Done {
            state.insert(id.to_string(), Mark::Failed);
            failed.push((id.to_string(), format!("depends on {dep:?}, which is not loadable")));
            return Mark::Failed;
        }
    }
    state.insert(id.to_string(), Mark::Done);
    if let Some(manifest) = by_id.get(id) {
        ordered.push(manifest.clone());
    }
    Mark::Done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, deps: &[&str]) -> PluginManifest {
        let deps = deps.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>().join(", ");
        PluginManifest::parse(
            Path::new("/nowhere"),
            &format!(
                r#"
                [plugin]
                id = {id:?}
                name = "Test"
                version = "0.0.0"
                api = "1.0"
                wasm = "test.wasm"

                [dependencies]
                plugins = [{deps}]
                "#
            ),
        )
        .expect("test manifest parses")
    }

    #[test]
    fn parses_a_full_manifest() {
        let m = PluginManifest::parse(
            Path::new("/somewhere"),
            r#"
            [plugin]
            id = "command-line"
            name = "Command Line"
            version = "0.1.0"
            api = "1.0"
            wasm = "command_line.wasm"

            [[surfaces]]
            id = "console"
            kind = "console"
            title = "Command Line"

            [[panels]]
            id = "monitor"
            title = "Monitor"
            element = "pult-cli-monitor"
            script = "assets/panel.js"

            [permissions]
            data = "read-write"
            commands = true
            http = ["localhost"]

            [config]
            prompt = ">"
            "#,
        )
        .expect("parses");
        assert_eq!(m.plugin.id, "command-line");
        assert_eq!(m.surfaces[0].kind, SurfaceKind::Console);
        assert_eq!(m.permissions.data, DataPermission::ReadWrite);
        assert!(m.permissions.commands);
        assert_eq!(m.wasm_path(), Path::new("/somewhere/command_line.wasm"));
    }

    #[test]
    fn refuses_what_a_manifest_must_not_say() {
        let bad = |body: &str| PluginManifest::parse(Path::new("/x"), body).unwrap_err();

        // An id with a path in it would become a path on disk and a panel id.
        let err = bad(r#"[plugin]
            id = "Bad/Id"
            name = "x"
            version = "0"
            api = "1.0"
            wasm = "x.wasm""#);
        assert!(err.contains("lowercase"), "{err}");

        // A wasm path may not leave the plugin directory.
        let err = bad(r#"[plugin]
            id = "escape"
            name = "x"
            version = "0"
            api = "1.0"
            wasm = "../../outside.wasm""#);
        assert!(err.contains("inside the plugin directory"), "{err}");

        // A different API version is a clear report, not a load attempt.
        let err = bad(r#"[plugin]
            id = "future"
            name = "x"
            version = "0"
            api = "9.9"
            wasm = "x.wasm""#);
        assert!(err.contains("this station speaks"), "{err}");
    }

    #[test]
    fn parses_the_stores_a_plugin_declares() {
        let m = PluginManifest::parse(
            Path::new("/somewhere"),
            r#"
            [plugin]
            id = "macros"
            name = "Macros"
            version = "0.1.0"
            api = "1.0"
            wasm = "macros.wasm"

            [[stores]]
            id = "saved"
            scope = "show"
            undoable = true

            [[stores]]
            id = "cache"
            scope = "station"
            max_keys = 16
            "#,
        )
        .expect("parses");

        let saved = m.store("saved").expect("the show store is declared");
        assert_eq!(saved.scope, StoreScope::Show);
        assert!(saved.undoable, "the operator saved this on purpose");
        assert_eq!(saved.max_keys, DEFAULT_MAX_KEYS, "an unstated limit is the default");

        let cache = m.store("cache").expect("the station store is declared");
        assert_eq!(cache.scope, StoreScope::Station);
        assert!(!cache.undoable, "a cache is the plugin's own business");
        assert_eq!(cache.max_keys, 16, "an author may say their cache is small");
        assert_eq!(cache.max_bytes, DEFAULT_MAX_BYTES);

        // Declaring the store is the permission, so a store nobody declared is
        // not addressable — this is the lookup the host does on every call.
        assert!(m.store("somebody-elses").is_none());
    }

    #[test]
    fn refuses_what_a_stores_block_must_not_say() {
        let bad = |stores: &str| {
            PluginManifest::parse(
                Path::new("/x"),
                &format!(
                    r#"[plugin]
                    id = "p"
                    name = "x"
                    version = "0"
                    api = "1.0"
                    wasm = "x.wasm"
                    {stores}"#
                ),
            )
            .unwrap_err()
        };

        let err = bad(
            r#"[[stores]]
            id = "same"
            scope = "show"
            [[stores]]
            id = "same"
            scope = "station""#,
        );
        assert!(err.contains("duplicate store id"), "{err}");

        // A scope that is neither is a typo with consequences — "local" would
        // read as "on this machine" and mean nothing here.
        let err = bad(
            r#"[[stores]]
            id = "s"
            scope = "local""#,
        );
        assert!(err.contains("local"), "names what it did not understand: {err}");

        // A ceiling a plugin could raise would not be a ceiling.
        let err = bad(
            r#"[[stores]]
            id = "greedy"
            scope = "show"
            max_bytes = 999999999"#,
        );
        assert!(err.contains("the most a store may hold"), "{err}");
        let err = bad(
            r#"[[stores]]
            id = "greedy"
            scope = "show"
            max_keys = 100000"#,
        );
        assert!(err.contains("the most a store may hold"), "{err}");

        // Station data never reaches the oplog, so there is nothing there to
        // take back. Saying otherwise is a misunderstanding worth naming.
        let err = bad(
            r#"[[stores]]
            id = "cache"
            scope = "station"
            undoable = true"#,
        );
        assert!(err.contains("cannot be undoable"), "{err}");
    }

    /// The plugin's `api` is a floor the station must reach, not a string to
    /// match — because that is the rule wasmtime enforces underneath, and a
    /// pre-flight check that disagreed with the linker would be worse than none.
    #[test]
    fn the_api_version_is_a_floor_not_a_match() {
        let v = |text: &str| text.parse::<ApiVersion>().expect("parses");

        // A station runs what was built against it, and what was built against
        // anything it has since grown past. The extra interfaces of a later
        // minor are ones the old guest simply never imports.
        assert!(v("1.0").satisfies(v("1.0")));
        assert!(v("1.4").satisfies(v("1.0")), "a station may be ahead of the plugin");
        assert!(v("1.4").satisfies(v("1.4")));

        // The other direction cannot work: the host has nothing to satisfy an
        // import that did not exist when it was built.
        assert!(!v("1.0").satisfies(v("1.1")), "a station cannot be behind the plugin");

        // A major is a different contract, in either direction. `0.x` in
        // particular is every version of this before the contract settled.
        assert!(!v("1.0").satisfies(v("0.1")));
        assert!(!v("1.0").satisfies(v("2.0")));

        // A patch rides along and is ignored — the WIT package carries one and
        // an author copying it across should not be punished for it.
        assert_eq!(v("1.0.0"), v("1.0"));
    }

    #[test]
    fn a_version_mismatch_says_which_thing_to_change() {
        let bad = |api: &str| {
            PluginManifest::parse(
                Path::new("/x"),
                &format!(
                    r#"[plugin]
                    id = "p"
                    name = "x"
                    version = "0"
                    api = {api:?}
                    wasm = "x.wasm""#
                ),
            )
            .unwrap_err()
        };

        // The two failures want opposite actions from the operator, so they do
        // not get to share a sentence.
        let old = bad("0.1");
        assert!(old.contains("rebuilding"), "{old}");
        let new = bad("1.9");
        assert!(new.contains("update the console"), "{new}");

        // Something that is not a version at all is refused as itself rather
        // than silently reading as 0.
        let nonsense = bad("banana");
        assert!(nonsense.contains("major version"), "{nonsense}");
    }

    #[test]
    fn configuration_layers_merge_key_by_key_with_the_station_last() {
        let manifest = PluginManifest::parse(
            Path::new("/nowhere"),
            r#"
            [plugin]
            id = "nl"
            name = "NL"
            version = "0.1.0"
            api = "1.0"
            wasm = "nl.wasm"

            [config]
            provider = "ollama"
            model = "llama3"
            temperature = 0.2
            "#,
        )
        .expect("parses");

        let show = serde_json::json!({ "provider": "openrouter", "model": "sonnet" });
        let station = serde_json::json!({ "model": "the-one-on-this-machine" });

        let composed = compose_config(&manifest, &show, &station);

        // The station said one thing, so it changed one thing. Replacing the
        // table wholesale would have lost the show's provider, and a station
        // that had to restate everything to change a line would drift out of
        // step the first time the show's copy moved.
        assert_eq!(composed["model"], "the-one-on-this-machine", "the station wins");
        assert_eq!(composed["provider"], "openrouter", "and the show still beats the manifest");
        assert_eq!(composed["temperature"], 0.2, "and untouched keys survive from the manifest");
    }

    #[test]
    fn a_layer_with_nothing_to_say_says_nothing() {
        let manifest = PluginManifest::parse(
            Path::new("/nowhere"),
            r#"
            [plugin]
            id = "nl"
            name = "NL"
            version = "0.1.0"
            api = "1.0"
            wasm = "nl.wasm"

            [config]
            provider = "ollama"
            "#,
        )
        .expect("parses");

        // Null is "did not mention it", not "set it to empty" — otherwise a
        // show with no plugin configuration would blank out the defaults every
        // plugin ships with.
        let composed = compose_config(&manifest, &serde_json::Value::Null, &serde_json::Value::Null);
        assert_eq!(composed["provider"], "ollama");
    }

    #[test]
    fn nested_tables_merge_rather_than_replace() {
        let manifest = PluginManifest::parse(
            Path::new("/nowhere"),
            r#"
            [plugin]
            id = "nested"
            name = "Nested"
            version = "0.1.0"
            api = "1.0"
            wasm = "n.wasm"

            [config.limits]
            requests = 10
            seconds = 30
            "#,
        )
        .expect("parses");

        let composed = compose_config(
            &manifest,
            &serde_json::json!({ "limits": { "requests": 100 } }),
            &serde_json::Value::Null,
        );
        assert_eq!(composed["limits"]["requests"], 100);
        assert_eq!(composed["limits"]["seconds"], 30, "the sibling key is not collateral damage");
    }

    #[test]
    fn orders_dependencies_before_dependents() {
        let (ordered, failed) = load_order(vec![
            manifest("natural-language-control", &["command-line"]),
            manifest("command-line", &[]),
        ]);
        assert!(failed.is_empty(), "{failed:?}");
        let ids: Vec<&str> = ordered.iter().map(|m| m.plugin.id.as_str()).collect();
        assert_eq!(ids, ["command-line", "natural-language-control"]);
    }

    #[test]
    fn a_missing_dependency_fails_the_dependent_only() {
        let (ordered, failed) = load_order(vec![
            manifest("alone", &[]),
            manifest("needy", &["absent"]),
        ]);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].plugin.id, "alone");
        assert_eq!(failed.len(), 1);
        assert!(failed[0].1.contains("absent"), "{failed:?}");
    }

    #[test]
    fn a_cycle_fails_its_members_and_names_itself() {
        let (ordered, failed) = load_order(vec![
            manifest("a", &["b"]),
            manifest("b", &["a"]),
            manifest("c", &[]),
        ]);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].plugin.id, "c");
        assert!(failed.iter().any(|(_, why)| why.contains("cycle")), "{failed:?}");
    }
}
