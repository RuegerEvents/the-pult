use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Everything a station needs to come up. The binary fills this in from its
/// flags and the desktop app fills it in from its own defaults, so neither one
/// has to know how the other starts a console.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The address to serve HTTP and the WebSocket on. `0.0.0.0` so a tablet on
    /// the same network can reach the console.
    #[serde(default = "default_bind")]
    pub bind: IpAddr,
    /// `0` asks the OS for a free one; the port that was actually bound comes
    /// back on [`crate::Running::http_addr`].
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_sync_port")]
    pub sync_port: u16,
    /// The show to open: a `Name.pult` bundle directory.
    ///
    /// `None` is a console with **no show open** — which is a real state and the one
    /// a console started with no arguments comes up in. Everything runs: the engine,
    /// the sync layer, the HTTP server serving the welcome screen. What it runs
    /// against is an in-memory database nothing is written to, and the asset store is
    /// what says no, since it is the only part with nowhere to put anything.
    #[serde(default)]
    pub show: Option<std::path::PathBuf>,
    /// Where this station's own id is kept.
    ///
    /// `None` falls back to `PULT_IDENTITY` and then to the config directory. Here as
    /// well as in the environment for the reason `plugin_data` is: an environment
    /// variable is one per process, and two stations inside one program have to be
    /// told separately.
    #[serde(default)]
    pub identity: Option<std::path::PathBuf>,
    /// Where the shows this console makes for itself go, and what the welcome screen
    /// lists. `None` takes the station preference, and then the platform's data
    /// directory.
    #[serde(default)]
    pub shows_dir: Option<std::path::PathBuf>,
    /// Put a demo show in this one, if it has no rig yet.
    ///
    /// Applied after the load and only to a show with no fixtures, so a console that
    /// was started with `--demo` once and has been programmed since does not get a
    /// second rig on top of somebody's work.
    #[serde(default)]
    pub demo: Option<crate::demo::Demo>,
    /// Seeds for the `outputs` collection, applied only to a show that has none.
    #[serde(default)]
    pub artnet: Vec<SocketAddr>,
    /// `Some(None)` is sACN multicast; `Some(Some(addr))` unicasts there.
    #[serde(default)]
    pub sacn: Option<Option<SocketAddr>>,
    /// Port for the MQTT broker this node runs for its OpenHaunt devices.
    #[serde(default = "default_broker_port")]
    pub openhaunt_broker_port: u16,
    /// Use this station id instead of the one this machine has recorded.
    #[serde(default)]
    pub node_id: Option<Uuid>,
    /// Directories to load WASM plugins from: each is one plugin's directory or
    /// a directory of plugin directories. Empty means no plugin runtime work at
    /// all, same philosophy as the output flags.
    #[serde(default)]
    pub plugin_dirs: Vec<std::path::PathBuf>,
    /// Where this station keeps what its plugins remember about *this machine*.
    ///
    /// `None` falls back to `PULT_PLUGIN_DATA` and then to the config directory,
    /// which is what an operator's console does. It is here as well as in the
    /// environment because an environment variable is one per process: two stations
    /// started inside one program — a test binary, or a desktop app opening a second
    /// — cannot each have their own that way.
    #[serde(default)]
    pub plugin_data: Option<std::path::PathBuf>,
    /// The console's own log, if this process installed one.
    ///
    /// Filled in by whoever called [`crate::logging::install`], which is `main` in
    /// both binaries — `tracing_subscriber`'s `init` is once per process, and a
    /// station is a library a process may start more than one of, so the subscriber
    /// cannot be built in here. A station given `None` simply has no log to show,
    /// which is what every test wants.
    ///
    /// Skipped by serde because a handle is not data. Config is never read from a
    /// file, so nothing is lost by that; if it ever is, this field is the one that
    /// must not come back from one.
    #[serde(skip)]
    pub log: Option<crate::logging::LogHandle>,
}

fn default_bind() -> IpAddr { IpAddr::V4(Ipv4Addr::UNSPECIFIED) }
fn default_port() -> u16 { 7700 }
fn default_sync_port() -> u16 { 7701 }
fn default_broker_port() -> u16 { 1883 }

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            sync_port: default_sync_port(),
            show: None,
            identity: None,
            shows_dir: None,
            demo: None,
            artnet: Vec::new(),
            sacn: None,
            openhaunt_broker_port: default_broker_port(),
            node_id: None,
            plugin_dirs: Vec::new(),
            plugin_data: None,
            log: None,
        }
    }
}
