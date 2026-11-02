//! The plugins a station carries inside itself, and installing a bundle into a show.
//!
//! The command line is a plugin, and the wing types into it — so a show made fresh at
//! the venue had a wing that said "no command-line plugin is running" and an Install
//! button that opened a file picker on whatever tablet pressed it, where no bundle
//! was. Porting the command line into the station would be the grammar, the catalog,
//! the help and the panel again; carrying the bundle is a few hundred kilobytes. So
//! the station embeds `bundled/command-line.pult-plugin.zip` (copied there by
//! `scripts/build-plugins.sh --bundle`), installs it into a show that has **no plugins
//! at all** when the show opens, and offers it in the Plugins panel for one with others.
//!
//! "No plugins at all" rather than "a new show", because the case that hurt is a show
//! made yesterday: it has nothing, and the next open fixes it. The edge it leaves is a
//! show somebody emptied on purpose getting the command line back — disable it instead
//! of removing it, and it stays off.

use pult_schema::{lifecycle::Lifecycle, path::PathSegment, types::plugin::PluginPackage};
use serde_json::{json, Value};

use crate::{engine::EngineHandle, infra::assets};

/// What the station carries: plugin id and bundle bytes.
pub const BUILTIN: &[(&str, &[u8])] =
    &[("command-line", include_bytes!("../../../bundled/command-line.pult-plugin.zip"))];

/// A built-in bundle, by plugin id.
pub fn bundle(id: &str) -> Option<&'static [u8]> {
    BUILTIN.iter().find(|(each, _)| *each == id).map(|(_, bytes)| *bytes)
}

/// What the Plugins panel lists: id, name and version out of each manifest.
pub fn listed() -> Vec<Value> {
    BUILTIN
        .iter()
        .filter_map(|(id, bytes)| {
            let info = super::bundle::read_manifest(bytes, &std::env::temp_dir().join(id)).ok()?;
            Some(json!({
                "id": info.manifest.plugin.id,
                "name": info.manifest.plugin.name,
                "version": info.manifest.plugin.version,
            }))
        })
        .collect()
}

/// Install a bundle into the show: validate it, store it, and write its roster row.
///
/// The manifest is read and validated **before** anything is stored, so a rejected
/// bundle leaves neither an asset nor a row behind. One row per plugin id, replaced
/// rather than added, which is what makes a new build an upgrade instead of a
/// conflict — and an upgrade keeps whether the operator had it on.
pub async fn install(
    engine: &EngineHandle,
    store: &assets::AssetStore,
    body: &[u8],
) -> Result<(Value, bool), String> {
    let would_be = super::cache::root().unwrap_or_else(std::env::temp_dir).join(assets::digest(body));
    let info = super::bundle::read_manifest(body, &would_be).map_err(|e| format!("{e:#}"))?;
    let manifest = info.manifest;

    let sha256 = store.put(assets::BUNDLE_MIME, body).await.map_err(|e| e.to_string())?;

    let roster: Vec<PluginPackage> = engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let existing = roster.iter().find(|p| p.plugin_id == manifest.plugin.id);

    let package = PluginPackage {
        id: existing.map(|p| p.id).unwrap_or_else(uuid::Uuid::new_v4),
        plugin_id: manifest.plugin.id.clone(),
        name: manifest.plugin.name.clone(),
        version: manifest.plugin.version.clone(),
        api: manifest.plugin.api.clone(),
        sha256: sha256.clone(),
        enabled: existing.map(|p| p.enabled).unwrap_or(true),
        stage: existing.map(|p| p.stage).unwrap_or_default(),
        config: existing.map(|p| p.config.clone()).unwrap_or(Value::Null),
    };
    let value = serde_json::to_value(&package).map_err(|e| e.to_string())?;
    let path = match existing {
        Some(p) => vec![PathSegment::Key("plugin_packages".into()), PathSegment::Id(p.id)],
        None => vec![PathSegment::Key("plugin_packages".into()), PathSegment::Key("__create".into())],
    };
    engine.set(path, Lifecycle::Persisted, value.clone()).await.map_err(|e| e.to_string())?;
    Ok((value, existing.is_some()))
}

/// Put every built-in plugin into a show that has none at all. Called once a show is
/// open; a show with any plugin in its roster is left exactly as it is.
pub async fn equip(engine: &EngineHandle, store: &assets::AssetStore) {
    let roster: Vec<PluginPackage> = engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if !roster.is_empty() {
        return;
    }
    for (id, bytes) in BUILTIN {
        match install(engine, store, bytes).await {
            Ok(_) => tracing::info!("[plugin] installed the built-in {id} into this show"),
            Err(e) => tracing::warn!("[plugin] could not install the built-in {id}: {e}"),
        }
    }
}
