//! Engine tests.
//!
//! These pin down the behaviour of `ShowEngine::apply_set` and its path dispatch
//! before that code is rewritten to be registry-driven. Each test drives a real
//! engine task through `EngineHandle` against an in-memory showfile.

use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    types::{cue::{Cue, FollowMode}, fixture::{Fixture, FixtureAddress}, sequence::Sequence, show::Show},
};
use uuid::Uuid;

use super::*;
use crate::infra::showfile;

// ── Fixtures ──────────────────────────────────────────────────────────────────

struct Harness {
    engine: EngineHandle,
    broadcast: UpdateBroadcast,
    pool: Arc<sqlx::SqlitePool>,
}

async fn harness() -> Harness {
    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let (engine, handle, broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool.clone(), None);
    tokio::spawn(engine.run());
    Harness { engine: handle, broadcast, pool }
}

impl Harness {
    /// Restart the engine against the same showfile and reload from disk.
    /// Anything not PERSISTED is gone afterwards.
    async fn reload(&mut self) {
        let _ = self.engine.0.send(EngineCommand::Stop).await;
        let (engine, handle, broadcast) =
            ShowEngine::new(NodeId(Uuid::new_v4()), self.pool.clone(), None);
        tokio::spawn(engine.run());
        self.engine = handle;
        self.broadcast = broadcast;
        let _ = self.engine.0.send(EngineCommand::LoadFromShowfile).await;
        // LoadFromShowfile has no reply channel; a round-trip Get orders behind it.
        let _ = self.engine.get(key("show")).await;
    }
}

fn key(k: &str) -> Path {
    vec![PathSegment::Key(k.into())]
}

fn entity_path(collection: &str, id: Uuid) -> Path {
    vec![PathSegment::Key(collection.into()), PathSegment::Id(id)]
}

fn field_path(collection: &str, id: Uuid, field: &str) -> Path {
    vec![
        PathSegment::Key(collection.into()),
        PathSegment::Id(id),
        PathSegment::Key(field.into()),
    ]
}

fn field_path_on_singleton(table: &str, field: &str) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Key(field.into())]
}

fn create_path(collection: &str) -> Path {
    vec![PathSegment::Key(collection.into()), PathSegment::Key("__create".into())]
}

fn delete_path(collection: &str, id: Uuid) -> Path {
    vec![
        PathSegment::Key(collection.into()),
        PathSegment::Id(id),
        PathSegment::Key("__delete".into()),
    ]
}

fn a_show() -> Show {
    Show {
        id: Uuid::new_v4(),
        name: "Hamlet".into(),
        created_at: Utc::now(),
        is_running: false,
        active_sequence: None,
        editing_cue: None,
    }
}

fn a_sequence(name: &str, cue_ids: Vec<Uuid>) -> Sequence {
    Sequence { id: Uuid::new_v4(), name: name.into(), cue_ids, active_cue_index: None }
}

fn a_cue(name: &str, number: f64) -> Cue {
    Cue {
        id: Uuid::new_v4(),
        name: name.into(),
        number,
        captures: vec![],
        follow_mode: FollowMode::Manual,
        fade_in_ms: 3000,
        fade_out_ms: 3000,
        is_active: false,
    }
}

fn a_fixture(name: &str, address: u16) -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: name.into(),
        fixture_type_id: Uuid::new_v4(),
        address: FixtureAddress::Dmx { universe: 1, address },
        position: None,
        live_values: Default::default(),
        active_preset: None,
    }
}

fn json(v: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(v).unwrap()
}

// ── Show (singleton) ──────────────────────────────────────────────────────────

#[tokio::test]
async fn set_show_then_get_it_back() {
    let h = harness().await;
    let show = a_show();

    h.engine.set(key("show"), Lifecycle::Persisted, json(&show)).await.unwrap();

    let got = h.engine.get(key("show")).await.unwrap();
    assert_eq!(got["name"], "Hamlet");
    assert_eq!(got["id"], show.id.to_string());
}

#[tokio::test]
async fn get_show_before_it_is_set_is_a_path_error() {
    let h = harness().await;
    let err = h.engine.get(key("show")).await.unwrap_err();
    assert!(matches!(err, BackendError::PathNotFound(_)));
}

// ── Create, read, delete ──────────────────────────────────────────────────────

#[tokio::test]
async fn created_sequence_is_readable_by_id_and_in_the_collection() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);

    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let one = h.engine.get(entity_path("sequences", seq.id)).await.unwrap();
    assert_eq!(one["name"], "Act 1");

    let all = h.engine.get(key("sequences")).await.unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_a_sequence_removes_it_from_state_and_from_the_showfile() {
    let mut h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    h.engine
        .set(delete_path("sequences", seq.id), Lifecycle::Persisted, serde_json::Value::Null)
        .await
        .unwrap();

    let all = h.engine.get(key("sequences")).await.unwrap();
    assert!(all.as_array().unwrap().is_empty());

    h.reload().await;
    let all = h.engine.get(key("sequences")).await.unwrap();
    assert!(all.as_array().unwrap().is_empty(), "delete must not come back after a reload");
}

#[tokio::test]
async fn cues_and_fixtures_support_the_same_create_read_delete_cycle() {
    let h = harness().await;
    let cue = a_cue("Blackout", 1.0);
    let fixture = a_fixture("Spot L", 12);

    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    assert_eq!(h.engine.get(entity_path("cues", cue.id)).await.unwrap()["name"], "Blackout");
    assert_eq!(
        h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap()["address"]["Dmx"]
            ["address"],
        12
    );

    h.engine
        .set(delete_path("cues", cue.id), Lifecycle::Persisted, serde_json::Value::Null)
        .await
        .unwrap();
    assert!(h.engine.get(key("cues")).await.unwrap().as_array().unwrap().is_empty());
}

// ── Field patches ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn patching_one_field_leaves_the_others_alone() {
    let h = harness().await;
    let cue_id = Uuid::new_v4();
    let seq = a_sequence("Act 1", vec![cue_id]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    h.engine
        .set(field_path("sequences", seq.id, "name"), Lifecycle::Persisted, json(&"Act 2"))
        .await
        .unwrap();

    let got = h.engine.get(entity_path("sequences", seq.id)).await.unwrap();
    assert_eq!(got["name"], "Act 2");
    assert_eq!(got["cue_ids"][0], cue_id.to_string());
}

#[tokio::test]
async fn patching_a_field_on_a_missing_entity_is_a_path_error() {
    let h = harness().await;
    let err = h
        .engine
        .set(field_path("sequences", Uuid::new_v4(), "name"), Lifecycle::Persisted, json(&"X"))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::PathNotFound(_)));
}

#[tokio::test]
async fn an_unroutable_path_is_rejected_rather_than_silently_ignored() {
    let h = harness().await;
    let err = h
        .engine
        .set(key("nonsense"), Lifecycle::Persisted, json(&"X"))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::PathNotFound(_)));
}

// ── Lifecycle routing ─────────────────────────────────────────────────────────

