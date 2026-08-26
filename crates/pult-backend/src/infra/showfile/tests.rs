use std::str::FromStr;

use pult_schema::{db, types::fixture::{Fixture, FixtureAddress}};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};

/// An empty in-memory database with no migrations run against it, so a test can
/// put a showfile from an older version of the schema into it first.
async fn blank_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

/// `fixtures` as it was written before `FixtureAddress` existed.
async fn write_a_legacy_showfile(pool: &SqlitePool) -> uuid::Uuid {
    sqlx::query(
        "CREATE TABLE fixtures (
            id TEXT NOT NULL PRIMARY KEY,
            name TEXT NOT NULL,
            fixture_type_id TEXT NOT NULL,
            universe INTEGER NOT NULL,
            dmx_address INTEGER NOT NULL,
            position TEXT
        )",
    )
    .execute(pool)
    .await
    .unwrap();

    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO fixtures (id, name, fixture_type_id, universe, dmx_address, position) VALUES (?1, ?2, ?3, 1, 7, NULL)")
        .bind(id.to_string())
        .bind("Backlight")
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(pool)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn a_showfile_written_before_fixture_addresses_still_opens() {
    let pool = blank_pool().await;
    let id = write_a_legacy_showfile(&pool).await;

    super::migrate_for_test(&pool).await.unwrap();

    let fixtures: Vec<Fixture> = db::get_all(&pool).await.unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].id, id);
    assert_eq!(
        fixtures[0].address,
        FixtureAddress::Dmx { universe: 1, address: 7 },
        "the old universe/address pair has to survive as an address",
    );
}

#[tokio::test]
async fn an_upgraded_showfile_can_be_written_to() {
    let pool = blank_pool().await;
    write_a_legacy_showfile(&pool).await;
    super::migrate_for_test(&pool).await.unwrap();

    // The old columns were NOT NULL and are no longer written, so an INSERT that
    // names only the current columns fails the constraint unless they are gone.
    let mut fixture: Fixture = db::get_all(&pool).await.unwrap().pop().unwrap();
    fixture.address = FixtureAddress::Dmx { universe: 3, address: 21 };
    db::upsert(&pool, &fixture).await.unwrap();

    let reloaded: Vec<Fixture> = db::get_all(&pool).await.unwrap();
    assert_eq!(reloaded[0].address, FixtureAddress::Dmx { universe: 3, address: 21 });
}

#[tokio::test]
async fn opening_the_same_showfile_twice_upgrades_it_once() {
    let pool = blank_pool().await;
    write_a_legacy_showfile(&pool).await;

    super::migrate_for_test(&pool).await.unwrap();
    super::migrate_for_test(&pool).await.unwrap();

    let fixtures: Vec<Fixture> = db::get_all(&pool).await.unwrap();
    assert_eq!(fixtures[0].address, FixtureAddress::Dmx { universe: 1, address: 7 });
}
