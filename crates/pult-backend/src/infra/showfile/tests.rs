//! What opening a showfile does, now that opening one never migrates it.
//!
//! The three things worth proving are the three ways a file meets a build it was not
//! written by: it is new and gets stamped, it is from another generation and is
//! refused, or a field was added to it that every row has to have a value for and
//! nothing filled in. The fourth case — a field that is allowed to be absent — is the
//! one that still just works, and is here so that it goes on working.

use std::str::FromStr;

use pult_schema::{db, types::fixture::Fixture};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};

/// An empty in-memory database with no migrations run against it, so a test can put
/// a showfile of its own shape into it first.
async fn blank_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

async fn user_version(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("PRAGMA user_version").fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn a_fresh_showfile_is_stamped_with_the_generation_that_made_it() {
    let pool = blank_pool().await;

    super::migrate_for_test(&pool).await.unwrap();

    assert_eq!(user_version(&pool).await, super::SCHEMA_GENERATION);
}

#[tokio::test]
async fn opening_the_same_showfile_twice_is_fine() {
    let pool = blank_pool().await;

    super::migrate_for_test(&pool).await.unwrap();
    super::migrate_for_test(&pool).await.expect("its own stamp is the one it reads");
}

/// An unstamped file is every showfile written before the stamp existed, which is
/// every showfile written before this. It is refused rather than read hopefully.
#[tokio::test]
async fn a_showfile_from_before_the_stamp_is_refused() {
    let pool = blank_pool().await;
    sqlx::query("CREATE TABLE fixtures (id TEXT NOT NULL PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();

    let error = super::migrate_for_test(&pool).await.unwrap_err().to_string();

    assert!(error.contains("generation 0"), "it says what the file is: {error}");
    assert!(error.contains("start a fresh one"), "and what to do about it: {error}");
}

#[tokio::test]
async fn a_showfile_from_a_later_build_is_refused_too() {
    let pool = blank_pool().await;
    super::migrate_for_test(&pool).await.unwrap();
    sqlx::query("PRAGMA user_version = 99").execute(&pool).await.unwrap();

    let error = super::migrate_for_test(&pool).await.unwrap_err().to_string();

    assert!(error.contains("generation 99"), "{error}");
}

/// The hole the additive pass leaves, and the reason this check exists beside the
/// stamp: a new non-`Option` field arrives as a nullable column, every existing row
/// holds NULL in it, and `from_columns` unwraps each column on its own — so the show
/// panics while it is being opened unless something looks first.
///
/// Simulated by emptying a column that is already required, which is exactly the
/// state such a file is in.
#[tokio::test]
async fn a_column_every_row_needs_and_nothing_filled_in_is_refused_by_name() {
    let pool = blank_pool().await;
    // A file whose stamp says this build can read it, written before `home_values`
    // was a field: the additive pass will add the column nullable and no row has a
    // value for it.
    sqlx::query(
        "CREATE TABLE fixtures (
            id TEXT NOT NULL PRIMARY KEY,
            name TEXT NOT NULL,
            fixture_type_id TEXT NOT NULL,
            address TEXT NOT NULL,
            position TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO fixtures VALUES (?1, 'Backlight', ?2, ?3, NULL)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(AN_ADDRESS)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(&format!("PRAGMA user_version = {}", super::SCHEMA_GENERATION))
        .execute(&pool)
        .await
        .unwrap();

    let error = super::migrate_for_test(&pool).await.unwrap_err().to_string();

    assert!(error.contains("fixtures.home_values"), "it names the column: {error}");
}

/// And the case that still works without anybody doing anything: a field that may be
/// absent. `position` is `Option`, so its column is added to a file that never had
/// one and every row reads back with nothing in it.
#[tokio::test]
async fn a_field_that_may_be_absent_is_added_to_a_file_that_never_had_it() {
    let pool = blank_pool().await;
    super::migrate_for_test(&pool).await.unwrap();
    let id = a_fixture_row(&pool).await;
    sqlx::query("ALTER TABLE fixtures DROP COLUMN position").execute(&pool).await.unwrap();

    super::migrate_for_test(&pool).await.expect("an absent optional column is not a refusal");

    let fixtures: Vec<Fixture> = db::get_all(&pool).await.unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].id, id);
    assert!(fixtures[0].position.is_none(), "nothing to say, rather than nothing at all");
}

const AN_ADDRESS: &str = r#"{"Dmx":{"mode":"Default","breaks":[{"universe":1,"address":7}]}}"#;

/// One fixture, written through the columns the current build has.
async fn a_fixture_row(pool: &SqlitePool) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO fixtures (id, name, fixture_type_id, address, position, home_values) \
         VALUES (?1, 'Backlight', ?2, ?3, NULL, '{}')",
    )
    .bind(id.to_string())
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(AN_ADDRESS)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// An optional *number* survives the round trip through SQLite.
///
/// Worth its own test because the way it failed was silent. Every `Option` field is
/// stored as JSON text, but the generated column used to take the *inner* type's
/// affinity — so `Option<u32>` declared INTEGER, SQLite converted the text `101` to
/// the number 101 on the way in, the text-based reader found no text on the way out,
/// and the field read back as `None`. No error, no bad data, just a value gone.
#[tokio::test]
async fn an_optional_number_comes_back_as_the_number_it_was() {
    use pult_schema::types::fixture::Fixture;

    let pool = blank_pool().await;
    super::migrate_for_test(&pool).await.unwrap();

    let written = Fixture {
        id: uuid::Uuid::new_v4(),
        name: "Mover 1".into(),
        fixture_number: Some(101),
        unit_number: Some(7),
        ..Fixture::default()
    };
    db::upsert(&pool, &written).await.unwrap();

    let read: Vec<Fixture> = db::get_all(&pool).await.unwrap();

    assert_eq!(read[0].fixture_number, Some(101));
    assert_eq!(read[0].unit_number, Some(7));
}

/// And an absent one is still absent, which is the case that always worked.
#[tokio::test]
async fn an_optional_number_nobody_set_is_still_nothing() {
    use pult_schema::types::fixture::Fixture;

    let pool = blank_pool().await;
    super::migrate_for_test(&pool).await.unwrap();
    db::upsert(&pool, &Fixture { id: uuid::Uuid::new_v4(), ..Fixture::default() }).await.unwrap();

    let read: Vec<Fixture> = db::get_all(&pool).await.unwrap();

    assert_eq!(read[0].fixture_number, None);
}
