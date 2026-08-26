use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_sync_port")]
    pub sync_port: u16,
    #[serde(default = "default_showfile")]
    pub showfile: String,
}

fn default_port() -> u16 { 7700 }
fn default_sync_port() -> u16 { 7701 }
fn default_showfile() -> String { "show.db".to_owned() }

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            sync_port: default_sync_port(),
            showfile: default_showfile(),
        }
    }
}
