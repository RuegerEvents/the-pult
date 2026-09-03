//! A show is a folder.
//!
//! One SQLite file used to be the whole of a show, with the assets as blobs inside
//! it and the station's identity in a sidecar beside it. Three things that were
//! wrong with that all have the same fix. A 256 MB GDTF inside the file makes every
//! `VACUUM`, every backup and every copy carry it; a version of a show has to be a
//! *snapshot* and a snapshot that duplicated the assets would cost the rig's whole
//! mesh library per save; and a showfile a user can put on a stick has to be one
//! thing they can drag, not a file and whatever happens to be beside it.
//!
//! So a show is `Name.pult/`:
//!
//! ```text
//! Name.pult/
//!   bundle.toml        # format, and the name a show with no row yet is seeded with
//!   show.db            # + -wal/-shm while it is open
//!   assets/<sha256>    # the bytes; the `assets` table keeps mime, length and when
//!   versions/<id>.db   # VACUUM INTO snapshots, sharing the assets above
//! ```
//!
//! What is deliberately *not* in here is the station's identity. It used to live
//! beside the showfile, which meant copying a show cloned the station that made it —
//! two stations with one id both claim the same outputs and break the vector clock's
//! tie-break. A folder is far easier to copy than a file was, so the identity moved
//! to the machine: see [`crate::infra::identity`].
//!
//! The travelling form is a `.pultz`, which is this folder zipped. See
//! [`super::travel`].

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// The extension a bundle directory carries, so a file browser can show it as one
/// thing and this console can tell a show from any other folder it is pointed at.
pub const EXTENSION: &str = "pult";

/// The travelling form: the same folder, zipped.
pub const TRAVEL_EXTENSION: &str = "pultz";

/// The manifest at the root of a bundle. Tiny on purpose: everything a show knows
/// about itself lives in `show.db`, and a second copy of any of it here would be a
/// second copy to go stale. What is left is the two things that have to be readable
/// *before* the database is opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// The layout this folder is in. One, and a folder claiming any other number is
    /// refused by name rather than opened and misread.
    pub format: u32,
    /// What to call the show if `show.db` has no row yet — a bundle this console has
    /// created but never opened, which is what `show.new` leaves behind between
    /// making the folder and the engine seeding the row.
    pub name: String,
}

/// The format this build writes and reads.
pub const FORMAT: u32 = 1;

/// An open show's folder. Holding one says where the parts are and nothing else:
/// it opens no database and starts nothing.
#[derive(Debug, Clone)]
pub struct Bundle {
    path: PathBuf,
    manifest: Manifest,
}