#[tokio::test]
async fn persisted_fields_survive_a_reload_and_synced_fields_do_not() {
    let mut h = harness().await;
    let seq = a_sequence("Act 1", vec![Uuid::new_v4(), Uuid::new_v4()]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // name is PERSISTED, active_cue_index is SYNCED.
    h.engine
        .set(field_path("sequences", seq.id, "name"), Lifecycle::Persisted, json(&"Act 2"))
        .await
        .unwrap();
    h.engine
        .set(field_path("sequences", seq.id, "active_cue_index"), Lifecycle::Synced, json(&1usize))
        .await
        .unwrap();

    let before = h.engine.get(entity_path("sequences", seq.id)).await.unwrap();
    assert_eq!(before["name"], "Act 2");
    assert_eq!(before["active_cue_index"], 1);

    h.reload().await;

    let after = h.engine.get(entity_path("sequences", seq.id)).await.unwrap();
    assert_eq!(after["name"], "Act 2", "PERSISTED field must survive");
    assert!(after["active_cue_index"].is_null(), "SYNCED field must not be written to the showfile");
}

#[tokio::test]
async fn session_state_is_local_and_never_reaches_the_showfile() {
    let mut h = harness().await;
    let session = serde_json::json!({
        "is_advertising": true,
        "is_follower": false,
        "session_id": Uuid::new_v4(),
        "discovered": [],
    });

    h.engine.set(key("session"), Lifecycle::Local, session).await.unwrap();
    assert_eq!(h.engine.get(key("session")).await.unwrap()["is_advertising"], true);

    h.reload().await;
    assert_eq!(h.engine.get(key("session")).await.unwrap()["is_advertising"], false);
}

// ── Command dispatch ──────────────────────────────────────────────────────────

#[tokio::test]
async fn go_next_walks_the_cue_list_and_falls_off_the_end() {
    let h = harness().await;
    let cues = vec![Uuid::new_v4(), Uuid::new_v4()];
    let seq = a_sequence("Act 1", cues.clone());
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let go_next = field_path("sequences", seq.id, "goNext");
    let active = |h: &Harness| {
        let engine = h.engine.clone();
        let path = entity_path("sequences", seq.id);
        async move { engine.get(path).await.unwrap()["active_cue_index"].clone() }
    };

    h.engine.set(go_next.clone(), Lifecycle::Synced, serde_json::json!({})).await.unwrap();
    assert_eq!(active(&h).await, 0);

    h.engine.set(go_next.clone(), Lifecycle::Synced, serde_json::json!({})).await.unwrap();
    assert_eq!(active(&h).await, 1);

    h.engine.set(go_next.clone(), Lifecycle::Synced, serde_json::json!({})).await.unwrap();
    assert!(active(&h).await.is_null(), "past the last cue the sequence goes idle");
}

#[tokio::test]
async fn go_to_cue_jumps_to_the_named_cue() {
    let h = harness().await;
    let cues = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let seq = a_sequence("Act 1", cues.clone());
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    h.engine
        .set(
            field_path("sequences", seq.id, "goToCue"),
            Lifecycle::Synced,
            serde_json::json!({ "cueId": cues[2] }),
        )
        .await
        .unwrap();

    let got = h.engine.get(entity_path("sequences", seq.id)).await.unwrap();
    assert_eq!(got["active_cue_index"], 2);
}

#[tokio::test]
async fn a_command_that_fails_reports_the_reason() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // goToCue expects { cueId: <uuid> }; an empty object cannot deserialize.
    let err = h
        .engine
        .set(field_path("sequences", seq.id, "goToCue"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::InvalidValue { .. }));
}

#[tokio::test]
async fn legacy_call_dispatch_reaches_the_same_commands() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![Uuid::new_v4()]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    h.engine
        .call("sequences.goNext".into(), serde_json::json!({ "sequenceId": seq.id }))
        .await
        .unwrap();

    let got = h.engine.get(entity_path("sequences", seq.id)).await.unwrap();
    assert_eq!(got["active_cue_index"], 0);
}

// ── Ordering ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sequences_keep_creation_order_and_are_reachable_by_index() {
    let h = harness().await;
    let names = ["Act 1", "Act 2", "Act 3"];
    for name in names {
        h.engine
            .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence(name, vec![])))
            .await
            .unwrap();
    }

    let all = h.engine.get(key("sequences")).await.unwrap();
    let got: Vec<&str> = all.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(got, names);

    let second = h
        .engine
        .get(vec![PathSegment::Key("sequences".into()), PathSegment::Index(1)])
        .await
        .unwrap();
    assert_eq!(second["name"], "Act 2");
}

// ── Broadcasts ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_field_set_broadcasts_that_field() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let mut updates = h
        .engine
        .subscribe_pattern(PathPattern::new(&format!("sequences/{}/name", seq.id)))
        .await;

    h.engine
        .set(field_path("sequences", seq.id, "name"), Lifecycle::Persisted, json(&"Act 2"))
        .await
        .unwrap();

    let value = updates.next().await.expect("expected a broadcast");
    assert_eq!(value, "Act 2");
}

#[tokio::test]
async fn create_broadcasts_the_whole_collection_so_subscribers_need_no_refetch() {
    let h = harness().await;
    let mut updates = h.engine.subscribe_pattern(PathPattern::new("sequences")).await;

    h.engine
        .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence("Act 1", vec![])))
        .await
        .unwrap();

    let value = updates.next().await.expect("expected a broadcast");
    assert_eq!(value.as_array().expect("collection, not a single entity").len(), 1);
}

#[tokio::test]
async fn delete_broadcasts_the_whole_collection() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let mut updates = h.engine.subscribe_pattern(PathPattern::new("sequences")).await;
    h.engine
        .set(delete_path("sequences", seq.id), Lifecycle::Persisted, serde_json::Value::Null)
        .await
        .unwrap();

    let value = updates.next().await.expect("expected a broadcast");
    assert!(value.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_singleton_field_set_broadcasts_the_whole_singleton() {
    // A panel watching `show` asked for the show. A pattern is matched against the
    // path a write names, so broadcasting `show/editing_cue` would reach nobody who
    // did — which is how the edit banner came to never appear.
    let h = harness().await;
    let show = a_show();
    h.engine.set(key("show"), Lifecycle::Persisted, json(&show)).await.unwrap();

    let mut updates = h.broadcast.subscribe_filtered(PathPattern::new("show"));

    let cue_id = Uuid::new_v4();
    h.engine
        .set(field_path_on_singleton("show", "editing_cue"), Lifecycle::Synced, json(&cue_id))
        .await
        .unwrap();

    let value = updates.next().await.expect("an update");
    assert_eq!(value["editing_cue"], json(&cue_id));
    assert_eq!(value["name"], "Hamlet", "and the rest of the show comes with it");
}

#[tokio::test]
async fn an_entity_field_set_still_broadcasts_only_that_field() {
    // The other half of the rule: this one runs at forty a second per fixture in a
    // fade, and sending the whole rig each time is what `subscribeDeep` exists to
    // avoid.
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    let mut updates = h.broadcast.subscribe_filtered(PathPattern::new("fixtures/*/name"));
    h.engine
        .set(field_path("fixtures", fixture.id, "name"), Lifecycle::Persisted, json(&"Spot R"))
        .await
        .unwrap();

    let value = updates.next().await.expect("an update");
    assert_eq!(value, "Spot R", "the field itself, not the fixture it is on");
}

#[tokio::test]
async fn a_failed_set_broadcasts_nothing() {
    let h = harness().await;
    let mut updates = h.engine.subscribe_pattern(PathPattern::new("**")).await;

    let _ = h
        .engine
        .set(field_path("sequences", Uuid::new_v4(), "name"), Lifecycle::Persisted, json(&"X"))
        .await;

    // Follow with a set that does succeed. If the failed one had broadcast,
    // it would arrive first.
    h.engine
        .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence("Act 1", vec![])))
        .await
        .unwrap();

    let value = updates.next().await.expect("expected a broadcast");
    assert!(value.is_array(), "first broadcast should be the successful create");
}

