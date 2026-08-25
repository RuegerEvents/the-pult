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
    types::{cue::{Cue, FollowMode}, fixture::Fixture, sequence::Sequence, show::Show},
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
        universe: 1,
        dmx_address: address,
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
        h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap()["dmx_address"],
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
