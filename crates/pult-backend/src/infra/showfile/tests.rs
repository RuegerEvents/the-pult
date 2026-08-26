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

// ── Triggers become flows ─────────────────────────────────────────────────────

/// `triggers` as it was written when a rule was one row.
async fn write_a_showfile_with_triggers(pool: &SqlitePool) -> (uuid::Uuid, uuid::Uuid) {
    sqlx::query(
        "CREATE TABLE triggers (
            id TEXT NOT NULL PRIMARY KEY,
            name TEXT NOT NULL,
            source TEXT NOT NULL,
            condition TEXT NOT NULL,
            action TEXT NOT NULL,
            delay_ms INTEGER NOT NULL,
            enabled INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await
    .unwrap();

    let fixture_id = uuid::Uuid::new_v4();
    let sequence_id = uuid::Uuid::new_v4();
    let source = format!(
        r#"{{"Parameter":{{"fixture_id":"{fixture_id}","parameter":{{"Contact":0}}}}}}"#
    );
    let action = format!(r#"{{"GoNext":{{"sequence_id":"{sequence_id}"}}}}"#);

    // One without a delay and one with, because they migrate to chains of
    // different lengths.
    for (name, delay) in [("Doorbell", 0), ("Porch light", 2500)] {
        sqlx::query(
            "INSERT INTO triggers (id, name, source, condition, action, delay_ms, enabled) \
             VALUES (?, ?, ?, '\"RisingEdge\"', ?, ?, 1)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(name)
        .bind(&source)
        .bind(&action)
        .bind(delay)
        .execute(pool)
        .await
        .unwrap();
    }

    (fixture_id, sequence_id)
}

#[tokio::test]
async fn a_showfile_written_when_a_rule_was_one_row_opens_as_flows() {
    use pult_schema::types::flow::{
        Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource,
    };

    let pool = blank_pool().await;
    let (fixture_id, sequence_id) = write_a_showfile_with_triggers(&pool).await;

    super::migrate_for_test(&pool).await.unwrap();

    let mut flows: Vec<Flow> = db::get_all(&pool).await.unwrap();
    flows.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        flows.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        vec!["Doorbell", "Porch light"],
        "a rule keeps its name; only the drawing changed",
    );
    assert!(flows.iter().all(|f| f.enabled));

    let nodes: Vec<FlowNode> = db::get_all(&pool).await.unwrap();
    let edges: Vec<FlowEdge> = db::get_all(&pool).await.unwrap();

    let doorbell = flows.iter().find(|f| f.name == "Doorbell").unwrap();
    let chain: Vec<&FlowNode> = nodes.iter().filter(|n| n.flow_id == doorbell.id).collect();
    assert_eq!(chain.len(), 3, "no delay means source, condition, action");
    assert_eq!(
        edges.iter().filter(|e| e.flow_id == doorbell.id).count(),
        2,
        "three nodes in a row are wired by two edges",
    );

    assert!(chain.iter().any(|n| matches!(
        &n.kind,
        FlowNodeKind::Source(TriggerSource::Parameter { fixture_id: f, .. }) if *f == fixture_id
    )));
    assert!(chain
        .iter()
        .any(|n| matches!(n.kind, FlowNodeKind::Condition(TriggerCondition::RisingEdge))));
    assert!(chain.iter().any(|n| matches!(
        &n.kind,
        FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: s }) if *s == sequence_id
    )));

    let porch = flows.iter().find(|f| f.name == "Porch light").unwrap();
    let delayed: Vec<&FlowNode> = nodes.iter().filter(|n| n.flow_id == porch.id).collect();
    assert_eq!(delayed.len(), 4, "a delay is a node of its own");
    assert!(
        delayed.iter().any(|n| matches!(n.kind, FlowNodeKind::Delay { ms: 2500 })),
        "the wait has to survive the redrawing",
    );

    // Laid out rather than piled at the origin, so the graph opens readable.
    assert_eq!(chain.iter().filter(|n| n.x == 0.0).count(), 1);
}

#[tokio::test]
async fn opening_a_migrated_showfile_twice_does_not_double_its_flows() {
    use pult_schema::types::flow::Flow;

    let pool = blank_pool().await;
    write_a_showfile_with_triggers(&pool).await;

    super::migrate_for_test(&pool).await.unwrap();
    super::migrate_for_test(&pool).await.unwrap();

    let flows: Vec<Flow> = db::get_all(&pool).await.unwrap();
    assert_eq!(flows.len(), 2, "the second open finds no triggers table to convert");
}

#[tokio::test]
async fn a_fresh_showfile_has_nothing_to_convert() {
    use pult_schema::types::flow::Flow;

    let pool = blank_pool().await;

    super::migrate_for_test(&pool).await.unwrap();

    let flows: Vec<Flow> = db::get_all(&pool).await.unwrap();
    assert!(flows.is_empty());
}