// ── Snapshots ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_snapshot_carries_state_to_a_second_engine() {
    let leader = harness().await;
    leader.engine.set(key("show"), Lifecycle::Persisted, json(&a_show())).await.unwrap();
    let seq = a_sequence("Act 1", vec![]);
    leader.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();
    leader.engine.set(create_path("cues"), Lifecycle::Persisted, json(&a_cue("Blackout", 1.0))).await.unwrap();

    let snapshot = leader.engine.get_snapshot().await;

    let follower = harness().await;
    follower.engine.apply_state_snapshot(snapshot).await;

    assert_eq!(follower.engine.get(key("show")).await.unwrap()["name"], "Hamlet");
    assert_eq!(follower.engine.get(key("sequences")).await.unwrap().as_array().unwrap().len(), 1);
    assert_eq!(follower.engine.get(key("cues")).await.unwrap().as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn applying_a_snapshot_writes_it_to_the_local_showfile() {
    let leader = harness().await;
    leader.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence("Act 1", vec![]))).await.unwrap();
    let snapshot = leader.engine.get_snapshot().await;

    let mut follower = harness().await;
    follower.engine.apply_state_snapshot(snapshot).await;
    // apply_state_snapshot has no reply channel; a round-trip Get orders behind it.
    let _ = follower.engine.get(key("sequences")).await;

    follower.reload().await;
    assert_eq!(follower.engine.get(key("sequences")).await.unwrap().as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn applying_a_snapshot_notifies_frontend_subscribers() {
    let leader = harness().await;
    leader.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence("Act 1", vec![]))).await.unwrap();
    let snapshot = leader.engine.get_snapshot().await;

    let follower = harness().await;
    let mut updates = follower.engine.subscribe_pattern(PathPattern::new("sequences")).await;
    follower.engine.apply_state_snapshot(snapshot).await;

    let value = updates.next().await.expect("expected a broadcast");
    assert_eq!(value.as_array().unwrap().len(), 1);
}

// ── Peer operations ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_peer_operation_is_applied_and_broadcast_locally() {
    use pult_schema::events::operation::{Operation, VectorClock};

    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let mut updates = h
        .engine
        .subscribe_pattern(PathPattern::new(&format!("sequences/{}/name", seq.id)))
        .await;

    let op = Operation {
        id: Uuid::new_v4(),
        node_id: NodeId(Uuid::new_v4()),
        seq: 1,
        clock: VectorClock::default(),
        path: field_path("sequences", seq.id, "name"),
        value: json(&"Renamed by peer"),
        lifecycle: Lifecycle::Persisted,
        timestamp: Utc::now(),
    };
    h.engine.0.send(EngineCommand::ApplyPeerOperation(op)).await.unwrap();

    assert_eq!(updates.next().await.unwrap(), "Renamed by peer");
    assert_eq!(
        h.engine.get(entity_path("sequences", seq.id)).await.unwrap()["name"],
        "Renamed by peer"
    );
}

// ── Registry-driven dispatch ──────────────────────────────────────────────────
//
// The engine names no entity type. These tests hold it to that.

/// FixtureType has a schema and a table but was never mentioned in the engine, so
/// before the dispatch became registry-driven it was unreachable. Nothing about this
/// entity is special: it is here because it is the one the old engine forgot.
#[tokio::test]
async fn fixture_types_are_reachable_without_the_engine_naming_them() {
    use pult_schema::types::fixture::{
        FixtureType, ParameterBinding, ParameterDefinition, ParameterDirection, ParameterKind,
        ParameterValue,
    };

    let mut h = harness().await;
    let ft = FixtureType {
        id: Uuid::new_v4(),
        name: "Source Four".into(),
        manufacturer: "ETC".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition {
            kind: ParameterKind::Intensity,
            direction: ParameterDirection::Output,
            binding: ParameterBinding::Dmx { channel: 1 },
            default_value: ParameterValue::Float(0.0),
        }],
    };

    h.engine.set(create_path("fixture_types"), Lifecycle::Persisted, json(&ft)).await.unwrap();

    let got = h.engine.get(entity_path("fixture_types", ft.id)).await.unwrap();
    assert_eq!(got["manufacturer"], "ETC");

    h.reload().await;
    let after = h.engine.get(entity_path("fixture_types", ft.id)).await.unwrap();
    assert_eq!(after["name"], "Source Four", "must round-trip through the showfile too");

    h.engine
        .set(delete_path("fixture_types", ft.id), Lifecycle::Persisted, serde_json::Value::Null)
        .await
        .unwrap();
    assert!(h.engine.get(key("fixture_types")).await.unwrap().as_array().unwrap().is_empty());
}

#[tokio::test]
async fn every_registered_collection_is_readable() {
    use pult_schema::registry::EntityMeta;

    let h = harness().await;
    for meta in EntityMeta::all_with_tables() {
        let table = meta.table_name.unwrap();
        if meta.is_singleton {
            continue;
        }
        let got = h.engine.get(key(table)).await;
        assert!(
            got.is_ok_and(|v| v.is_array()),
            "collection {table} is registered but not readable from the engine",
        );
    }
}

#[tokio::test]
async fn an_entity_can_be_written_through_its_index() {
    let h = harness().await;
    for name in ["Act 1", "Act 2"] {
        h.engine
            .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence(name, vec![])))
            .await
            .unwrap();
    }

    h.engine
        .set(
            vec![
                PathSegment::Key("sequences".into()),
                PathSegment::Index(1),
                PathSegment::Key("name".into()),
            ],
            Lifecycle::Persisted,
            json(&"Renamed"),
        )
        .await
        .unwrap();

    let all = h.engine.get(key("sequences")).await.unwrap();
    assert_eq!(all[0]["name"], "Act 1");
    assert_eq!(all[1]["name"], "Renamed");
}

#[tokio::test]
async fn a_singleton_field_can_be_patched_on_its_own() {
    let mut h = harness().await;
    let show = a_show();
    h.engine.set(key("show"), Lifecycle::Persisted, json(&show)).await.unwrap();

    // is_running is SYNCED, name is PERSISTED.
    h.engine
        .set(vec![PathSegment::Key("show".into()), PathSegment::Key("is_running".into())],
             Lifecycle::Synced, json(&true))
        .await
        .unwrap();
    h.engine
        .set(vec![PathSegment::Key("show".into()), PathSegment::Key("name".into())],
             Lifecycle::Persisted, json(&"Macbeth"))
        .await
        .unwrap();

    let got = h.engine.get(key("show")).await.unwrap();
    assert_eq!(got["is_running"], true);
    assert_eq!(got["name"], "Macbeth");

    h.reload().await;
    let after = h.engine.get(key("show")).await.unwrap();
    assert_eq!(after["name"], "Macbeth");
    assert_eq!(after["is_running"], false, "SYNCED field must not be persisted");
}

#[tokio::test]
async fn a_single_field_can_be_read_without_fetching_the_whole_entity() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let name = h.engine.get(field_path("sequences", seq.id, "name")).await.unwrap();
    assert_eq!(name, "Act 1");
}

#[tokio::test]
async fn writing_a_field_the_schema_does_not_have_is_rejected() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let err = h
        .engine
        .set(field_path("sequences", seq.id, "colour"), Lifecycle::Persisted, json(&"red"))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::PathNotFound(_)));
}

#[tokio::test]
async fn a_value_that_is_not_a_valid_entity_is_rejected() {
    let h = harness().await;
    let err = h
        .engine
        .set(create_path("sequences"), Lifecycle::Persisted, serde_json::json!({ "id": "not-a-uuid" }))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::InvalidValue { .. }));
    assert!(h.engine.get(key("sequences")).await.unwrap().as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_whole_entity_can_be_replaced_in_place() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let mut replacement = seq.clone();
    replacement.name = "Act 1 (revised)".into();
    h.engine
        .set(entity_path("sequences", seq.id), Lifecycle::Persisted, json(&replacement))
        .await
        .unwrap();

    let all = h.engine.get(key("sequences")).await.unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1, "replace must not add a second entity");
    assert_eq!(all[0]["name"], "Act 1 (revised)");
}

