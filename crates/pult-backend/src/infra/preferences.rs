//! What this console prefers, across every show it opens.
//!
//! The first thing here that is neither show data nor a start-up flag. A show's
//! settings belong in the showfile and replicate; a flag is decided before the
//! console is running and cannot be changed from the desk. This is the third kind:
//! something an operator sets once and expects to still be true tomorrow, on this
//! machine, whichever show they open next.
//!
//! Machine-wide rather than beside the showfile, which is the opposite of
//! [`crate::infra::identity`] and for the opposite reason. An identity must not
//! travel with a copied showfile; a preference about *new* shows has no showfile to
//! sit beside.
//!
//! Never fails. A console that cannot read its preferences uses the defaults and
//! starts; one that cannot write them says so and carries on. Neither is a reason to
//! keep an operator from their show.

use std::path::PathBuf;

use pult_schema::types::show::{clamp_history_depth, HISTORY_DEPTH_DEFAULT};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// The settings that outlive a show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// What a newly created show starts its `history_depth` at.
    pub history_depth: u32,
    /// Per-plugin overrides, keyed by plugin id: `[plugins.natural-language-control]`.
    ///
    /// The most specific layer of a plugin's configuration, and the only one
    /// that does not travel with the show — which is what makes it the right
    /// home for a credential, or for anything true of this machine and no
    /// other. A show cannot know which console has the local model on it.
    pub plugins: std::collections::BTreeMap<String, toml::Table>,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            history_depth: HISTORY_DEPTH_DEFAULT,
            plugins: std::collections::BTreeMap::new(),
        }
    }
}

impl Preferences {
    /// This machine's overrides for one plugin, as JSON.
    pub fn plugin_config(&self, plugin_id: &str) -> serde_json::Value {
        self.plugins
            .get(plugin_id)
            .and_then(|table| serde_json::to_value(table).ok())
            .unwrap_or(serde_json::Value::Null)
    }
}

impl Preferences {
    /// The same values, with anything out of range brought back inside it.
    pub fn sane(mut self) -> Self {
        self.history_depth = clamp_history_depth(self.history_depth);
        self
    }
}

/// Where the file lives.
///
/// `PULT_PREFERENCES` names it outright, which is how the tests get a file of their
/// own and how somebody running two consoles on one machine can give them separate
/// preferences. Otherwise the platform's own place for this sort of thing.
pub fn path() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PULT_PREFERENCES") {
        return Some(PathBuf::from(named));
    }
    Some(config_dir()?.join("the-pult").join("preferences.toml"))
}

/// The platform's configuration directory, without a crate to ask.
pub(crate) fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return std::env::var_os("APPDATA").map(PathBuf::from);
    }
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        return Some(PathBuf::from(home).join("Library").join("Application Support"));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

/// This console's preferences, or the defaults.
pub fn load() -> Preferences {
    let Some(path) = path() else { return Preferences::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Preferences::default() };
    match toml::from_str::<Preferences>(&raw) {
        Ok(prefs) => prefs.sane(),
        Err(e) => {
            // Replaced by the defaults rather than treated as fatal, for the same
            // reason a corrupt identity is: a console that has forgotten a
            // preference is recoverable, and refusing to open the show is not.
            warn!("[preferences] {} is not readable ({e}); using the defaults", path.display());
            Preferences::default()
        }
    }
}

/// Write them down. Reports why not rather than pretending it worked.
pub fn save(prefs: &Preferences) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("nowhere to keep preferences"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, toml::to_string_pretty(prefs)?)?;
    info!("[preferences] written to {}", path.display());
    Ok(())
}

/// A preferences file belonging to one test, removed when it goes out of scope.
///
/// `PULT_PREFERENCES` is process-wide, so everything holding one of these runs one at
/// a time — and the lock lives here rather than in either test module, because two
/// locks would not be a lock.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) struct OwnFile(pub PathBuf, #[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    /// A file nothing else will touch, named but not yet created.
    pub(crate) fn own_file() -> OwnFile {
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir()
            .join(format!("pult-preferences-{}", uuid::Uuid::new_v4()))
            .join("preferences.toml");
        // SAFETY: the lock above is what keeps this single-threaded.
        unsafe { std::env::set_var("PULT_PREFERENCES", &path) };
        OwnFile(path, guard)
    }

    impl Drop for OwnFile {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
            unsafe { std::env::remove_var("PULT_PREFERENCES") };
        }
    }
}

#[cfg(test)]
mod tests;
