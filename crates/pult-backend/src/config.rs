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
    #[serde(default = "default_showfile")]
    pub showfile: String,
    /// Seeds for the `outputs` collection, applied only to a show that has none.
    #[serde(default)]
    pub artnet: Vec<SocketAddr>,
    /// `Some(None)` is sACN multicast; `Some(Some(addr))` unicasts there.
    #[serde(default)]
    pub sacn: Option<Option<SocketAddr>>,
    /// Port for the MQTT broker this node runs for its OpenHaunt devices.
    #[serde(default = "default_broker_port")]
    pub openhaunt_broker_port: u16,
    /// Use this station id instead of the one recorded beside the showfile.
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
}

fn default_bind() -> IpAddr { IpAddr::V4(Ipv4Addr::UNSPECIFIED) }
fn default_port() -> u16 { 7700 }
fn default_sync_port() -> u16 { 7701 }
fn default_showfile() -> String { "show.db".to_owned() }
fn default_broker_port() -> u16 { 1883 }

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            sync_port: default_sync_port(),
            showfile: default_showfile(),
            artnet: Vec::new(),
            sacn: None,
            openhaunt_broker_port: default_broker_port(),
            node_id: None,
            plugin_dirs: Vec::new(),
            plugin_data: None,
        }
    }
}