#[tokio::test]
async fn a_leader_snapshot_leaves_this_node_s_session_alone() {
    let leader = harness().await;
    leader
        .engine
        .set(
            key("session"),
            Lifecycle::Local,
            serde_json::json!({
                "is_advertising": true, "is_follower": false,
                "session_id": Uuid::new_v4(), "discovered": [],
            }),
        )
        .await
        .unwrap();
    let snapshot = leader.engine.get_snapshot().await;

    let follower = harness().await;
    follower
        .engine
        .set(
            key("session"),
            Lifecycle::Local,
            serde_json::json!({
                "is_advertising": false, "is_follower": true,
                "session_id": null, "discovered": [],
            }),
        )
        .await
        .unwrap();

    follower.engine.apply_state_snapshot(snapshot).await;

    let session = follower.engine.get(key("session")).await.unwrap();
    assert_eq!(session["is_follower"], true, "LOCAL session must survive a leader snapshot");
    assert_eq!(session["is_advertising"], false);
}

#[tokio::test]
async fn a_leader_snapshot_leaves_this_node_s_device_list_alone() {
    // Same rule as the session, and for the same reason: what a leader can see on
    // its network segment says nothing about what is plugged in here.
    let leader = harness().await;
    leader
        .engine
        .set(
            key("devices"),
            Lifecycle::Local,
            serde_json::json!({ "discovered": {}, "broker_addr": "10.0.0.1:1883", "active": true }),
        )
        .await
        .unwrap();
    let snapshot = leader.engine.get_snapshot().await;

    let follower = harness().await;
    follower
        .engine
        .set(
            key("devices"),
            Lifecycle::Local,
            serde_json::json!({ "discovered": {}, "broker_addr": null, "active": false }),
        )
        .await
        .unwrap();

    follower.engine.apply_state_snapshot(snapshot).await;

    let devices = follower.engine.get(key("devices")).await.unwrap();
    assert_eq!(devices["active"], false, "LOCAL devices must survive a leader snapshot");
    assert_eq!(devices["broker_addr"], serde_json::Value::Null);
}

#[tokio::test]
async fn every_local_path_answers_before_anything_has_written_to_it() {
    // A frontend subscribes on connect, before any manager has run. An empty
    // state is an answer; a path error is not.
    let h = harness().await;
    for path in ["session", "devices"] {
        let value = h.engine.get(key(path)).await.unwrap();
        assert!(value.is_object(), "{path} must exist from the start, got {value}");
    }
}

// ── Live values from a device ─────────────────────────────────────────────────

#[tokio::test]
async fn setting_one_live_value_leaves_the_others_where_they_were() {
    // Two ports on one node reporting in the same millisecond would each write back
    // a map missing the other's key, if the merge happened outside the actor.
    let h = harness().await;
    let fixture = a_fixture("Sensor", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    h.engine
        .set_live_value(fixture.id, "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": true }))
        .await
        .unwrap();
    h.engine
        .set_live_value(fixture.id, "Temperature".into(), serde_json::json!({ "type": "Float", "value": 21.5 }))
        .await
        .unwrap();

    let values = h.engine.get(field_path("fixtures", fixture.id, "live_values")).await.unwrap();
    assert_eq!(values["Contact:0"]["value"], true);
    assert_eq!(values["Temperature"]["value"], 21.5);
}

#[tokio::test]
async fn a_later_reading_replaces_the_earlier_one_on_the_same_key() {
    let h = harness().await;
    let fixture = a_fixture("Sensor", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    for value in [true, false] {
        h.engine
            .set_live_value(
                fixture.id,
                "Contact:0".into(),
                serde_json::json!({ "type": "Bool", "value": value }),
            )
            .await
            .unwrap();
    }

    let values = h.engine.get(field_path("fixtures", fixture.id, "live_values")).await.unwrap();
    assert_eq!(values["Contact:0"]["value"], false);
}

#[tokio::test]
async fn a_live_value_for_a_fixture_that_is_not_patched_is_refused() {
    let h = harness().await;
    let result = h
        .engine
        .set_live_value(Uuid::new_v4(), "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": true }))
        .await;
    assert!(result.is_err(), "an input from a device nothing is patched to has nowhere to go");
}

#[tokio::test]
async fn a_live_value_reaches_the_frontends() {
    let h = harness().await;
    let fixture = a_fixture("Sensor", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    let mut updates = h.broadcast.subscribe_filtered(PathPattern::new("fixtures/**"));

    h.engine
        .set_live_value(fixture.id, "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": true }))
        .await
        .unwrap();

    let update = tokio::time::timeout(std::time::Duration::from_secs(1), updates.next())
        .await
        .expect("a live value has to be broadcast")
        .expect("the stream stays open");
    assert_eq!(update["Contact:0"]["value"], true);
}

// ── Flows ─────────────────────────────────────────────────────────────────────

mod flows {
    use pult_schema::types::{
        fixture::ParameterKind,
        flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
    };

    use super::*;

    fn a_node(flow_id: Uuid, kind: FlowNodeKind) -> FlowNode {
        FlowNode {
            id: Uuid::new_v4(),
            flow_id,
            kind,
            x: 0.0,
            y: 0.0,
            active: false,
            last_fired_at: None,
        }
    }

    fn an_edge(flow_id: Uuid, from: Uuid, to: Uuid) -> FlowEdge {
        FlowEdge {
            id: Uuid::new_v4(),
            flow_id,
            from_node: from,
            from_port: 0,
            to_node: to,
            to_port: 0,
        }
    }

    /// A doorbell wired to an action: source → rising edge → whatever it does.
    async fn a_flow(h: &Harness, fixture_id: Uuid, action: TriggerAction) -> (Flow, FlowNode) {
        let flow = Flow { id: Uuid::new_v4(), name: "Doorbell".into(), enabled: true };
        h.engine.set(create_path("flows"), Lifecycle::Persisted, json(&flow)).await.unwrap();

        let source = a_node(
            flow.id,
            FlowNodeKind::Source(TriggerSource::Parameter {
                fixture_id,
                parameter: ParameterKind::Contact(0),
            }),
        );
        let gate = a_node(flow.id, FlowNodeKind::Condition(TriggerCondition::RisingEdge));
        let act = a_node(flow.id, FlowNodeKind::Action(action));
        for node in [&source, &gate, &act] {
            h.engine.set(create_path("flow_nodes"), Lifecycle::Persisted, json(node)).await.unwrap();
        }
        for edge in [an_edge(flow.id, source.id, gate.id), an_edge(flow.id, gate.id, act.id)] {
            h.engine
                .set(create_path("flow_edges"), Lifecycle::Persisted, json(&edge))
                .await
                .unwrap();
        }

        (flow, act)
    }

    /// A sensor fixture, a sequence with two cues, and a flow between them.
    async fn a_show(h: &Harness) -> (Uuid, Uuid, Flow, FlowNode) {
        let fixture = a_fixture("Doorbell", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

        let cues = vec![Uuid::new_v4(), Uuid::new_v4()];
        let sequence = a_sequence("Act 1", cues);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&sequence)).await.unwrap();

        let (flow, act) =
            a_flow(h, fixture.id, TriggerAction::GoNext { sequence_id: sequence.id }).await;

