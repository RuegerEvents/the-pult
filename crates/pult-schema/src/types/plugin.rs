//! Plugins, which are two questions rather than one.
//!
//! **What the show asks for** is [`PluginPackage`]: a PERSISTED entity, so the
//! roster saves with the show and replicates, and every console working one show
//! runs the same plugins. A station-local roster would not be a preference, it
//! would be a disagreement about what the show *is*.
//!
//! **What this station is running** is [`PluginsState`], LOCAL throughout. It
//! includes a plugin loaded from a directory that the show knows nothing about,
//! and a failure that is this station's alone — a peer with the same roster and a
//! missing bundle has a different answer. A frontend asking "what panels can I
//! offer?" is asking the station it is connected to.
//!
//! The panel reads both: the roster gives the rows, the LOCAL state gives each
//! row's state here. Same shape as `stations` (each station writes its own row)
//! beside the LOCAL per-link latency it measures.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// When a plugin is relevant: while a show is being built, while it is being
/// run, or both.
///
/// Advisory. It groups the Plugins panel and nothing else — a setup-only plugin
/// loads and runs exactly like any other. Gating loading on it is a later
/// decision, deliberately not taken here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PluginStage {
    Setup,
    Runtime,
    #[default]
    Both,
}

/// One plugin the show carries.
///
/// The bundle itself is not here: `sha256` names bytes in the asset store, which
/// is what lets a station that has never seen this plugin fetch it from a peer
/// and verify what came back. Two shows carrying the same plugin name the same
/// digest and share one copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "plugin_packages")]
pub struct PluginPackage {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// The id from the bundle's own manifest. Unique across the roster, enforced
    /// at the install path rather than by the schema: nothing in the entity
    /// machinery expresses a unique secondary key, and inventing one here would
    /// be the first exception to a rule that has held since task 2.
    #[pult(lifecycle = PERSISTED)]
    pub plugin_id: String,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub version: String,
    /// The plugin API version the bundle was built against, copied from its
    /// manifest so the panel can explain a refusal without opening the zip.
    #[pult(lifecycle = PERSISTED)]
    pub api: String,
    /// The bundle's content address: the sha256 of the zip, in hex.
    #[pult(lifecycle = PERSISTED)]
    pub sha256: String,
    #[pult(lifecycle = PERSISTED)]
    pub enabled: bool,
    #[pult(lifecycle = PERSISTED)]
    #[serde(default)]
    pub stage: PluginStage,
    /// Show-level configuration, merged over the manifest's own defaults and
    /// under this station's overrides. Never a home for a credential: it is in
    /// the showfile, so it replicates and it lands in every backup.
    #[pult(lifecycle = PERSISTED)]
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Where a plugin is in its life. Failed carries the reason, because the place
/// this is read is a panel telling an operator why their command line is gone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "state", content = "reason")]
pub enum PluginStatus {
    Loading,
    Running,
    Failed(String),
}

/// A built-in frontend surface a plugin drives: the frontend supplies the
/// component, the plugin supplies the behaviour over `plugin.<id>.surface.*`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SurfaceInfo {
    /// Unique within the plugin. The workspace panel id becomes
    /// `plugin:<plugin-id>:<surface-id>`.
    pub id: String,
    /// Which built-in component renders it: `"console"` or `"bar"`.
    pub kind: String,
    pub title: String,
}

/// A web-component panel a plugin ships as its own JavaScript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WebPanelInfo {
    /// Unique within the plugin. The workspace panel id becomes
    /// `plugin:<plugin-id>:<panel-id>`.
    pub id: String,
    pub title: String,
    /// The custom element tag the script defines.
    pub element: String,
    /// Script path under the plugin's assets, served at
    /// `/api/plugins/<plugin-id>/assets/<script>`.
    pub script: String,
    /// Whether the panel does its own scrolling and wants exact height.
    pub fills: bool,
}

/// One loaded (or failing) plugin, as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub status: PluginStatus,
    pub surfaces: Vec<SurfaceInfo>,
    pub panels: Vec<WebPanelInfo>,
}

/// This station's view of its plugin runtime.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PluginsState {
    pub plugins: Vec<PluginInfo>,
}
