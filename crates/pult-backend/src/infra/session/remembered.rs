//! The session a station last joined, so it can go back to it after a restart.
//!
//! A spare that reboots — a power cut, an unplugged lead, a new build — came back on
//! its own show and stayed there until somebody joined it again by hand, which on show
//! night is exactly the moment nobody is looking at it. So a join that succeeds writes
//! down the leader's sync address, and a station that starts with one written down
//! goes back to it, retrying until it answers. **A deliberate act forgets it**: leaving
//! the session, or starting one of its own. A failover does not — the spare that took
//! over still means to be the old leader's follower once that leader is back.
//!
//! Beside `preferences.toml`, like `recent.toml`, and for its reason: it is something
//! the console noticed rather than something an operator set. **Keyed by the station's
//! own id**, because two stations on one machine (`demo.sh --two`, a test) share that
//! directory and only one of them is the follower.

use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf};

use pult_schema::events::operation::NodeId;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Remembered {
    /// Station id to the leader's sync address it last joined.
    #[serde(default)]
    joined: BTreeMap<String, SocketAddr>,
}

fn path() -> Option<PathBuf> {
    Some(crate::infra::preferences::path()?.with_file_name("sessions.toml"))
}

fn load_all() -> Remembered {
    let Some(path) = path() else { return Remembered::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Remembered::default() };
    toml::from_str(&raw).unwrap_or_default()
}

fn save_all(all: &Remembered) {
    let Some(file) = path() else { return };
    let write = || -> anyhow::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, toml::to_string_pretty(all)?)?;
        Ok(())
    };
    // Never loudly: a station that cannot write this down has still joined.
    if let Err(e) = write() {
        debug!("[session] could not write {}: {e}", file.display());
    }
}

/// The leader this station last joined, if it has not left since.
pub fn load(station: NodeId) -> Option<SocketAddr> {
    load_all().joined.get(&station.0.to_string()).copied()
}

pub fn remember(station: NodeId, leader: SocketAddr) {
    let mut all = load_all();
    if all.joined.get(&station.0.to_string()) == Some(&leader) {
        return;
    }
    all.joined.insert(station.0.to_string(), leader);
    save_all(&all);
}

pub fn forget(station: NodeId) {
    let mut all = load_all();
    if all.joined.remove(&station.0.to_string()).is_some() {
        save_all(&all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_is_remembered_per_station_and_forgotten_on_leaving() {
        let _own = crate::infra::preferences::testing::own_file();
        let spare = NodeId::new();
        let other = NodeId::new();
        let leader: SocketAddr = "192.168.43.225:7701".parse().unwrap();

        remember(spare, leader);
        assert_eq!(load(spare), Some(leader));
        assert_eq!(load(other), None, "two stations on one machine keep their own");

        forget(spare);
        assert_eq!(load(spare), None);
    }
}
