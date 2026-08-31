//! Where a carried plugin's bytes are unpacked, keyed by their own digest.
//!
//! A cache entry is immutable by construction — the directory's name is the
//! sha256 of the bundle that made it — so a hit needs no validation, and two
//! shows carrying the same plugin share one directory rather than one each.
//!
//! Station-local and never replicated: it holds nothing a peer does not already
//! have the bundle for, and it can be deleted by hand with no loss beyond the
//! next unpack.

use std::path::PathBuf;

/// The cache root, or `None` if this machine has nowhere to keep one.
///
/// `PULT_PLUGIN_CACHE` names it outright, the way `PULT_PREFERENCES` does: it is
/// how the tests get a root of their own, and how two stations on one machine
/// can be kept apart.
pub fn root() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PULT_PLUGIN_CACHE") {
        return Some(PathBuf::from(named));
    }
    Some(crate::infra::preferences::config_dir()?.join("the-pult").join("plugin-cache"))
}

/// Where a bundle with this digest lives, unpacked.
pub fn dir_for(sha256: &str) -> Option<PathBuf> {
    // The digest goes into a path, so it may only be what a digest is. A roster
    // row is replicated data: it arrives from a peer, and a peer is not a
    // reason to trust a string that is about to be joined onto a directory.
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(root()?.join(sha256))
}

/// Is this bundle already unpacked?
pub fn holds(sha256: &str) -> bool {
    dir_for(sha256).is_some_and(|dir| dir.join(super::bundle::MANIFEST_NAME).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_that_is_not_a_digest_names_no_directory() {
        // The roster replicates, so this string arrives from a peer.
        assert!(dir_for("../../../etc").is_none());
        assert!(dir_for("").is_none());
        assert!(dir_for("nothex".repeat(9).as_str()).is_none());
        assert!(dir_for(&"a".repeat(63)).is_none(), "too short is not a sha256 either");
        assert!(dir_for(&"a".repeat(64)).is_some());
    }
}