        (fixture.id, sequence.id, flow, act)
    }

    async fn close_the_contact(h: &Harness, fixture_id: Uuid) {
        h.engine
            .set_live_value(
                fixture_id,
                "Contact:0".into(),
                serde_json::json!({ "type": "Bool", "value": true }),
            )
            .await
            .unwrap();
    }

    async fn active_cue(h: &Harness, sequence_id: Uuid) -> serde_json::Value {
        h.engine.get(entity_path("sequences", sequence_id)).await.unwrap()["active_cue_index"]
            .clone()
    }

    /// Wait for the engine's own tick to have run the flow.
    async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
        for _ in 0..100 {
            if check().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn a_contact_closing_advances_the_sequence_it_is_wired_to() {
        let h = harness().await;
        let (fixture_id, sequence_id, _, _) = a_show(&h).await;

        close_the_contact(&h, fixture_id).await;

        eventually("the cue to advance", async || {
            active_cue(&h, sequence_id).await == serde_json::json!(0)
        })
        .await;
    }

    #[tokio::test]
    async fn an_action_node_records_when_it_last_fired() {
        let h = harness().await;
        let (fixture_id, _, _, act) = a_show(&h).await;

        close_the_contact(&h, fixture_id).await;

        eventually("the action to record its firing", async || {
            !h.engine.get(entity_path("flow_nodes", act.id)).await.unwrap()["last_fired_at"]
                .is_null()
        })
        .await;
    }

    #[tokio::test]
    async fn a_source_lights_up_when_its_contact_closes() {
        // The graph is meant to be watchable, not just drawable: `active` is what
        // says a signal went through, and it replicates to every console watching.
        let h = harness().await;
        let fixture = a_fixture("Doorbell", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        let sequence = a_sequence("Act 1", vec![Uuid::new_v4()]);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&sequence)).await.unwrap();
        a_flow(&h, fixture.id, TriggerAction::GoNext { sequence_id: sequence.id }).await;

        close_the_contact(&h, fixture.id).await;

        eventually("a node to light up", async || {
            h.engine
                .get(key("flow_nodes"))
                .await
                .unwrap()
                .as_array()
                .is_some_and(|nodes| nodes.iter().any(|n| n["active"] == serde_json::json!(true)))
        })
        .await;
    }

    #[tokio::test]
    async fn a_disabled_flow_leaves_the_show_alone() {
        let h = harness().await;
        let (fixture_id, sequence_id, flow, _) = a_show(&h).await;
        h.engine
            .set(
                field_path("flows", flow.id, "enabled"),
                Lifecycle::Persisted,
                serde_json::Value::Bool(false),
            )
            .await
            .unwrap();

        close_the_contact(&h, fixture_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        assert_eq!(active_cue(&h, sequence_id).await, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn a_follower_never_fires_a_flow() {
        // The action is a write to replicated state, and the leader is about to send
        // it. A follower firing too would apply the same change twice.
        let h = harness().await;
        let (fixture_id, sequence_id, _, _) = a_show(&h).await;
        h.engine
            .set(
                key("session"),
                Lifecycle::Local,
                serde_json::json!({
                    "is_advertising": false, "is_follower": true,
                    "session_id": Uuid::new_v4(), "discovered": [],
                }),
            )
            .await
            .unwrap();

        close_the_contact(&h, fixture_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        assert_eq!(active_cue(&h, sequence_id).await, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn a_flow_can_drive_a_parameter_instead_of_a_cue() {
        let h = harness().await;
        let sensor = a_fixture("Doorbell", 1);
        let lamp = a_fixture("Porch light", 2);
        for fixture in [&sensor, &lamp] {
            h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(fixture)).await.unwrap();
        }
        a_flow(
            &h,
            sensor.id,
            TriggerAction::SetParameter {
                fixture_id: lamp.id,
                parameter: ParameterKind::Switch(0),
                value: pult_schema::types::fixture::ParameterValue::Bool(true),
            },
        )
        .await;

        close_the_contact(&h, sensor.id).await;

        eventually("the lamp to come on", async || {
            h.engine.get(field_path("fixtures", lamp.id, "live_values")).await.unwrap()["Switch:0"]
                ["value"]
                == serde_json::json!(true)
        })
        .await;
    }

    #[tokio::test]
    async fn pressing_a_button_node_fires_what_it_is_wired_to() {
        // A press is `last_fired_at` changing, which is why any console can press a
        // button and only the leader acts on it.
        let h = harness().await;
        let sequence = a_sequence("Act 1", vec![Uuid::new_v4(), Uuid::new_v4()]);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&sequence)).await.unwrap();

        let flow = Flow { id: Uuid::new_v4(), name: "Panic".into(), enabled: true };
        h.engine.set(create_path("flows"), Lifecycle::Persisted, json(&flow)).await.unwrap();
        let button = a_node(flow.id, FlowNodeKind::Button);
        let act = a_node(
            flow.id,
            FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: sequence.id }),
        );
        for node in [&button, &act] {
            h.engine.set(create_path("flow_nodes"), Lifecycle::Persisted, json(node)).await.unwrap();
        }
        h.engine
            .set(create_path("flow_edges"), Lifecycle::Persisted, json(&an_edge(flow.id, button.id, act.id)))
            .await
            .unwrap();

        // Let the first tick record the button before pressing it.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        h.engine
            .set(field_path("flow_nodes", button.id, "press"), Lifecycle::Synced, serde_json::json!({}))
            .await
            .unwrap();

        eventually("the cue to advance", async || {
            active_cue(&h, sequence.id).await == serde_json::json!(0)
        })
        .await;
    }

    #[tokio::test]
    async fn a_flow_survives_the_showfile() {
        let mut h = harness().await;
        let (_, _, flow, act) = a_show(&h).await;

        h.reload().await;

        let reloaded = h.engine.get(entity_path("flows", flow.id)).await.unwrap();
        assert_eq!(reloaded["name"], "Doorbell");
        assert_eq!(reloaded["enabled"], true);

        let node = h.engine.get(entity_path("flow_nodes", act.id)).await.unwrap();
        assert_eq!(node["flow_id"], json(&flow.id));
        assert_eq!(
            node["active"], false,
            "active is SYNCED, so it comes back as its default rather than from disk",
        );

        let edges = h.engine.get(key("flow_edges")).await.unwrap();
        assert_eq!(edges.as_array().map(|e| e.len()), Some(2), "the wiring comes back too");
    }
}

// ── Rust accessor API ─────────────────────────────────────────────────────────
//
// The path-proxy API from CLAUDE.md, driven against a real engine. Accessor path
// keys used to be camelCased while the wire uses serde's snake_case names, so every
// field set through this API wrote a key the entity did not have, serde dropped it,
// and the call reported success having changed nothing.

fn data_root(engine: &EngineHandle) -> pult_schema::handle::ShowDataRoot<crate::handle::EngineDataHandle> {
    pult_schema::handle::ShowDataRoot::new(crate::handle::EngineDataHandle(engine.clone()))
}

#[tokio::test]
async fn a_field_set_through_the_rust_accessor_actually_lands() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let data = data_root(&h.engine);
    data.sequences().by_id(seq.id).name().set("Act 2".into()).await.unwrap();

    assert_eq!(h.engine.get(entity_path("sequences", seq.id)).await.unwrap()["name"], "Act 2");
}

#[tokio::test]
async fn a_multi_word_field_set_through_the_rust_accessor_actually_lands() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![Uuid::new_v4(), Uuid::new_v4()]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let data = data_root(&h.engine);
    data.sequences().by_id(seq.id).active_cue_index().set(Some(1)).await.unwrap();

    assert_eq!(
        h.engine.get(entity_path("sequences", seq.id)).await.unwrap()["active_cue_index"],
        1,
        "a snake_case field must be reachable through its accessor",
    );
}

#[tokio::test]
async fn the_rust_accessor_reads_back_what_it_wrote() {
    let h = harness().await;
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    let data = data_root(&h.engine);
    let accessor = data.sequences().by_id(seq.id);
    accessor.name().set("Act 3".into()).await.unwrap();
    assert_eq!(accessor.name().get().await.unwrap(), "Act 3");
}

