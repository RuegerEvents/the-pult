//! This station's own identity, kept across restarts.
//!
//! `NodeId` used to be generated fresh every process start, which was harmless
//! while nothing wrote it down. It stopped being harmless when an output began
//! naming the station that sends it: after a restart the station had a new id, the
//! saved output belonged to nobody, and it silently stopped sending.
//!
//! The id is stored beside the showfile rather than inside it, because it belongs
//! to *this station* and not to the show. Copying a showfile to another machine
//! must not clone the identity — two stations sharing an id would both claim the
//! same outputs and would break the vector clock's tie-break, which resolves
//! concurrent writes by comparing node ids and needs them to differ.
//!
//! Beside the showfile rather than somewhere machine-wide, because two backends on
//! one machine are two stations, and they each have their own showfile.

use std::path::{Path, PathBuf};

use pult_schema::events::operation::NodeId;
use tracing::{info, warn};
use uuid::Uuid;

/// Where the id for a given showfile lives: `show.db` → `show.db.node`.
pub fn identity_path(showfile: &str) -> PathBuf {
    PathBuf::from(format!("{showfile}.node"))
}

/// This station's id: the one already recorded, or a new one written down.
///
/// Never fails. A station that cannot persist its identity still has to start —
/// losing output ownership on the next restart is bad, and refusing to open the
/// show at all is worse.
pub fn load_or_create(showfile: &str) -> NodeId {
    let path = identity_path(showfile);
    if let Some(id) = read(&path) {
        return id;
    }

    let id = NodeId::new();
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

    /// A unique showfile path in a temporary directory, never actually created.
    fn a_showfile(name: &str) -> (tempdir::Dir, String) {
        let dir = tempdir::Dir::new();
        let path = dir.path().join(name).to_string_lossy().into_owned();
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
        let (_dir, showfile) = a_showfile("show.db");

        let first = load_or_create(&showfile);
        let second = load_or_create(&showfile);

        assert_eq!(first, second);
    }

    #[test]
    fn two_showfiles_on_one_machine_are_two_stations() {
        // Two backends on one machine is the ordinary two-node setup, and the
        // vector clock's tie-break needs their ids to differ.
        let (_dir, one) = a_showfile("one.db");
        let (_other, two) = a_showfile("two.db");

        assert_ne!(load_or_create(&one), load_or_create(&two));
    }

    #[test]
    fn the_id_lives_beside_the_showfile_and_not_inside_it() {
        // Copying a showfile to another machine must not clone the identity.
        let (_dir, showfile) = a_showfile("show.db");
        let id = load_or_create(&showfile);

        let sidecar = identity_path(&showfile);
        assert!(sidecar.exists());
        assert!(!std::path::Path::new(&showfile).exists(), "the showfile itself is untouched");
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap().trim(), id.0.to_string());
    }

    #[test]
    fn a_corrupt_identity_is_replaced_rather_than_fatal() {
        let (_dir, showfile) = a_showfile("show.db");
        std::fs::write(identity_path(&showfile), "not a uuid at all").unwrap();

        let id = load_or_create(&showfile);

        assert_eq!(id, load_or_create(&showfile), "and the replacement sticks");
    }

    #[test]
    fn a_station_that_cannot_write_its_id_still_starts() {
        // A read-only or missing directory. The station runs; it just will not be
        // the same station tomorrow, which is logged.
        let id = load_or_create("/no/such/directory/show.db");
        assert_ne!(id.0, uuid::Uuid::nil());
    }
}