impl Bundle {
    /// Make a new bundle at `path`, which must not already exist.
    pub fn create(path: impl AsRef<Path>, name: &str) -> Result<Bundle> {
        let path = absolute(path.as_ref());
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        let manifest = Manifest { format: FORMAT, name: name.trim().to_string() };
        std::fs::create_dir_all(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        std::fs::create_dir_all(path.join("assets"))?;
        std::fs::create_dir_all(path.join("versions"))?;
        std::fs::write(path.join("bundle.toml"), toml::to_string_pretty(&manifest)?)?;
        info!("[bundle] created {}", path.display());
        Ok(Bundle { path, manifest })
    }

    /// Open an existing bundle.
    ///
    /// Two wrong answers are named rather than guessed at. A directory with no
    /// `bundle.toml` is somebody's Documents folder and opening it would scatter a
    /// show through it. A plain `.db` is the old single-file showfile, and the error
    /// says what became of those instead of failing on a missing manifest.
    pub fn open(path: impl AsRef<Path>) -> Result<Bundle> {
        let path = absolute(path.as_ref());
        if !path.exists() {
            bail!("there is no show at {}", path.display());
        }
        if path.is_file() {
            bail!(
                "{} is a file. A show is a folder now — `Name.{EXTENSION}` with the \
                 database and the assets inside it. Start a new show, or import a \
                 `.{TRAVEL_EXTENSION}`.",
                path.display(),
            );
        }
        let manifest_path = path.join("bundle.toml");
        if !manifest_path.exists() {
            bail!(
                "{} is not a show: it has no bundle.toml in it",
                path.display(),
            );
        }
        let manifest: Manifest = toml::from_str(&std::fs::read_to_string(&manifest_path)?)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        if manifest.format != FORMAT {
            bail!(
                "{} is a format {} show and this console reads {FORMAT}",
                path.display(),
                manifest.format,
            );
        }
        // A bundle that arrived without them — half a copy, or a zip written by
        // something that drops empty directories.
        std::fs::create_dir_all(path.join("assets"))?;
        std::fs::create_dir_all(path.join("versions"))?;
        Ok(Bundle { path, manifest })
    }

    /// Open it, or make it if it is not there.
    ///
    /// What `--show` does: naming a show on the command line is how a script asks for
    /// one, and refusing because it has not been created yet would make every script
    /// create it first.
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Bundle> {
        let path = path.as_ref();
        if path.exists() {
            Bundle::open(path)
        } else {
            Bundle::create(path, &name_from_path(path))
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What to call a show whose `show` row has not been written yet.
    pub fn seed_name(&self) -> &str {
        &self.manifest.name
    }

    pub fn db_path(&self) -> PathBuf {
        self.path.join("show.db")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.path.join("assets")
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.path.join("versions")
    }

    /// The snapshot file for one version.
    pub fn version_path(&self, id: uuid::Uuid) -> PathBuf {
        self.versions_dir().join(format!("{id}.db"))
    }

    /// Which versions this station actually holds a snapshot for.
    ///
    /// Not the same as the `versions` rows: those replicate, and a station that
    /// joined after a peer saved one never held that state and has nothing to write.
    pub fn versions_here(&self) -> Vec<uuid::Uuid> {
        let Ok(entries) = std::fs::read_dir(self.versions_dir()) else { return Vec::new() };
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                uuid::Uuid::parse_str(name.strip_suffix(".db")?).ok()
            })
            .collect()
    }

    /// Rename what a show with no row yet would be called. Used by save-as, whose
    /// copy is a different show before anything has opened it.
    pub fn set_seed_name(&mut self, name: &str) -> Result<()> {
        self.manifest.name = name.trim().to_string();
        std::fs::write(self.path.join("bundle.toml"), toml::to_string_pretty(&self.manifest)?)?;
        Ok(())
    }

    /// Everything a welcome screen wants to say about a show it has not opened.
    ///
    /// Read without the engine and without migrating anything: `show.db` is opened
    /// read-only, so listing a folder full of shows cannot change one. A file from
    /// another generation of the schema is *reported* rather than refused — a list
    /// that failed because one row in it was old would be a list nobody could use.
    pub async fn summary(&self) -> Summary {
        let mut summary = Summary {
            path: self.path.clone(),
            name: self.manifest.name.clone(),
            ..Summary::default()
        };
        summary.bytes = folder_bytes(&self.path);
        summary.versions = self.versions_here().len();

        match self.read_summary(&mut summary).await {
            Ok(()) => {}
            Err(e) => summary.problem = Some(format!("{e:#}")),
        }
        summary
    }

    async fn read_summary(&self, summary: &mut Summary) -> Result<()> {
        use sqlx::Row;
        use std::str::FromStr;

        let db = self.db_path();
        if !db.exists() {
            // A folder created but never opened. Everything else in the summary is
            // still true, and the counts are honestly zero.
            return Ok(());
        }
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
            "sqlite:{}?mode=ro",
            db.display()
        ))?
        // Read-only, so it must not try to create a journal beside the file either.
        .read_only(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;

        let generation: i64 =
            sqlx::query_scalar("PRAGMA user_version").fetch_one(&pool).await.unwrap_or(0);
        if generation != super::SCHEMA_GENERATION {
            summary.made_by_another_build = true;
            // Nothing below can be trusted to parse, but the counts are integers out
            // of tables whose names have not moved, so they are still worth having.
        }

        if let Ok(row) = sqlx::query("SELECT name, created_at FROM show LIMIT 1")
            .fetch_optional(&pool)
            .await
        {
            if let Some(row) = row {
                if let Ok(name) = row.try_get::<String, _>("name") {
                    summary.name = name;
                }
                summary.created_at = row.try_get::<String, _>("created_at").ok();
            }
        }
        summary.fixtures = count(&pool, "fixtures").await;
        summary.cues = count(&pool, "cues").await;
        pool.close().await;
        Ok(())
    }
}

