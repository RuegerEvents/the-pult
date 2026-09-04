//! A show that goes away and comes back is the same show.

use super::*;

/// A directory of this test's own, removed when it goes out of scope.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new() -> Dir {
        let path = std::env::temp_dir().join(format!("pult-travel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Dir(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A show with a row, an asset and a version in it.
async fn a_show(dir: &Dir, name: &str) -> (Bundle, SqlitePool) {
    let bundle = Bundle::create(dir.path().join(format!("{name}.pult")), name).unwrap();
    let pool = super::super::open(&bundle.db_path()).await.unwrap();
    sqlx::query(
        "INSERT INTO show (id, name, created_at, history_depth, home_fade_ms, \
         haze_density, haze_turbulence, fade_curves) VALUES (?, ?, '2026-01-01T00:00:00Z', 500, 0, 0.2, 0.5, '{}')",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(name)
    .execute(&pool)
    .await
    .unwrap();
    std::fs::write(bundle.assets_dir().join("a".repeat(64)), b"a drawing").unwrap();
    std::fs::write(bundle.version_path(uuid::Uuid::new_v4()), b"a snapshot").unwrap();
    (bundle, pool)
}

#[tokio::test]
async fn a_show_that_travels_and_comes_back_is_the_same_show() {
    let dir = Dir::new();
    let (bundle, pool) = a_show(&dir, "Panto").await;

    let zipped = export(&bundle, &pool, false).await.unwrap();
    pool.close().await;

    let elsewhere = Dir::new();
    let back = import(&zipped, elsewhere.path()).unwrap();

    assert_eq!(back.seed_name(), "Panto");
    assert!(back.db_path().is_file());
    assert!(
        back.assets_dir().join("a".repeat(64)).is_file(),
        "the assets travel: without them a rig has no drawings in it",
    );

    let pool = super::super::open(&back.db_path()).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT name FROM show").fetch_one(&pool).await.unwrap();
    assert_eq!(name, "Panto");
    pool.close().await;
}

#[tokio::test]
async fn the_versions_stay_behind_unless_they_are_asked_for() {
    // They are one whole database each, and somebody sending a show to a colleague
    // is sending the show, not their afternoon's undo history.
    let dir = Dir::new();
    let (bundle, pool) = a_show(&dir, "Heavy").await;

    let without = import(&export(&bundle, &pool, false).await.unwrap(), Dir::new().path()).unwrap();
    assert!(without.versions_here().is_empty());

    let elsewhere = Dir::new();
    let with = import(&export(&bundle, &pool, true).await.unwrap(), elsewhere.path()).unwrap();
    assert_eq!(with.versions_here().len(), 1);
    pool.close().await;
}

#[tokio::test]
async fn two_exports_of_one_show_are_the_same_bytes() {
    // So that a reader diffing them sees what changed rather than what the
    // filesystem felt like listing first.
    let dir = Dir::new();
    let (bundle, pool) = a_show(&dir, "Stable").await;
    for n in 0..4 {
        std::fs::write(bundle.assets_dir().join(format!("{n}").repeat(64)), b"x").unwrap();
    }

    let once = export(&bundle, &pool, true).await.unwrap();
    let twice = export(&bundle, &pool, true).await.unwrap();
    pool.close().await;

    assert_eq!(once, twice);
}

#[tokio::test]
async fn a_taken_name_gets_a_number_rather_than_an_overwrite() {
    let dir = Dir::new();
    let (bundle, pool) = a_show(&dir, "Rehearsal").await;
    let zipped = export(&bundle, &pool, false).await.unwrap();
    pool.close().await;

    let elsewhere = Dir::new();
    let first = import(&zipped, elsewhere.path()).unwrap();
    let second = import(&zipped, elsewhere.path()).unwrap();

    assert_ne!(first.path(), second.path());
    assert_eq!(second.path().file_name().unwrap(), "Rehearsal 2.pult");
}

#[test]
fn a_zip_that_is_not_a_show_is_refused_and_leaves_nothing_behind() {
    let dir = Dir::new();
    let err = import(b"not a zip at all", dir.path()).unwrap_err().to_string();
    assert!(err.contains(".pultz"), "{err}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn an_entry_naming_a_path_outside_the_show_does_not_get_one() {
    // A `.pultz` is somebody else's file, and an entry called `../../.ssh/authorized_keys`
    // is the oldest trick there is.
    use zip::write::SimpleFileOptions;

    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default();
        zip.start_file("bundle.toml", options).unwrap();
        zip.write_all(b"format = 1\nname = \"Sneaky\"\n").unwrap();
        zip.start_file("../escaped.txt", options).unwrap();
        zip.write_all(b"nope").unwrap();
        zip.start_file("show.db", options).unwrap();
        zip.write_all(b"SQLite format 3\0").unwrap();
        zip.finish().unwrap();
    }

    let dir = Dir::new();
    let made = import(&buffer.into_inner(), dir.path()).unwrap();

    assert!(made.db_path().is_file(), "the show itself still arrives");
    assert!(!dir.path().join("escaped.txt").exists());
    assert!(!made.path().join("escaped.txt").exists());
}
