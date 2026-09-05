//! Where the archives this station committed are kept.
//!
//! An `MVR_COMMIT` announces a `FileUUID` and a `FileSize` and nothing else; the bytes
//! move only when somebody sends `MVR_REQUEST`, possibly much later, possibly from a
//! station that joins tomorrow and reads our whole commit history out of `MVR_JOIN`.
//! So an archive we announced has to still exist, byte for byte, with the size we
//! claimed.
//!
//! **Not the asset store.** Content-addressed assets are PERSISTED: they replicate to
//! peers and ride in the `.pultz`. A megabyte-scale archive per commit, kept for ever
//! in the showfile and pushed across the link carrying the show, is a cost nobody
//! asked for — and a commit is this *station's* traffic rather than the show's
//! content. So it is a directory beside `preferences.toml`, LOCAL like the log file.
//!
//! **And not regenerated on demand.** Answering a request for last Tuesday's commit by
//! exporting today's rig is a lie with the right uuid on it.
//!
//! The limit is a count of commits rather than a byte budget, because a count is what
//! the panel shows and what an operator can reason about. Where the bytes go when it
//! prunes is the same place they came from: a commit that has aged out is simply one
//! this station can no longer serve, and it stops being announced at all.

use std::path::{Path, PathBuf};

use tracing::{debug, warn};
use uuid::Uuid;

/// One archive this station committed, as the cache remembers it.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedCommit {
    pub file_uuid: Uuid,
    pub file_size: u64,
    pub comment: String,
    pub file_name: String,
    /// Console milliseconds at the moment of committing. The prune order.
    pub at_ms: i64,
}

/// The archives, and the note beside each saying what it was.
///
/// Two files per commit: `<uuid>.mvr` and `<uuid>.json`. The note is what lets a
/// restarted station still announce a commit it made yesterday — without it, the cache
/// would be a directory of files with no comments, no sizes and no order.
pub struct CommitCache {
    dir: PathBuf,
    keep: usize,
}

impl CommitCache {
    pub fn new(dir: PathBuf, keep: usize) -> Self {
        CommitCache { dir, keep: keep.max(1) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// What this station holds, oldest first.
    ///
    /// Read off the disk every time rather than held in memory: the answer is a few
    /// dozen small files, it is asked once per commit and once per request, and a
    /// cached copy is a second answer to "what can this station actually serve" — the
    /// one question this module exists to answer truthfully.
    pub fn list(&self) -> Vec<CachedCommit> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut commits: Vec<CachedCommit> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| {
                let bytes = std::fs::read(e.path()).ok()?;
                let note: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
                let file_uuid: Uuid = note.get("file_uuid")?.as_str()?.parse().ok()?;
                // A note with no archive beside it is not a commit anybody can be
                // served, so it is not one this station announces.
                let archive = self.archive_path(file_uuid);
                let file_size = std::fs::metadata(&archive).ok()?.len();
                Some(CachedCommit {
                    file_uuid,
                    file_size,
                    comment: note.get("comment").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    file_name: note
                        .get("file_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    at_ms: note.get("at_ms").and_then(serde_json::Value::as_i64).unwrap_or(0),
                })
            })
            .collect();
        commits.sort_by_key(|c| c.at_ms);
        commits
    }

    /// Keep an archive, and prune the oldest past the limit.
    pub fn put(&self, commit: &CachedCommit, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.archive_path(commit.file_uuid), bytes)?;
        let note = serde_json::json!({
            "file_uuid": commit.file_uuid,
            "comment": commit.comment,
            "file_name": commit.file_name,
            "at_ms": commit.at_ms,
        });
        std::fs::write(self.note_path(commit.file_uuid), serde_json::to_vec_pretty(&note)?)?;
        self.prune();
        Ok(())
    }

    /// The bytes of one commit, if this station has them.
    pub fn get(&self, file_uuid: Uuid) -> Option<Vec<u8>> {
        std::fs::read(self.archive_path(file_uuid)).ok()
    }

    /// The most recent commit, which is what a request with no `FileUUID` asks for.
    pub fn latest(&self) -> Option<CachedCommit> {
        self.list().pop()
    }

