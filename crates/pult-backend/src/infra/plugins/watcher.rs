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
        // The unpack cache is written by the manager itself, so watching it
        // would make every carried plugin reload the instant it started — and
        // then again, for ever. It normally sits in the config directory and is
        // nowhere near a plugin root; this is here for when somebody points the
        // two at each other, which is otherwise an infinite loop with no
        // message to explain it.
        if watches_the_cache(root) {
            warn!(
                "[plugin] not watching {}: it holds the unpack cache, and watching that \
                 would reload every carried plugin as soon as it started",
                root.display()
            );
            continue;
        }
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

/// Would watching this root mean watching the unpack cache?
///
/// True when the root *is* the cache or holds it. The other direction — a
/// plugin root that happens to live inside the cache — cannot happen: the cache
/// holds only directories named after digests, and one of those is replaced
/// wholesale rather than edited.
fn watches_the_cache(root: &Path) -> bool {
    let Some(cache) = super::cache::root() else { return false };
    // Compared canonically where possible, so `./plugins` and an absolute path
    // to the same directory are not two different answers.
    let cache = cache.canonicalize().unwrap_or(cache);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    cache.starts_with(&root)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `PULT_PLUGIN_CACHE` is process-wide, so these run one at a time.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_root_holding_the_unpack_cache_is_not_watched() {
        let _guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("pult-watch-{}", uuid::Uuid::new_v4()));
        let cache = base.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        // SAFETY: the lock above is what keeps this single-threaded.
        unsafe { std::env::set_var("PULT_PLUGIN_CACHE", &cache) };

        // Watching this would reload every carried plugin the moment it was
        // unpacked, and then again after each reload.
        assert!(watches_the_cache(&base));
        assert!(watches_the_cache(&cache));

        let elsewhere = base.join("plugins");
        std::fs::create_dir_all(&elsewhere).unwrap();
        assert!(!watches_the_cache(&elsewhere), "an ordinary plugin root is watched");

        unsafe { std::env::remove_var("PULT_PLUGIN_CACHE") };
        std::fs::remove_dir_all(&base).ok();
    }
}