#[tokio::test]
async fn the_root_exposes_every_registered_table_including_fixture_types() {
    use pult_schema::types::fixture::FixtureType;

    let h = harness().await;
    let ft = FixtureType {
        id: Uuid::new_v4(),
        name: "Source Four".into(),
        manufacturer: "ETC".into(),
        channel_count: 1,
        parameters: vec![],
    };
    h.engine.set(create_path("fixture_types"), Lifecycle::Persisted, json(&ft)).await.unwrap();

    let data = data_root(&h.engine);
    // The root accessor's path key is the table name, so this reaches fixture_types
    // rather than a "fixtureTypes" table that has never existed.
    let all = data.fixture_types().all().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].manufacturer, "ETC");

    data.fixture_types().by_id(ft.id).name().set("Source Four 50deg".into()).await.unwrap();
    assert_eq!(
        h.engine.get(entity_path("fixture_types", ft.id)).await.unwrap()["name"],
        "Source Four 50deg",
    );
}

#[tokio::test]
async fn the_root_reaches_the_show_singleton() {
    let h = harness().await;
    h.engine.set(key("show"), Lifecycle::Persisted, json(&a_show())).await.unwrap();

    let data = data_root(&h.engine);
    data.show().name().set("Macbeth".into()).await.unwrap();

    assert_eq!(h.engine.get(key("show")).await.unwrap()["name"], "Macbeth");
}

#[tokio::test]
async fn accessor_path_keys_are_the_serde_field_names() {
    use pult_schema::registry::EntityMeta;

    // field_lifecycles() is what path_lifecycle and the engine both key on. If an
    // accessor ever emits a different spelling, writes through it go nowhere.
    for meta in EntityMeta::all_with_tables() {
        for (field, _) in (meta.field_lifecycles)() {
            assert_eq!(
                *field,
                field.to_lowercase().replace(' ', "_"),
                "{}::{field} is not a snake_case serde name",
                meta.entity_name,
            );
        }
    }
}

// ── Playback through the engine ───────────────────────────────────────────────
//
// These run on a paused clock, so tokio advances virtual time to the next timer and
// a four-second fade finishes in microseconds.

use pult_schema::types::{
    cue::ParameterCapture,
    fixture::{ParameterKind, ParameterValue},
};

fn an_intensity_cue(fixture_id: Uuid, level: f32, fade_in_ms: u32) -> Cue {
    let mut cue = a_cue("Up", 1.0);
    cue.fade_in_ms = fade_in_ms;
    cue.captures = vec![ParameterCapture {
        fixture_id,
        parameter_kind: ParameterKind::Intensity,
        value: ParameterValue::Float(level),
        fade_in_ms: 0,
        fade_out_ms: 0,
        delay_in_ms: 0,
    }];
    cue
}

async fn intensity_of(h: &Harness, fixture_id: Uuid) -> f32 {
    let fixture = h.engine.get(entity_path("fixtures", fixture_id)).await.unwrap();
    fixture["live_values"]["Intensity"]["value"].as_f64().unwrap_or(f64::NAN) as f32
}

#[tokio::test]
async fn taking_a_cue_fades_the_fixture_up() {
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    let cue = an_intensity_cue(fixture.id, 1.0, 4000);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // Setup is done, so the clock can go virtual: tokio now jumps to the next
    // timer and a multi-second fade finishes in microseconds.
    tokio::time::pause();

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    let midway = intensity_of(&h, fixture.id).await;
    assert!(midway > 0.1 && midway < 0.9, "expected a partial level midway, got {midway}");

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(intensity_of(&h, fixture.id).await, 1.0);
}

#[tokio::test]
async fn the_cue_being_played_is_marked_active() {
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    let cue = an_intensity_cue(fixture.id, 1.0, 0);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // Setup is done, so the clock can go virtual: tokio now jumps to the next
    // timer and a multi-second fade finishes in microseconds.
    tokio::time::pause();

    assert_eq!(h.engine.get(entity_path("cues", cue.id)).await.unwrap()["is_active"], false);

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(h.engine.get(entity_path("cues", cue.id)).await.unwrap()["is_active"], true);
}

#[tokio::test]
async fn a_follow_cue_advances_the_sequence_on_its_own() {
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    let mut first = an_intensity_cue(fixture.id, 1.0, 0);
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 2000 };
    let second = an_intensity_cue(fixture.id, 0.0, 0);
    let seq = a_sequence("Act 1", vec![first.id, second.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&first)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&second)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // Setup is done, so the clock can go virtual: tokio now jumps to the next
    // timer and a multi-second fade finishes in microseconds.
    tokio::time::pause();

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(h.engine.get(entity_path("sequences", seq.id)).await.unwrap()["active_cue_index"], 0);

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(
        h.engine.get(entity_path("sequences", seq.id)).await.unwrap()["active_cue_index"],
        1,
        "the follow cue should have taken itself",
    );
    assert_eq!(intensity_of(&h, fixture.id).await, 0.0);
}

#[tokio::test]
async fn playback_output_is_broadcast_to_frontends() {
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    let cue = an_intensity_cue(fixture.id, 1.0, 0);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // Setup is done, so the clock can go virtual: tokio now jumps to the next
    // timer and a multi-second fade finishes in microseconds.
    tokio::time::pause();

    let mut updates = h
        .engine
        .subscribe_pattern(PathPattern::new(&format!("fixtures/{}/live_values", fixture.id)))
        .await;

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();

    let value = updates.next().await.expect("expected a live-values broadcast");
    assert_eq!(value["Intensity"]["value"], 1.0);
}

#[tokio::test]
async fn playback_output_is_never_written_to_the_showfile() {
    let mut h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    let cue = an_intensity_cue(fixture.id, 1.0, 0);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();
    // Setup is done, so the clock can go virtual: tokio now jumps to the next
    // timer and a multi-second fade finishes in microseconds.
    tokio::time::pause();

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(intensity_of(&h, fixture.id).await, 1.0);

    tokio::time::resume();
    h.reload().await;
    let after = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert!(
        after["live_values"].as_object().is_none_or(|m| m.is_empty()),
        "live values are output, not show data",
    );
}

#[tokio::test]
async fn a_programmer_entry_takes_a_fixture_over_and_giving_it_back_returns_the_cue() {
    // The whole path, over the wire: a `programmer_values` row is created the way a
    // frontend creates one, and the fixture's output follows it — then stops.
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    let cue = an_intensity_cue(fixture.id, 1.0, 0);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    tokio::time::pause();

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(intensity_of(&h, fixture.id).await, 1.0);

    let entry = serde_json::json!({
        "id": Uuid::new_v4(),
        "fixture_id": fixture.id,
        "parameter_kind": "Intensity",
        "value": { "type": "Float", "value": 0.2 },
        "locked": false,
    });
    h.engine.set(create_path("programmer_values"), Lifecycle::Synced, entry.clone()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        intensity_of(&h, fixture.id).await,
        0.2,
        "the programmer holds the parameter, so the cue does not reach the output",
    );

    let entry_id: Uuid = serde_json::from_value(entry["id"].clone()).unwrap();
    h.engine
        .set(delete_path("programmer_values", entry_id), Lifecycle::Synced, serde_json::Value::Null)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        intensity_of(&h, fixture.id).await,
        1.0,
        "letting go puts the parameter back where the cue had it",
    );
}

#[tokio::test]
async fn programmer_values_never_reach_the_showfile() {
    // SYNCED, like a station: what is in the operator's hands is not part of the
    // show, and a showfile that reopened asserting it over playback would be a fault.
    let mut h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine
        .set(
            create_path("programmer_values"),
            Lifecycle::Synced,
            serde_json::json!({
                "id": Uuid::new_v4(),
                "fixture_id": fixture.id,
                "parameter_kind": "Intensity",
                "value": { "type": "Float", "value": 0.6 },
                "locked": true,
            }),
        )
        .await
        .unwrap();

    assert_eq!(h.engine.get(key("programmer_values")).await.unwrap().as_array().unwrap().len(), 1);

    h.reload().await;

    assert!(h.engine.get(key("programmer_values")).await.unwrap().as_array().unwrap().is_empty());
}

