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

/// One thing a plugin remembers, in a store that travels with the show.
///
/// An ordinary entity on purpose. Being one buys SQLite persistence, replication
/// to peers, catch-up from the oplog, the snapshot round trip and vector-clock
/// conflict resolution with no code written for any of them — the promise task 2
/// made, taken. A bespoke table written by the host would have needed its own
/// replication, and a plugin's macros not reaching the second console would be
/// reported as "the plugin is broken".
///
/// **`id` is derived, not fresh.** It is a UUIDv5 over
/// `(plugin_id, store, key)`, so a key names the same row on every station. With
/// a random id, two stations each writing `macros/opening` would create two rows
/// holding one key — not a conflict the vector clock resolves, but a duplicate
/// it has no reason to notice, and a plugin reading back two values for one key.
/// [`PluginDatum::id_for`] is where that is spelled.
///
/// Station-scoped stores are deliberately *not* here: they are persistent and
/// not replicated, which is a combination the lifecycle enum has no name for,
/// and they live in a SQLite file beside the station's preferences instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "plugin_data")]
pub struct PluginDatum {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// Whose data this is. Survives the plugin being removed, which is what
    /// makes a mistaken removal recoverable and lets an operator see what is
    /// left behind and whose it was.
    #[pult(lifecycle = PERSISTED)]
    pub plugin_id: String,
    /// Which of that plugin's declared stores.
    #[pult(lifecycle = PERSISTED)]
    pub store: String,
    #[pult(lifecycle = PERSISTED)]
    pub key: String,
    #[pult(lifecycle = PERSISTED)]
    pub value: serde_json::Value,
}

impl PluginDatum {
    /// The row a key names, the same on every station.
    ///
    /// UUIDv5 rather than v4: the point is that two stations writing one key
    /// write one row, so the id has to be a function of the key rather than of
    /// when it was first written.
    pub fn id_for(plugin_id: &str, store: &str, key: &str) -> Uuid {
        // A namespace of this crate's own, so these ids cannot collide with a
        // v5 minted for anything else.
        const NAMESPACE: Uuid = Uuid::from_bytes([
            0x8f, 0x2d, 0x41, 0x3a, 0x6c, 0x18, 0x5e, 0x94, 0xa7, 0x0b, 0x39, 0xd6, 0x14, 0x82,
            0x7f, 0xc5,
        ]);
        // Length-prefixed rather than joined by a separator: a store called
        // `a/b` with key `c` and a store called `a` with key `b/c` are two
        // different places and must not hash to one row.
        let mut name = Vec::new();
        for part in [plugin_id, store, key] {
            name.extend_from_slice(&(part.len() as u64).to_be_bytes());
            name.extend_from_slice(part.as_bytes());
        }
        Uuid::new_v5(&NAMESPACE, &name)
    }
}

/// Where a plugin is in its life. Failed carries the reason, because the place
/// this is read is a panel telling an operator why their command line is gone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "state", content = "reason")]
pub enum PluginStatus {
    /// The show asks for this plugin and this station does not have its bytes
    /// yet. A state of its own rather than a kind of failure: a station that
    /// has just joined a session is *working*, and saying "failed" while it
    /// downloads would send an operator looking for a fault that is not there.
    Fetching,
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
    /// The digest this station started it from, or `None` for one loaded from a
    /// plugin directory. It is what tells the panel apart a plugin the show
    /// carries from one somebody is editing.
    #[serde(default)]
    pub sha256: Option<String>,
    /// This station is running a copy from disk in place of the one the show
    /// carries. Published because the alternative is an operator on the next
    /// console wondering why the two of them behave differently.
    #[serde(default)]
    pub overridden_by_disk: bool,
    /// What the manifest says this plugin may do, so what a show is asking for
    /// is readable without opening the bundle.
    #[serde(default)]
    pub permissions: PluginPermissions,
}

/// A plugin's declared permissions, as text for an operator rather than as the
/// enforcement itself — that lives in the host, where the guest cannot reach it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PluginPermissions {
    /// `"none"`, `"read"` or `"read-write"`.
    pub data: String,
    pub commands: bool,
    /// Hosts it may reach over outbound HTTP.
    pub http: Vec<String>,
    /// Environment variable *names* passed through to it. Never the values.
    pub env: Vec<String>,
}

/// This station's view of its plugin runtime.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PluginsState {
    pub plugins: Vec<PluginInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id has to be a function of the key and nothing else, because that is
    /// what makes two stations writing one key write one row.
    #[test]
    fn a_key_names_the_same_row_everywhere() {
        let here = PluginDatum::id_for("macros", "saved", "opening");
        let there = PluginDatum::id_for("macros", "saved", "opening");
        assert_eq!(here, there, "two stations must agree without asking each other");

        // And different places stay different, in each of the three parts.
        assert_ne!(here, PluginDatum::id_for("other-plugin", "saved", "opening"));
        assert_ne!(here, PluginDatum::id_for("macros", "other-store", "opening"));
        assert_ne!(here, PluginDatum::id_for("macros", "saved", "other-key"));
    }

    /// The three parts are length-prefixed rather than joined, so no two
    /// distinct triples can spell the same byte string.
    #[test]
    fn the_parts_cannot_run_into_one_another() {
        // Joined with a separator these would both be "a/b/c".
        assert_ne!(
            PluginDatum::id_for("a", "b", "c"),
            PluginDatum::id_for("a", "b/c", ""),
        );
        assert_ne!(
            PluginDatum::id_for("a", "b", "c"),
            PluginDatum::id_for("a/b", "c", ""),
        );
        // And a key that contains a slash is still just a key.
        assert_ne!(
            PluginDatum::id_for("p", "s", "a/b"),
            PluginDatum::id_for("p", "s/a", "b"),
        );
    }
}