    fn archive_path(&self, file_uuid: Uuid) -> PathBuf {
        self.dir.join(format!("{file_uuid}.mvr"))
    }

    fn note_path(&self, file_uuid: Uuid) -> PathBuf {
        self.dir.join(format!("{file_uuid}.json"))
    }

    fn prune(&self) {
        let commits = self.list();
        let Some(over) = commits.len().checked_sub(self.keep) else { return };
        for commit in commits.iter().take(over) {
            debug!("[xchange] pruning commit {} to keep {}", commit.file_uuid, self.keep);
            if let Err(e) = std::fs::remove_file(self.archive_path(commit.file_uuid)) {
                warn!("[xchange] could not prune {}: {e}", commit.file_uuid);
            }
            let _ = std::fs::remove_file(self.note_path(commit.file_uuid));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_commit(n: u128, at_ms: i64) -> CachedCommit {
        CachedCommit {
            file_uuid: Uuid::from_u128(n),
            file_size: 4,
            comment: format!("commit {n}"),
            file_name: format!("show-{n}.mvr"),
            at_ms,
        }
    }

    /// The house pattern for a directory a test owns: named for the run, left
    /// behind if a test panics, which is what makes a failure inspectable.
    fn a_cache(keep: usize) -> CommitCache {
        let dir = std::env::temp_dir().join(format!("pult-xchange-{}", Uuid::new_v4()));
        CommitCache::new(dir, keep)
    }

    #[test]
    fn what_goes_in_comes_back_out() {
        let cache = a_cache(4);
        cache.put(&a_commit(1, 100), b"abcd").unwrap();
        assert_eq!(cache.get(Uuid::from_u128(1)).as_deref(), Some(&b"abcd"[..]));
        assert_eq!(cache.list().len(), 1);
        assert_eq!(cache.list()[0].comment, "commit 1");
    }

    #[test]
    fn the_size_is_the_file_s_own_and_not_what_was_claimed() {
        let cache = a_cache(4);
        let mut commit = a_commit(1, 100);
        commit.file_size = 9999;
        cache.put(&commit, b"abcd").unwrap();
        // What is announced has to be what a requester will actually receive.
        assert_eq!(cache.list()[0].file_size, 4);
    }

    #[test]
    fn the_oldest_is_pruned_past_the_limit() {
        let cache = a_cache(2);
        for n in 1..=4u128 {
            cache.put(&a_commit(n, n as i64 * 100), b"abcd").unwrap();
        }
        let kept: Vec<u128> = cache.list().iter().map(|c| c.file_uuid.as_u128()).collect();
        assert_eq!(kept, vec![3, 4], "the two most recent, oldest first");
        assert_eq!(cache.get(Uuid::from_u128(1)), None, "and the pruned one is gone");
    }

    #[test]
    fn the_latest_is_what_a_request_with_no_uuid_gets() {
        let cache = a_cache(4);
        cache.put(&a_commit(1, 100), b"a").unwrap();
        cache.put(&a_commit(2, 300), b"b").unwrap();
        cache.put(&a_commit(3, 200), b"c").unwrap();
        assert_eq!(cache.latest().unwrap().file_uuid, Uuid::from_u128(2));
    }

    /// A note whose archive is gone is not a commit this station can serve, so it is
    /// not one it announces. Half a commit is worse than none: it is an offer that
    /// fails when somebody takes it up.
    #[test]
    fn a_note_with_no_archive_is_not_a_commit() {
        let cache = a_cache(4);
        cache.put(&a_commit(1, 100), b"abcd").unwrap();
        std::fs::remove_file(cache.dir().join(format!("{}.mvr", Uuid::from_u128(1)))).unwrap();
        assert!(cache.list().is_empty());
    }

    #[test]
    fn a_cache_that_was_never_written_is_empty_rather_than_an_error() {
        let cache = CommitCache::new(PathBuf::from("/nowhere/at/all/xchange"), 4);
        assert!(cache.list().is_empty());
        assert!(cache.latest().is_none());
    }
}