async fn count(pool: &sqlx::SqlitePool, table: &str) -> u64 {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        .max(0) as u64
}

/// What a show looks like from outside, before anything opens it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub path: PathBuf,
    pub name: String,
    pub created_at: Option<String>,
    pub fixtures: u64,
    pub cues: u64,
    pub versions: usize,
    /// Everything in the folder, so an operator can see which show is the large one.
    pub bytes: u64,
    /// `PRAGMA user_version` disagrees with this build. Said rather than refused: a
    /// welcome screen listing ten shows must not fail because one of them is old.
    pub made_by_another_build: bool,
    /// Whatever went wrong reading it, where something did.
    pub problem: Option<String>,
}

/// Every byte under a directory, symlinks not followed.
fn folder_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// A path without a `.` or a `..` in it, so a station reporting where its show is
/// reports somewhere a person can find, and the disk figure matches a mount point.
///
/// The *directory* is resolved and the last component re-joined, because
/// canonicalizing a bundle that is about to be created fails.
pub fn absolute(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(PathBuf::from).unwrap_or_default();
    match parent.canonicalize() {
        Ok(resolved) => resolved.join(name),
        Err(_) => std::env::current_dir().map(|cwd| cwd.join(path)).unwrap_or_else(|_| path.into()),
    }
}

/// What to call a show whose folder is all anybody has told us: `Big Rig.pult` is
/// the show "Big Rig".
pub fn name_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Untitled Show".to_string())
}

/// A show's name as a folder name: no separators, no leading dot, not empty.
///
/// Deliberately not a slug — an operator naming a show *Hänsel & Gretel* should find
/// a folder called that, not `hansel-gretel`. Only what a filesystem or a shell would
/// actually misread is replaced.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    let cleaned = if cleaned.is_empty() { "Untitled Show".to_string() } else { cleaned };
    format!("{}.{EXTENSION}", cleaned.chars().take(120).collect::<String>())
}

/// Where in `dir` a show called `name` should go, with a number on the end if that
/// name is taken.
///
/// A number rather than an overwrite, and a number rather than a refusal: two shows
/// honestly called *Rehearsal* is an ordinary thing, and either alternative loses
/// somebody's work or makes them think of a different word.
pub fn free_path_in(dir: &Path, name: &str) -> PathBuf {
    let base = folder_name(name);
    let candidate = dir.join(&base);
    if !candidate.exists() {
        return candidate;
    }
    let stem = base.strip_suffix(&format!(".{EXTENSION}")).unwrap_or(&base).to_string();
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} {n}.{EXTENSION}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} {}.{EXTENSION}", uuid::Uuid::new_v4()))
}

/// Where this console keeps the shows nobody gave a path for.
///
/// The platform's data directory, the same resolution preferences use, so a console
/// started from the dock and one started from a shell open the same shows. A station
/// preference overrides it; `PULT_SHOWS` overrides that, which is how the demo and
/// the tests get a directory of their own.
pub fn default_shows_dir() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PULT_SHOWS") {
        return Some(PathBuf::from(named));
    }
    Some(crate::infra::preferences::config_dir()?.join("the-pult").join("shows"))
}

/// Every bundle directly inside a directory, whatever else is in it.
pub fn bundles_in(dir: &Path) -> Vec<Bundle> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<Bundle> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| match Bundle::open(entry.path()) {
            Ok(bundle) => Some(bundle),
            Err(e) => {
                // A folder that is not a show is the ordinary case in a directory
                // somebody also keeps other things in, so this is not a warning.
                tracing::trace!("[bundle] {} is not a show: {e:#}", entry.path().display());
                None
            }
        })
        .collect();
    found.sort_by(|a, b| a.path().cmp(b.path()));
    found
}