#[tokio::test]
async fn an_idle_show_does_no_playback_work() {
    let h = harness().await;
    // Setup is done, so the clock can go virtual: tokio now jumps to the next
    // timer and a multi-second fade finishes in microseconds.
    tokio::time::pause();

    let mut updates = h.engine.subscribe_pattern(PathPattern::new("**")).await;

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert!(
        futures::poll!(updates.next()).is_pending(),
        "an idle engine must not broadcast anything on its own",
    );
}

// ── Output ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn playback_hands_the_patch_to_the_output_plugins() {
    use crate::infra::connectors::{OutputCommand, OutputHandle};

    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let (mut engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
    let (tx, mut output_rx) = tokio::sync::mpsc::channel(8);
    engine.set_output(OutputHandle(tx));
    tokio::spawn(engine.run());

    let fixture = a_fixture("Spot L", 1);
    let cue = an_intensity_cue(fixture.id, 1.0, 0);
    let seq = a_sequence("Act 1", vec![cue.id]);
    handle.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    handle.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    handle.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    handle
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();

    // Patching the fixture pushes too, so wait for the push that carries a level.
    let found = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while let Some(cmd) = output_rx.recv().await {
            let OutputCommand::Patch { fixtures, changed, .. } = cmd else { continue };
            if changed.contains(&fixture.id) {
                return fixtures;
            }
        }
        panic!("output channel closed");
    })
    .await
    .expect("expected playback output to reach the plugins within two seconds");

    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].live_values.get("Intensity"),
        Some(&ParameterValue::Float(1.0)),
        "output must see the level playback just set",
    );
}

#[tokio::test]
async fn an_idle_show_sends_nothing_to_output() {
    use crate::infra::connectors::{OutputCommand, OutputHandle};

    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let (mut engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
    let (tx, mut output_rx) = tokio::sync::mpsc::channel(8);
    engine.set_output(OutputHandle(tx));
    tokio::spawn(engine.run());

    let fixture = a_fixture("Spot L", 1);
    handle.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    // Drain everything the patch itself caused, including the one-off Configure that
    // tells the output side which outputs the show has.
    let deadline = std::time::Duration::from_millis(200);
    while tokio::time::timeout(deadline, output_rx.recv()).await.is_ok() {}

    // A Configure would be fine here; a Patch would not.
    let quiet = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        loop {
            match output_rx.recv().await {
                Some(OutputCommand::Patch { .. }) => return,
                Some(_) => continue,
                None => return,
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "a show with nothing running must not push output every tick");
}

// ── Collection order ──────────────────────────────────────────────────────────

async fn sequence_names(h: &Harness) -> Vec<String> {
    h.engine
        .get(key("sequences"))
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn collection_order_survives_a_reload() {
    let mut h = harness().await;
    // Names that do not sort the same way as their ids will, so a reload that fell
    // back to UUID order would show up here.
    for name in ["Prologue", "Act 1", "Interval", "Act 2", "Curtain"] {
        h.engine
            .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence(name, vec![])))
            .await
            .unwrap();
    }
    let before = sequence_names(&h).await;

    h.reload().await;

    assert_eq!(sequence_names(&h).await, before, "the operator's order is show data");
    assert_eq!(before[0], "Prologue");
    assert_eq!(before[4], "Curtain");
}

#[tokio::test]
async fn order_closes_up_after_a_delete_and_stays_closed() {
    let mut h = harness().await;
    let mut ids = Vec::new();
    for name in ["One", "Two", "Three"] {
        let seq = a_sequence(name, vec![]);
        ids.push(seq.id);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();
    }

    h.engine
        .set(delete_path("sequences", ids[1]), Lifecycle::Persisted, serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(sequence_names(&h).await, vec!["One", "Three"]);

    h.reload().await;
    assert_eq!(sequence_names(&h).await, vec!["One", "Three"]);
}

#[tokio::test]
async fn a_sequence_added_after_a_reload_goes_on_the_end() {
    let mut h = harness().await;
    h.engine
        .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence("One", vec![])))
        .await
        .unwrap();

    h.reload().await;
    h.engine
        .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence("Two", vec![])))
        .await
        .unwrap();

    assert_eq!(sequence_names(&h).await, vec!["One", "Two"]);
    h.reload().await;
    assert_eq!(sequence_names(&h).await, vec!["One", "Two"]);
}

#[tokio::test]
async fn every_collection_keeps_its_own_order() {
    let mut h = harness().await;
    let cues: Vec<Cue> = ["Blackout", "Warm", "Cold"].iter().map(|n| a_cue(n, 1.0)).collect();
    for cue in &cues {
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(cue)).await.unwrap();
    }
    for name in ["First", "Second"] {
        h.engine
            .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence(name, vec![])))
            .await
            .unwrap();
    }

    h.reload().await;

    let all_cues = h.engine.get(key("cues")).await.unwrap();
    let cue_names: Vec<&str> =
        all_cues.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(cue_names, vec!["Blackout", "Warm", "Cold"]);
    assert_eq!(sequence_names(&h).await, vec!["First", "Second"]);
}

#[tokio::test]
async fn an_order_row_for_a_deleted_entity_does_not_resurrect_it() {
    use crate::infra::showfile::order;

    let mut h = harness().await;
    let seq = a_sequence("Only", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // Leave a stale row behind, as a crash between the two writes would.
    let ghost = Uuid::new_v4();
    order::save(&h.pool, "sequences", &[ghost, seq.id]).await.unwrap();

    h.reload().await;

    assert_eq!(sequence_names(&h).await, vec!["Only"]);
}

#[tokio::test]
async fn an_entity_missing_from_the_order_still_appears() {
    use crate::infra::showfile::order;

    let mut h = harness().await;
    for name in ["One", "Two"] {
        h.engine
            .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence(name, vec![])))
            .await
            .unwrap();
    }

    // Forget the order entirely, as an older showfile would have.
    order::save(&h.pool, "sequences", &[]).await.unwrap();
    h.reload().await;

    assert_eq!(
        sequence_names(&h).await.len(),
        2,
        "an unordered entity is appended, never dropped",
    );
}

#[tokio::test]
async fn a_leader_snapshot_carries_the_leader_s_order() {
    let leader = harness().await;
    for name in ["Prologue", "Act 1", "Curtain"] {
        leader
            .engine
            .set(create_path("sequences"), Lifecycle::Persisted, json(&a_sequence(name, vec![])))
            .await
            .unwrap();
    }
    let snapshot = leader.engine.get_snapshot().await;

    let mut follower = harness().await;
    follower.engine.apply_state_snapshot(snapshot).await;
    let _ = follower.engine.get(key("sequences")).await;

    assert_eq!(sequence_names(&follower).await, vec!["Prologue", "Act 1", "Curtain"]);

    follower.reload().await;
    assert_eq!(
        sequence_names(&follower).await,
        vec!["Prologue", "Act 1", "Curtain"],
        "a follower must write the order it was given, not just hold it in memory",
    );
}

// ── Schema evolution ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_showfile_written_before_a_field_existed_still_opens() {
    use pult_schema::types::fixture::{FixturePosition, Vec3};

    let h = harness().await;
    let mut fixture = a_fixture("Spot L", 1);
    fixture.position = Some(FixturePosition::Point(Vec3 { x: 1.0, y: 2.0, z: 3.0 }));
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    // Take the column away, as an older showfile would have it. SQLite can drop a
    // column, which is exactly the state a show saved before the field existed is in.
    sqlx::query("ALTER TABLE fixtures DROP COLUMN position")
        .execute(h.pool.as_ref())
        .await
        .unwrap();

    // Reopening runs migrations, which must put the column back.
    crate::infra::showfile::migrate_for_test(&h.pool).await.unwrap();

    let mut h = h;
    h.reload().await;
    let after = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert_eq!(after["name"], "Spot L");
    assert!(after["position"].is_null(), "a column added later has no value on old rows");
}

