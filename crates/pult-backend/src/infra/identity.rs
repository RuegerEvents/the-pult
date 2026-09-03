//! This station's own identity, kept across restarts.
//!
//! `NodeId` used to be generated fresh every process start, which was harmless
//! while nothing wrote it down. It stopped being harmless when an output began
//! naming the station that sends it: after a restart the station had a new id, the
//! saved output belonged to nobody, and it silently stopped sending.
//!
//! It belongs to *this station* and not to the show, so it is not in the show. It
//! used to be beside it — `show.db.node` — which was already the right idea and the
//! wrong place, and a show became a folder is what made that plain: a folder is a
//! thing an operator drags onto a stick, and if the id travelled with it the second
//! machine to open the show would claim the first one's outputs and break the vector
//! clock's tie-break, which resolves concurrent writes by comparing node ids and
//! needs them to differ.
//!
//! So it lives with the machine's own configuration, beside `preferences.toml`.
//! Two stations on one machine is still an ordinary thing — it is what
//! `scripts/demo.sh --two` does — and each is *told* where its own is, by
//! `Config::identity` or `PULT_IDENTITY`. A path rather than an inference from the
//! show, because the show no longer implies one.

use std::path::{Path, PathBuf};

use pult_schema::events::operation::NodeId;
use tracing::{info, warn};
use uuid::Uuid;

/// Where this station's id lives when nobody said.
///
/// `PULT_IDENTITY` names it outright, which is how a second station on one machine
/// and every test get one of their own — and why it is an environment variable *and*
/// a `Config` field: an env var is one per process, and two stations inside one
/// program have to be told separately.
pub fn default_path() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PULT_IDENTITY") {
        return Some(PathBuf::from(named));
    }
    Some(crate::infra::preferences::config_dir()?.join("the-pult").join("node"))
}

/// This station's id: the one already recorded, or a new one written down.
///
/// Never fails. A station that cannot persist its identity still has to start —
/// losing output ownership on the next restart is bad, and refusing to open the
/// show at all is worse.
pub fn load_or_create(path: Option<&Path>) -> NodeId {
    let Some(path) = path.map(PathBuf::from).or_else(default_path) else {
        warn!("[identity] nowhere to keep this station's id; it will be a new station next time");
        return NodeId::new();
    };
    if let Some(id) = read(&path) {
        return id;
    }

    let id = NodeId::new();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, id.0.to_string()) {
        Ok(()) => info!("[identity] this station is {} ({})", id.0, path.display()),
        Err(e) => warn!(
            "[identity] could not write {}: {e} — this station will be a different one after a restart",
            path.display(),
        ),
    }
    id
}

/// Read a recorded id, or None if there is not a usable one there.
fn read(path: &Path) -> Option<NodeId> {
    let raw = std::fs::read_to_string(path).ok()?;
    match Uuid::parse_str(raw.trim()) {
        Ok(id) => Some(NodeId(id)),
        Err(e) => {
            // Replaced rather than treated as fatal: an unreadable identity file is
            // a station that has forgotten who it is, which is recoverable, and not
            // a reason to refuse to open the show.
            warn!("[identity] {} is not a node id ({e}); taking a new one", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path in a temporary directory, never actually created.
    fn an_identity(name: &str) -> (tempdir::Dir, PathBuf) {
        let dir = tempdir::Dir::new();
        let path = dir.path().join(name);
        (dir, path)
    }

    /// The smallest temporary directory that cleans up after itself.
    mod tempdir {
        pub struct Dir(std::path::PathBuf);

        impl Dir {
            pub fn new() -> Self {
                let path = std::env::temp_dir()
                    .join(format!("pult-identity-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(&path).expect("a temporary directory");
                Dir(path)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn a_station_keeps_its_id_across_restarts() {
        // Each call is one process start. This is the regression: an output that
        // named its station stopped sending after a restart, silently.
        let (_dir, path) = an_identity("node");

        let first = load_or_create(Some(&path));
        let second = load_or_create(Some(&path));

        assert_eq!(first, second);
    }

    #[test]
    fn two_stations_on_one_machine_are_two_stations() {
        // The ordinary two-node setup, and the vector clock's tie-break needs their
        // ids to differ. They are told apart by being *told*, since neither the show
        // nor the machine distinguishes them any more.
        let (_dir, one) = an_identity("one");
        let (_other, two) = an_identity("two");

        assert_ne!(load_or_create(Some(&one)), load_or_create(Some(&two)));
    }

    #[test]
    fn a_copied_show_does_not_clone_the_station_that_made_it() {
        // The reason it moved out of the bundle. Two stations sharing an id would
        // both claim the same outputs, and the clock's tie-break would have nothing
        // to break the tie with.
        let (_dir, path) = an_identity("node");
        let id = load_or_create(Some(&path));

        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), id.0.to_string());
    }

    #[test]
    fn a_directory_that_is_not_there_yet_is_made() {
        // The first run of a fresh install: nothing under the config directory
        // exists, and a station that would not write its id there would be a new
        // station every morning.
        let dir = tempdir::Dir::new();
        let path = dir.path().join("deep").join("deeper").join("node");

        let id = load_or_create(Some(&path));

        assert_eq!(id, load_or_create(Some(&path)));
    }

    #[test]
    fn a_corrupt_identity_is_replaced_rather_than_fatal() {
        let (_dir, path) = an_identity("node");
        std::fs::write(&path, "not a uuid at all").unwrap();

        let id = load_or_create(Some(&path));

        assert_eq!(id, load_or_create(Some(&path)), "and the replacement sticks");
    }

    #[test]
    fn a_station_that_cannot_write_its_id_still_starts() {
        // A read-only or missing directory. The station runs; it just will not be
        // the same station tomorrow, which is logged.
        let id = load_or_create(Some(std::path::Path::new("/no/such/directory/node")));
        assert_ne!(id.0, uuid::Uuid::nil());
    }
}