/// Make sure the shows directory exists, saying so rather than failing if it cannot.
pub fn ensure_dir(dir: &Path) -> Option<PathBuf> {
    match std::fs::create_dir_all(dir) {
        Ok(()) => Some(dir.to_path_buf()),
        Err(e) => {
            warn!("[bundle] cannot use {} for shows: {e}", dir.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, removed when it goes out of scope.
    pub(crate) struct Dir(PathBuf);

    impl Dir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!("pult-bundle-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("a temporary directory");
            Dir(path)
        }
        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_new_bundle_has_the_parts_a_show_is_made_of() {
        let dir = Dir::new();
        let bundle = Bundle::create(dir.path().join("Big Rig.pult"), "Big Rig").unwrap();

        assert!(bundle.assets_dir().is_dir());
        assert!(bundle.versions_dir().is_dir());
        assert_eq!(bundle.seed_name(), "Big Rig");
        assert_eq!(Bundle::open(bundle.path()).unwrap().seed_name(), "Big Rig");
    }

    #[test]
    fn a_folder_that_is_not_a_show_is_refused_by_name() {
        let dir = Dir::new();
        let err = Bundle::open(dir.path()).unwrap_err().to_string();
        assert!(err.contains("bundle.toml"), "{err}");
    }

    #[test]
    fn the_old_single_file_showfile_is_told_what_became_of_it() {
        // The one error somebody upgrading actually hits, so it says what to do
        // rather than complaining about a missing manifest inside a file.
        let dir = Dir::new();
        let old = dir.path().join("show.db");
        std::fs::write(&old, b"SQLite format 3\0").unwrap();

        let err = Bundle::open(&old).unwrap_err().to_string();
        assert!(err.contains("a folder now"), "{err}");
    }

    #[test]
    fn a_name_a_filesystem_would_misread_becomes_one_it_would_not() {
        assert_eq!(folder_name("Hänsel & Gretel"), "Hänsel & Gretel.pult");
        assert_eq!(folder_name("Act 1/2"), "Act 1-2.pult");
        assert_eq!(folder_name("   "), "Untitled Show.pult");
        assert_eq!(folder_name("../etc"), "-etc.pult", "no separators and no leading dots");
    }

    #[test]
    fn a_taken_name_gets_a_number_rather_than_an_overwrite() {
        let dir = Dir::new();
        let first = free_path_in(dir.path(), "Rehearsal");
        Bundle::create(&first, "Rehearsal").unwrap();

        let second = free_path_in(dir.path(), "Rehearsal");
        assert_ne!(first, second);
        assert_eq!(second.file_name().unwrap(), "Rehearsal 2.pult");
    }

    #[tokio::test]
    async fn a_show_that_was_never_opened_still_summarises() {
        let dir = Dir::new();
        let bundle = Bundle::create(dir.path().join("Fresh.pult"), "Fresh").unwrap();

        let summary = bundle.summary().await;
        assert_eq!(summary.name, "Fresh");
        assert_eq!(summary.fixtures, 0);
        assert_eq!(summary.versions, 0);
        assert!(summary.problem.is_none(), "{:?}", summary.problem);
    }

    #[tokio::test]
    async fn a_summary_reads_the_show_without_the_engine() {
        let dir = Dir::new();
        let bundle = Bundle::create(dir.path().join("Real.pult"), "Real").unwrap();
        let pool = super::super::open(&bundle.db_path()).await.unwrap();
        sqlx::query(
            "INSERT INTO show (id, name, created_at, history_depth, home_fade_ms, \
             haze_density, haze_turbulence) VALUES (?, 'Panto', '2026-01-01T00:00:00Z', 500, 0, 0.2, 0.5)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let summary = bundle.summary().await;
        assert_eq!(summary.name, "Panto", "the row wins over the manifest's seed");
        assert!(!summary.made_by_another_build);
        assert!(summary.bytes > 0);
    }

    #[test]
    fn only_the_snapshots_this_station_holds_are_listed() {
        let dir = Dir::new();
        let bundle = Bundle::create(dir.path().join("V.pult"), "V").unwrap();
        let id = uuid::Uuid::new_v4();
        std::fs::write(bundle.version_path(id), b"not really a database").unwrap();
        std::fs::write(bundle.versions_dir().join("notes.txt"), b"ignored").unwrap();

        assert_eq!(bundle.versions_here(), vec![id]);
    }
}
