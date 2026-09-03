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

use pult_schema::types::show::{
    clamp_haze, clamp_history_depth, clamp_home_fade_ms, HAZE_DENSITY_DEFAULT,
    HAZE_TURBULENCE_DEFAULT, HISTORY_DEPTH_DEFAULT,
};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// An hour: long enough that a peer dropping off a switch and coming back does not
/// cost a snapshot, short enough that an hour of two stations' telemetry is a few
/// thousand rows rather than a season's worth.
pub const OPLOG_RETENTION_MINUTES_DEFAULT: u32 = 60;
/// Below a minute the log stops being able to answer a reconnection at all, and
/// every peer that blinks is sent the whole show.
pub const OPLOG_RETENTION_MINUTES_MIN: u32 = 1;
/// A week. Past this the retention is not bounding anything an operator would
/// recognise as a session.
pub const OPLOG_RETENTION_MINUTES_MAX: u32 = 60 * 24 * 7;

/// The settings that outlive a show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// What a newly created show starts its `history_depth` at.
    pub history_depth: u32,
    /// How long this station keeps operations nobody performed, in minutes.
    ///
    /// The console's own writes — a fade advancing, a station publishing its memory
    /// use — are logged because a peer catching up needs them, and are the bulk of
    /// the table: one station writes its row every two seconds, for as long as it is
    /// up. Nobody can undo them and they never appear in the history of what people
    /// did, so the only thing that wants them is replication, and only for as long
    /// as a peer might still be away.
    ///
    /// A station preference rather than show data, unlike `history_depth`. What it
    /// encodes is "how long an absence should this machine be able to answer without
    /// sending a snapshot", which is about this rig's network and this machine's
    /// disk. Two stations in one session may legitimately differ, because pruning is
    /// local and each serves catch-up from what it has.
    ///
    /// Getting it wrong costs snapshots rather than correctness: a peer that has
    /// been away longer is sent the whole show, which is a path every joining
    /// station already takes.
    pub oplog_retention_minutes: u32,
    /// Per-plugin overrides, keyed by plugin id: `[plugins.natural-language-control]`.
    ///
    /// The most specific layer of a plugin's configuration, and the only one
    /// that does not travel with the show — which is what makes it the right
    /// home for a credential, or for anything true of this machine and no
    /// other. A show cannot know which console has the local model on it.
    pub plugins: std::collections::BTreeMap<String, toml::Table>,
    /// What a newly created show starts its `home_fade_ms` at.
    ///
    /// Like `history_depth` and for the same reason: how long the rig takes to let go
    /// is show data, because two stations driving one rig over different times is not
    /// a preference but a disagreement the audience can watch. This is what *this*
    /// desk starts a new show with, and after that it stops mattering.
    pub home_fade_ms: u32,
    /// What a newly created show starts its haze at, density then turbulence.
    ///
    /// Same shape as `home_fade_ms` above and for the same reason: how hazy the room
    /// is is show data, because it is a fact about the room rather than about the
    /// screen looking at it. This is what *this* desk starts a new show with.
    ///
    /// A station in an office and a station in a theatre reasonably disagree about
    /// the default, which is the whole argument for it being a preference at all.
    pub haze_density: f32,
    pub haze_turbulence: f32,
    /// What this station keeps in its own log: the panel, the ring and the file.
    ///
    /// A station preference and never show data, like `oplog_retention_minutes`
    /// beside it — what it encodes is how closely somebody is watching *this*
    /// machine, which two stations in one session may legitimately answer
    /// differently. `debug` is a line per write and a heartbeat every five seconds
    /// per peer, so `info` is what a console runs at until somebody is chasing
    /// something.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// What this station puts on the sync link for its peers.
    ///
    /// Quieter than `log_level` on purpose. A rig is several consoles, and every
    /// station's `debug` crossing the network that is also carrying the show is a
    /// cost nobody agreed to — while a peer's warnings and errors are exactly what
    /// the booth wants to see without going up a ladder. A console watching a peer
    /// can ask it for more, and gets it up to that peer's own `log_level` and no
    /// further: a station cannot publish what it never captured.
    #[serde(default = "default_peer_log_level")]
    pub peer_log_level: String,
    /// Where this console keeps the shows nobody gave it a path for.
    ///
    /// A station preference and not show data, which barely needs arguing: it is a
    /// question about this machine's disk. `None` is the platform's data directory,
    /// which is what an operator who never thinks about this gets.
    #[serde(default)]
    pub shows_dir: Option<PathBuf>,
    /// What to log in to the GDTF Share as.
    ///
    /// A station preference and never show data, for the reason a plugin credential is
    /// one: a showfile travels, and a password in it travels with it. `None` until
    /// somebody types one, and the Fixture Types panel says where to.
    ///
    /// The password is written here and **never read back out over HTTP** — `GET
    /// /api/preferences` answers with the user and whether a password is set, which is
    /// everything a settings form needs and nothing an onlooker can use.
    #[serde(default)]
    pub gdtf_share: Option<ShareCredentials>,
}

fn default_log_level() -> String {
    crate::logging::CAPTURE_LEVEL_DEFAULT.as_str().to_string()
}

fn default_peer_log_level() -> String {
    crate::logging::PEER_LEVEL_DEFAULT.as_str().to_string()
}

/// A login for the GDTF Share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareCredentials {
    pub user: String,
    pub password: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            history_depth: HISTORY_DEPTH_DEFAULT,
            // Snapping, which is what a programmer clear has always done.
            home_fade_ms: 0,
            haze_density: HAZE_DENSITY_DEFAULT,
            haze_turbulence: HAZE_TURBULENCE_DEFAULT,
            oplog_retention_minutes: OPLOG_RETENTION_MINUTES_DEFAULT,
            log_level: default_log_level(),
            peer_log_level: default_peer_log_level(),
            plugins: std::collections::BTreeMap::new(),
            shows_dir: None,
            gdtf_share: None,
        }
    }
}

impl Preferences {
    /// What this station keeps, with anything unspellable read as the default.
    pub fn capture_level(&self) -> pult_schema::ws::LogLevel {
        pult_schema::ws::LogLevel::parse(&self.log_level)
            .unwrap_or(crate::logging::CAPTURE_LEVEL_DEFAULT)
    }

    /// What this station tells its peers, never louder than what it captures —
    /// the same clamp [`crate::logging::LogHandle::publish_level_for`] applies to a
    /// raise, because a publish level above the capture level would silently
    /// publish nothing extra.
    pub fn peer_level(&self) -> pult_schema::ws::LogLevel {
        pult_schema::ws::LogLevel::parse(&self.peer_log_level)
            .unwrap_or(crate::logging::PEER_LEVEL_DEFAULT)
            .min(self.capture_level())
    }

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
        self.oplog_retention_minutes = self
            .oplog_retention_minutes
            .clamp(OPLOG_RETENTION_MINUTES_MIN, OPLOG_RETENTION_MINUTES_MAX);
        self.home_fade_ms = clamp_home_fade_ms(self.home_fade_ms);
        self.haze_density = clamp_haze(self.haze_density);
        self.haze_turbulence = clamp_haze(self.haze_turbulence);
        // A level nobody can spell is the default rather than a refusal to start:
        // a typo in a diagnostic setting must not keep a console off the air.
        self.log_level = self.capture_level().as_str().to_string();
        self.peer_log_level = self.peer_level().as_str().to_string();
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
