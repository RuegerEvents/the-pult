//! The shows this console has had open, most recent first.
//!
//! Beside `preferences.toml` rather than in it, and for a plain reason: a preference
//! is something an operator set, and this is something the console noticed. Mixing
//! the two would mean a file an operator edits by hand being rewritten under them
//! every time they open a show.
//!
//! A path that no longer exists is **kept and listed as missing**, not quietly
//! dropped. A show on a stick that is not plugged in is exactly the row somebody
//! wants to see, and forgetting it the moment it goes is how a list becomes useless.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::debug;

/// How many are kept. Ten is a couple of weeks of a busy season, and the eleventh
/// is something an operator finds by name rather than by memory.
const KEEP: usize = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Recent {
    #[serde(default, rename = "shows")]
    pub shows: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub last_opened: chrono::DateTime<chrono::Utc>,
}

/// Where the file lives, beside whatever `preferences.toml` this console reads.
pub fn path() -> Option<PathBuf> {
    let prefs = crate::infra::preferences::path()?;
    Some(prefs.with_file_name("recent.toml"))
}

pub fn load() -> Recent {
    let Some(path) = path() else { return Recent::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Recent::default() };
    toml::from_str(&raw).unwrap_or_else(|e| {
        debug!("[recent] {} is not readable ({e})", path.display());
        Recent::default()
    })
}

/// Put this show at the top, keeping the list at [`KEEP`].
///
/// Never fails loudly. A console that cannot write down what it opened has still
/// opened it, and refusing the show over a list would be the wrong half to give up.
pub fn remember(show: &Path) {
    let Some(file) = path() else { return };
    let mut recent = load();
    recent.shows.retain(|entry| entry.path != show);
    recent
        .shows
        .insert(0, Entry { path: show.to_path_buf(), last_opened: chrono::Utc::now() });
    recent.shows.truncate(KEEP);

    let write = || -> anyhow::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, toml::to_string_pretty(&recent)?)?;
        Ok(())
    };
    if let Err(e) = write() {
        debug!("[recent] could not write {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_recent_show_is_first_and_appears_once() {
        let _own = crate::infra::preferences::testing::own_file();

        remember(Path::new("/shows/A.pult"));
        remember(Path::new("/shows/B.pult"));
        remember(Path::new("/shows/A.pult"));

        let paths: Vec<_> = load().shows.into_iter().map(|e| e.path).collect();
        assert_eq!(paths, vec![PathBuf::from("/shows/A.pult"), PathBuf::from("/shows/B.pult")]);
    }

    #[test]
    fn the_list_is_bounded() {
        let _own = crate::infra::preferences::testing::own_file();

        for n in 0..KEEP + 5 {
            remember(&PathBuf::from(format!("/shows/{n}.pult")));
        }

        assert_eq!(load().shows.len(), KEEP);
        assert_eq!(load().shows[0].path, PathBuf::from(format!("/shows/{}.pult", KEEP + 4)));
    }
}
