//! Hot reload: a plugin directory that changes gets loaded fresh.
//!
//! A rebuild is several filesystem events in quick succession — the linker
//! writing, the build script copying — so events are debounced per plugin
//! directory and one `Reload` goes out after things settle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::PluginCommand;

const SETTLE: Duration = Duration::from_millis(300);

/// Watch the plugin roots on a plain thread — notify's callbacks are not
/// async — and feed debounced reloads into the manager. The thread lives for
/// the life of the process; when the manager goes away, sends fail and it ends.
pub fn spawn(roots: Vec<PathBuf>, tx: mpsc::Sender<PluginCommand>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("plugin-watcher".into())
        .spawn(move || watch(roots, tx))
        .expect("spawning the plugin watcher thread")
}

fn watch(roots: Vec<PathBuf>, tx: mpsc::Sender<PluginCommand>) {
    let (event_tx, event_rx) = std::sync::mpsc::channel::<notify::Event>();
    let mut watcher = match notify::recommended_watcher(move |event| {
        if let Ok(event) = event {
            let _ = event_tx.send(event);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            warn!("[plugin] no file watching, hot reload disabled: {e}");
            return;
        }
    };
    for root in &roots {
        if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
            warn!("[plugin] not watching {}: {e}", root.display());
        }
    }

    // Directory → when its events last settled enough to fire.
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    loop {
        let wait = pending
            .values()
            .min()
            .map(|due| due.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(3600));
        match event_rx.recv_timeout(wait) {
            Ok(event) => {
                for path in &event.paths {
                    if let Some(dir) = plugin_dir_of(&roots, path) {
                        pending.insert(dir, Instant::now() + SETTLE);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let due: Vec<PathBuf> = pending
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(dir, _)| dir.clone())
            .collect();
        for dir in due {
            pending.remove(&dir);
            debug!("[plugin] {} changed", dir.display());
            if tx.blocking_send(PluginCommand::Reload { dir }).is_err() {
                return;
            }
        }
    }
}

/// The plugin directory a changed path belongs to: the root itself when the
/// root is a plugin, otherwise the root's child the path is under.
fn plugin_dir_of(roots: &[PathBuf], path: &Path) -> Option<PathBuf> {
    for root in roots {
        let Ok(rest) = path.strip_prefix(root) else { continue };
        if root.join("pult-plugin.toml").is_file() {
            return Some(root.clone());
        }
        let first = rest.components().next()?;
        return Some(root.join(first));
    }
    None
}