#[tokio::test]
async fn a_fixture_position_round_trips_through_the_showfile() {
    use pult_schema::types::fixture::{FixturePosition, Vec3};

    let mut h = harness().await;
    let mut fixture = a_fixture("Spot L", 1);
    fixture.position = Some(FixturePosition::Axial {
        position: Vec3 { x: 1.5, y: 6.0, z: -2.0 },
        direction: Vec3 { x: 0.0, y: -1.0, z: 0.0 },
    });
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    h.reload().await;

    let after = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert_eq!(after["position"]["Axial"]["position"]["y"], 6.0);
    assert_eq!(after["position"]["Axial"]["direction"]["y"], -1.0);
}

#[tokio::test]
async fn a_fixture_with_no_position_is_still_a_valid_fixture() {
    let mut h = harness().await;
    let fixture = a_fixture("Unplaced", 1);
    assert!(fixture.position.is_none());
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    h.reload().await;

    let after = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert_eq!(after["name"], "Unplaced");
    assert!(after["position"].is_null());
}

// ── A flow watching what a cue drives ─────────────────────────────────────────

mod watching_playback {
    use pult_schema::types::{
        fixture::ParameterKind,
        flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
    };

    use super::*;

    /// A cue's own output is show state like any other, and a *Watch* node offers
    /// every driven parameter — so a fade has to reach the flow tick, or the whole
    /// dropdown would be full of things that can never fire.
    #[tokio::test]
    async fn a_cue_raising_a_level_fires_a_flow_watching_it() {
        let h = harness().await;

        let lamp = a_fixture("Porch light", 1);
        let switched = a_fixture("Siren", 2);
        for fixture in [&lamp, &switched] {
            h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(fixture)).await.unwrap();
        }

        let cue = Cue {
            id: Uuid::new_v4(),
            name: "House".into(),
            number: 1.0,
            captures: vec![pult_schema::types::cue::ParameterCapture {
                fixture_id: lamp.id,
                parameter_kind: ParameterKind::Intensity,
                value: pult_schema::types::fixture::ParameterValue::Float(1.0),
                fade_in_ms: 0,
                fade_out_ms: 0,
                delay_in_ms: 0,
            }],
            follow_mode: FollowMode::Manual,
            fade_in_ms: 0,
            fade_out_ms: 0,
            is_active: false,
        };
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
        let sequence = a_sequence("Act 1", vec![cue.id]);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&sequence)).await.unwrap();

        let flow = Flow { id: Uuid::new_v4(), name: "Siren".into(), enabled: true };
        h.engine.set(create_path("flows"), Lifecycle::Persisted, json(&flow)).await.unwrap();
        let node = |kind| FlowNode {
            id: Uuid::new_v4(),
            flow_id: flow.id,
            kind,
            x: 0.0,
            y: 0.0,
            active: false,
            last_fired_at: None,
        };
        let source = node(FlowNodeKind::Source(TriggerSource::Parameter {
            fixture_id: lamp.id,
            parameter: ParameterKind::Intensity,
        }));
        let gate = node(FlowNodeKind::Condition(TriggerCondition::Above(0.5)));
        let act = node(FlowNodeKind::Action(TriggerAction::SetParameter {
            fixture_id: switched.id,
            parameter: ParameterKind::Switch(0),
            value: pult_schema::types::fixture::ParameterValue::Bool(true),
        }));
        for n in [&source, &gate, &act] {
            h.engine.set(create_path("flow_nodes"), Lifecycle::Persisted, json(n)).await.unwrap();
        }
        for (from, to) in [(&source, &gate), (&gate, &act)] {
            h.engine
                .set(
                    create_path("flow_edges"),
                    Lifecycle::Persisted,
                    json(&FlowEdge {
                        id: Uuid::new_v4(),
                        flow_id: flow.id,
                        from_node: from.id,
                        from_port: 0,
                        to_node: to.id,
                        to_port: 0,
                    }),
                )
                .await
                .unwrap();
        }

        // Let the tick pick up what the new graph watches, then press Go.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        h.engine
            .set(field_path("sequences", sequence.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
            .await
            .unwrap();

        for _ in 0..100 {
            let values =
                h.engine.get(field_path("fixtures", switched.id, "live_values")).await.unwrap();
            if values["Switch:0"]["value"] == serde_json::json!(true) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the fade never reached the flow watching it");
    }

    #[tokio::test]
    async fn a_fade_on_a_fixture_nothing_watches_leaves_the_flow_alone() {
        // The other half of the gate: a graph reacts to the parameter it names and
        // to nothing else, however much else is moving at the same time.
        let h = harness().await;
        let watched = a_fixture("Porch light", 1);
        let other = a_fixture("Backlight", 2);
        let switched = a_fixture("Siren", 3);
        for fixture in [&watched, &other, &switched] {
            h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(fixture)).await.unwrap();
        }

        // The cue moves `other`; the flow is watching `watched`.
        let cue = Cue {
            id: Uuid::new_v4(),
            name: "House".into(),
            number: 1.0,
            captures: vec![pult_schema::types::cue::ParameterCapture {
                fixture_id: other.id,
                parameter_kind: ParameterKind::Intensity,
                value: pult_schema::types::fixture::ParameterValue::Float(1.0),
                fade_in_ms: 0,
                fade_out_ms: 0,
                delay_in_ms: 0,
            }],
            follow_mode: FollowMode::Manual,
            fade_in_ms: 0,
            fade_out_ms: 0,
            is_active: false,
        };
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
        let sequence = a_sequence("Act 1", vec![cue.id]);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&sequence)).await.unwrap();

        let flow = Flow { id: Uuid::new_v4(), name: "Siren".into(), enabled: true };
        h.engine.set(create_path("flows"), Lifecycle::Persisted, json(&flow)).await.unwrap();
        let node = |kind| FlowNode {
            id: Uuid::new_v4(),
            flow_id: flow.id,
            kind,
            x: 0.0,
            y: 0.0,
            active: false,
            last_fired_at: None,
        };
        let source = node(FlowNodeKind::Source(TriggerSource::Parameter {
            fixture_id: watched.id,
            parameter: ParameterKind::Intensity,
        }));
        let gate = node(FlowNodeKind::Condition(TriggerCondition::Above(0.5)));
        let act = node(FlowNodeKind::Action(TriggerAction::SetParameter {
            fixture_id: switched.id,
            parameter: ParameterKind::Switch(0),
            value: pult_schema::types::fixture::ParameterValue::Bool(true),
        }));
        for n in [&source, &gate, &act] {
            h.engine.set(create_path("flow_nodes"), Lifecycle::Persisted, json(n)).await.unwrap();
        }
        for (from, to) in [(&source, &gate), (&gate, &act)] {
            h.engine
                .set(
                    create_path("flow_edges"),
                    Lifecycle::Persisted,
                    json(&FlowEdge {
                        id: Uuid::new_v4(),
                        flow_id: flow.id,
                        from_node: from.id,
                        from_port: 0,
                        to_node: to.id,
                        to_port: 0,
                    }),
                )
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        h.engine
            .set(field_path("sequences", sequence.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let values = h.engine.get(field_path("fixtures", switched.id, "live_values")).await.unwrap();
        assert_eq!(
            values["Switch:0"], serde_json::Value::Null,
            "a fade on a fixture the graph does not name must not fire it",
        );
    }
}
