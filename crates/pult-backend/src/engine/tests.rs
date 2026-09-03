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

use pult_schema::types::effect::{Easing, RunningFade};
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
        editing_cue: None,
        history_depth: pult_schema::types::show::HISTORY_DEPTH_DEFAULT,
        home_fade_ms: 0,
    }
}

fn a_sequence(name: &str, cue_ids: Vec<Uuid>) -> Sequence {
    Sequence { id: Uuid::new_v4(), name: name.into(), cue_ids, active_cue_index: None, went_at: None }
}

fn a_cue(name: &str, number: f64) -> Cue {
    Cue {
        id: Uuid::new_v4(),
        name: name.into(),
        number,
        captures: vec![],
        follow_mode: FollowMode::Manual,
        fade_in_ms: 3000,
        // Zero means "this cue does not split its fade", so everything takes the in
        // time in both directions. Which is what these tests have always assumed:
        // this said 3000 while nothing read it, and `an_intensity_cue(_, _, 0)` — a
        // cue asking to snap — would otherwise take three seconds to come down.
        fade_out_ms: 0,
        is_active: false,
    }
}

fn a_fixture(name: &str, address: u16) -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: name.into(),
        fixture_type_id: Uuid::new_v4(),
        address: FixtureAddress::dmx(1, address),
        position: None,
        sensed_values: Default::default(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
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

/// A write in a shape the schema has since grown out of comes back in the shape it
/// has now.
///
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
            ["breaks"][0]["address"],
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

/// Go walks the list and then stops walking. Running out of cues is not turning the
/// sequence off, and Off is the only thing that is — which is what makes "no cue
/// active" mean something playback can release on.
#[tokio::test]
async fn go_next_walks_the_cue_list_and_stays_on_the_last_one() {
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
    assert_eq!(active(&h).await, 1, "one Go too many holds what is showing");

    h.engine
        .set(field_path("sequences", seq.id, "off"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    assert!(active(&h).await.is_null(), "and Off is what takes it off");
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
        user_id: None,
        previous: None,
        undoes: None,
        gesture: None,
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
        FixtureType, ParameterDefinition, ParameterKind,
        ParameterValue,
    };

    let mut h = harness().await;
    let ft = FixtureType {
        id: Uuid::new_v4(),
        name: "Source Four".into(),
        manufacturer: "ETC".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition::new(
            ParameterKind::Intensity,
            ParameterValue::Float(0.0),
        )],
        ..FixtureType::default()
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

    // editing_cue is SYNCED, name is PERSISTED.
    let cue_id = Uuid::new_v4();
    h.engine
        .set(vec![PathSegment::Key("show".into()), PathSegment::Key("editing_cue".into())],
             Lifecycle::Synced, json(&cue_id))
        .await
        .unwrap();
    h.engine
        .set(vec![PathSegment::Key("show".into()), PathSegment::Key("name".into())],
             Lifecycle::Persisted, json(&"Macbeth"))
        .await
        .unwrap();

    let got = h.engine.get(key("show")).await.unwrap();
    assert_eq!(got["editing_cue"], json(&cue_id));
    assert_eq!(got["name"], "Macbeth");

    h.reload().await;
    let after = h.engine.get(key("show")).await.unwrap();
    assert_eq!(after["name"], "Macbeth");
    assert_eq!(after["editing_cue"], serde_json::Value::Null, "SYNCED field must not be persisted");
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
        .set_sensed_value(fixture.id, "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": true }))
        .await
        .unwrap();
    h.engine
        .set_sensed_value(fixture.id, "Temperature".into(), serde_json::json!({ "type": "Float", "value": 21.5 }))
        .await
        .unwrap();

    let values = h.engine.get(field_path("fixtures", fixture.id, "sensed_values")).await.unwrap();
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
            .set_sensed_value(
                fixture.id,
                "Contact:0".into(),
                serde_json::json!({ "type": "Bool", "value": value }),
            )
            .await
            .unwrap();
    }

    let values = h.engine.get(field_path("fixtures", fixture.id, "sensed_values")).await.unwrap();
    assert_eq!(values["Contact:0"]["value"], false);
}

#[tokio::test]
async fn a_live_value_for_a_fixture_that_is_not_patched_is_refused() {
    let h = harness().await;
    let result = h
        .engine
        .set_sensed_value(Uuid::new_v4(), "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": true }))
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
        .set_sensed_value(fixture.id, "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": true }))
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
            .set_sensed_value(
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

        // A flow drives a parameter the way a cue does — it takes the key and parks
        // the value there — rather than writing a number into a map nothing keeps.
        eventually("the lamp to come on", async || {
            h.engine.get(field_path("fixtures", lamp.id, "live_fades")).await.unwrap()["Switch:0"]
                ["to"]["value"]
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
        ..FixtureType::default()
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
        effect: None,
        easing: Easing::Linear,
    }];
    cue
}

/// What a fixture's intensity is putting out right now.
///
/// Evaluated rather than read, because nothing stores it any more: this is the same
/// stack an output connector and a browser evaluate — the fade or shape driving the
/// parameter, the programmer over it, the home value under it — asked about this
/// moment. A test asserting on this is asserting what the rig is doing.
async fn intensity_of(h: &Harness, fixture_id: Uuid) -> f32 {
    match value_of(h, fixture_id, "Intensity").await {
        Some(ParameterValue::Float(level)) => level,
        _ => f32::NAN,
    }
}

async fn value_of(h: &Harness, fixture_id: Uuid, parameter: &str) -> Option<ParameterValue> {
    let row = h.engine.get(entity_path("fixtures", fixture_id)).await.ok()?;
    let fixture: pult_schema::types::fixture::Fixture = serde_json::from_value(row).ok()?;
    let types: Vec<pult_schema::types::fixture::FixtureType> = h
        .engine
        .get(key("fixture_types"))
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let entries: Vec<pult_schema::types::programmer::ProgrammerValue> = h
        .engine
        .get(key("programmer_values"))
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let held = pult_schema::types::fixture::HeldByProgrammer::of(&entries);
    pult_schema::types::fixture::value_at(
        &fixture,
        types.iter().find(|t| t.id == fixture.fixture_type_id),
        held.get(fixture_id, parameter),
        parameter,
        pult_schema::types::sequence::now_ms(),
    )
}

/// A dimmer that says where it rests, so a fade from nothing has somewhere to start.
async fn a_dimmer_type(h: &Harness) -> Uuid {
    use pult_schema::types::fixture::{
        FixtureType, ParameterDefinition, ParameterKind,
    };
    let ft = FixtureType {
        id: Uuid::new_v4(),
        name: "Source Four".into(),
        manufacturer: "ETC".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition::new(
            ParameterKind::Intensity,
            ParameterValue::Float(0.0),
        )],
        ..FixtureType::default()
    };
    h.engine.set(create_path("fixture_types"), Lifecycle::Persisted, json(&ft)).await.unwrap();
    ft.id
}

#[tokio::test]
async fn taking_a_cue_fades_the_fixture_up() {
    let h = harness().await;
    let mut fixture = a_fixture("Spot L", 1);
    // Patched as something, so the console knows the parameter rests dark and the
    // fade has a beginning. Without a type nothing can say, and the cue lands.
    fixture.fixture_type_id = a_dimmer_type(&h).await;
    // Short, and run in real time. This used to be a four-second fade fast-forwarded
    // with `tokio::time::pause()`, which worked while a fade was measured against the
    // monotonic clock. A fade is measured against the show clock now — the same
    // console milliseconds an effect always used, and the only clock a browser can
    // share — and that one does not fast-forward, so the fade has to be short enough
    // to sit through. Twelve ticks at 25 ms is plenty of resolution to catch a middle.
    let cue = an_intensity_cue(fixture.id, 1.0, 1000);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let midway = intensity_of(&h, fixture.id).await;
    assert!(midway > 0.1 && midway < 0.9, "expected a partial level midway, got {midway}");

    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
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
    first.follow_mode = FollowMode::FollowAfter { delay_ms: 300 };
    let second = an_intensity_cue(fixture.id, 0.0, 0);
    let seq = a_sequence("Act 1", vec![first.id, second.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&first)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&second)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    // Real time, deliberately. A follow is due at a console millisecond, and the show
    // clock advances monotonically from one wall reading rather than from tokio's, so
    // `pause()` no longer fast-forwards one. Three hundred milliseconds is short
    // enough to sit through and long enough that the first assertion lands inside it.
    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(h.engine.get(entity_path("sequences", seq.id)).await.unwrap()["active_cue_index"], 0);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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

    let mut updates = h
        .engine
        .subscribe_pattern(PathPattern::new(&format!("fixtures/{}/live_fades", fixture.id)))
        .await;

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();

    // The description, not the value. That is the whole of what a frontend is sent
    // now: where the parameter is going, when it started and how long it takes, from
    // which the browser works out what to draw at its own refresh.
    let fades = updates.next().await.expect("expected a live-fades broadcast");
    assert_eq!(fades["Intensity"]["to"]["value"], 1.0);
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

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(intensity_of(&h, fixture.id).await, 1.0);

    h.reload().await;
    let after = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert!(
        after["live_fades"].as_object().is_none_or(|m| m.is_empty()),
        "what is driving a parameter is output, not show data",
    );
    assert!(
        after["sensed_values"].as_object().is_none_or(|m| m.is_empty()),
        "and neither is what a device reported",
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
        found[0].live_fades.get("Intensity").map(|fade| fade.to.clone()),
        Some(ParameterValue::Float(1.0)),
        "output must see what playback just put on the parameter",
    );
}

#[tokio::test]
async fn unpatching_the_last_fixture_tells_the_output_plugins_so() {
    use crate::infra::connectors::{OutputCommand, OutputHandle};

    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let (mut engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
    let (tx, mut output_rx) = tokio::sync::mpsc::channel(16);
    engine.set_output(OutputHandle(tx));
    tokio::spawn(engine.run());

    let fixture = a_fixture("Spot L", 1);
    handle.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    // Wait for the patch that carries it before taking it away. The engine coalesces
    // a burst of writes into one pass, so a create and a delete issued back to back
    // would rightly produce a single push and there would be nothing to follow.
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while let Some(cmd) = output_rx.recv().await {
            let OutputCommand::Patch { fixtures, .. } = cmd else { continue };
            if fixtures.iter().any(|f| f.id == fixture.id) {
                return;
            }
        }
        panic!("output channel closed before the fixture was pushed");
    })
    .await
    .expect("the fixture should reach the plugins within two seconds");

    handle
        .set(delete_path("fixtures", fixture.id), Lifecycle::Persisted, serde_json::Value::Null)
        .await
        .unwrap();

    // Unpatching has to push a patch without it, or a plugin that remembered the
    // fixture keeps remembering it.
    let saw_empty = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while let Some(cmd) = output_rx.recv().await {
            let OutputCommand::Patch { fixtures, .. } = cmd else { continue };
            if !fixtures.iter().any(|f| f.id == fixture.id) {
                return true;
            }
        }
        false
    })
    .await
    .expect("the empty patch should follow within two seconds");
    assert!(saw_empty, "an empty patch follows the last fixture out");
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
    use pult_schema::types::{fixture::Vec3, scene::Transform};

    let h = harness().await;
    let mut fixture = a_fixture("Spot L", 1);
    fixture.position = Some(Transform::at(Vec3 { x: 1.0, y: 2.0, z: 3.0 }));
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
    use pult_schema::types::{fixture::Vec3, scene::Transform};

    let mut h = harness().await;
    let mut fixture = a_fixture("Spot L", 1);
    // Turned and mirrored, so the two halves a plain point would lose both have to
    // come back: a rotation, and a scale that is negative.
    fixture.position = Some(Transform {
        position: Vec3 { x: 1.5, y: 6.0, z: -2.0 },
        rotation: Vec3 { x: -90.0, y: 45.0, z: 0.0 },
        scale: Vec3 { x: -1.0, y: 1.0, z: 1.0 },
    });
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();

    h.reload().await;

    let after = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert_eq!(after["position"]["position"]["y"], 6.0);
    assert_eq!(after["position"]["rotation"]["x"], -90.0);
    assert_eq!(after["position"]["scale"]["x"], -1.0, "a reflection survives storage");
}

#[tokio::test]
async fn a_plugin_package_round_trips_through_the_showfile() {
    use pult_schema::types::plugin::{PluginPackage, PluginStage};

    let mut h = harness().await;
    let package = PluginPackage {
        id: Uuid::new_v4(),
        plugin_id: "command-line".into(),
        name: "Command Line".into(),
        version: "0.1.0".into(),
        api: "0.1".into(),
        sha256: "a3f9".repeat(16),
        enabled: true,
        stage: PluginStage::Runtime,
        config: serde_json::json!({ "prompt": ">" }),
    };
    h.engine.set(create_path("plugin_packages"), Lifecycle::Persisted, json(&package)).await.unwrap();

    h.reload().await;

    let after = h.engine.get(entity_path("plugin_packages", package.id)).await.unwrap();
    assert_eq!(after["plugin_id"], "command-line");
    assert_eq!(after["sha256"], package.sha256, "the digest is what finds the bytes again");
    assert_eq!(after["enabled"], true);
    assert_eq!(after["stage"], "Runtime");
    assert_eq!(after["config"]["prompt"], ">", "show-level config travels with the show");
}

#[tokio::test]
async fn a_plugin_package_written_before_stage_existed_still_reads() {
    let mut h = harness().await;
    // What an older build wrote: no `stage`, no `config`. Both are
    // `#[serde(default)]`, so the row is not a parse failure and the plugin is
    // not silently dropped from a show that has been opened on a newer console.
    let id = Uuid::new_v4();
    let older = serde_json::json!({
        "id": id,
        "plugin_id": "command-line",
        "name": "Command Line",
        "version": "0.1.0",
        "api": "0.1",
        "sha256": "b7c2".repeat(16),
        "enabled": true,
    });
    h.engine.set(create_path("plugin_packages"), Lifecycle::Persisted, older).await.unwrap();

    h.reload().await;

    let after = h.engine.get(entity_path("plugin_packages", id)).await.unwrap();
    assert_eq!(after["stage"], "Both", "the default is the permissive one");
    assert!(after["config"].is_null() || after["config"] == serde_json::json!(null));
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
                effect: None,
                easing: Easing::Linear,
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

        // The action drives the parameter, which shows up as a landed fade on it.
        for _ in 0..100 {
            let fades =
                h.engine.get(field_path("fixtures", switched.id, "live_fades")).await.unwrap();
            if fades["Switch:0"]["to"]["value"] == serde_json::json!(true) {
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
                effect: None,
                easing: Easing::Linear,
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

        let fades = h.engine.get(field_path("fixtures", switched.id, "live_fades")).await.unwrap();
        assert_eq!(
            fades["Switch:0"], serde_json::Value::Null,
            "a fade on a fixture the graph does not name must not fire it",
        );
    }

    /// The sampler is proportional to what is watched rather than to the rig.
    ///
    /// This is the whole reason the gate exists, and the reason it moved: it used to
    /// be handed the values of every fixture the engine had just written, forty times
    /// a second, and throw away the ones nothing was looking at. Now the watched set
    /// decides the work, so a graph watching one lamp costs one evaluation a sample
    /// whether the rig is five fixtures or two thousand.
    #[tokio::test]
    async fn a_flow_watching_one_parameter_of_a_large_rig_samples_one() {
        let h = harness().await;
        let fixture_type = a_dimmer_type(&h).await;

        // A rig large enough that sampling it would plainly cost something, all under
        // one cue, all fading. Five hundred rather than the two thousand of the `huge`
        // preset only because every one of them is patched over the wire here; the
        // property is that the number below does not move whichever it is.
        let rig: Vec<Fixture> = (0..500)
            .map(|n| {
                let mut f = a_fixture(&format!("Spot {n}"), (n % 500) as u16 + 1);
                f.fixture_type_id = fixture_type;
                f
            })
            .collect();
        for fixture in &rig {
            h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(fixture)).await.unwrap();
        }
        let captures: Vec<_> = rig
            .iter()
            .map(|f| pult_schema::types::cue::ParameterCapture {
                fixture_id: f.id,
                parameter_kind: ParameterKind::Intensity,
                value: pult_schema::types::fixture::ParameterValue::Float(1.0),
                fade_in_ms: 0,
                fade_out_ms: 0,
                delay_in_ms: 0,
                effect: None,
                easing: Easing::Linear,
            })
            .collect();
        let cue = Cue {
            id: Uuid::new_v4(),
            name: "Everything".into(),
            number: 1.0,
            captures,
            follow_mode: FollowMode::Manual,
            fade_in_ms: 800,
            fade_out_ms: 0,
            is_active: false,
        };
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
        let sequence = a_sequence("Act 1", vec![cue.id]);
        h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&sequence)).await.unwrap();

        // Nothing watched: the fade runs and no graph is offered anything at all.
        h.engine
            .set(field_path("sequences", sequence.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            h.engine.sampled_parameters().await,
            0,
            "nothing is watching anything, so nothing is sampled",
        );

        // Now one lamp of the two thousand is watched, and the cue is retaken.
        let flow = Flow { id: Uuid::new_v4(), name: "Siren".into(), enabled: true };
        h.engine.set(create_path("flows"), Lifecycle::Persisted, json(&flow)).await.unwrap();
        let source = FlowNode {
            id: Uuid::new_v4(),
            flow_id: flow.id,
            kind: FlowNodeKind::Source(TriggerSource::Parameter {
                fixture_id: rig[7].id,
                parameter: ParameterKind::Intensity,
            }),
            x: 0.0,
            y: 0.0,
            active: false,
            last_fired_at: None,
        };
        h.engine.set(create_path("flow_nodes"), Lifecycle::Persisted, json(&source)).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert_eq!(
            h.engine.sampled_parameters().await,
            1,
            "one parameter watched is one parameter sampled, whatever the rig",
        );
    }
}

/// The claim the whole change rests on: a fade in progress costs the engine nothing.
///
/// One pass when the cue is taken, saying what is driving what, and then silence for
/// the length of the fade — no writes, no broadcasts, no timer firing on playback's
/// account. What moves in that window is the value, and it moves because time passes
/// rather than because anything wrote it down.
#[tokio::test]
async fn a_fade_in_progress_produces_no_writes_and_no_broadcasts() {
    let h = harness().await;
    let mut fixture = a_fixture("Spot L", 1);
    fixture.fixture_type_id = a_dimmer_type(&h).await;
    let cue = an_intensity_cue(fixture.id, 1.0, 2_000);
    let seq = a_sequence("Act 1", vec![cue.id]);

    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();

    h.engine
        .set(field_path("sequences", seq.id, "goNext"), Lifecycle::Synced, serde_json::json!({}))
        .await
        .unwrap();
    // Let the pass that takes the cue happen and settle.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let logged_before = showfile::oplog::len(&h.pool).await.unwrap();
    let mut updates = h.broadcast.0.subscribe();
    let began = intensity_of(&h, fixture.id).await;

    // Half a second in the middle of the fade, with nobody touching the show.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert!(updates.try_recv().is_err(), "a fade in progress broadcasts nothing");
    assert_eq!(
        showfile::oplog::len(&h.pool).await.unwrap(),
        logged_before,
        "and writes nothing down",
    );

    // And the value moved anyway, which is the point.
    let now = intensity_of(&h, fixture.id).await;
    assert!(now > began + 0.1, "the fade advanced without being driven: {began} then {now}");
    assert!(now < 1.0, "and is still going");
}

/// The same for a show that is not doing anything at all. A settled station has no
/// periodic work left: no tick, no sampler, nothing to wake up for.
#[tokio::test]
async fn a_settled_show_does_no_periodic_work() {
    let h = harness().await;
    let fixture = a_fixture("Spot L", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let logged_before = showfile::oplog::len(&h.pool).await.unwrap();
    let mut updates = h.broadcast.0.subscribe();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert!(updates.try_recv().is_err(), "a settled show broadcasts nothing");
    assert_eq!(showfile::oplog::len(&h.pool).await.unwrap(), logged_before);
}

/// `live_effects` is the first LOCAL entity field in the system, so this pins the two
/// halves of what LOCAL means: it reaches a frontend watching the fixture, and it
/// does not reach the showfile's oplog or, through that, any peer.
#[tokio::test]
async fn a_running_effect_is_broadcast_but_never_logged() {
    let h = harness().await;
    let fixture = a_fixture("Spot", 1);
    h.engine
        .set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture))
        .await
        .unwrap();

    let logged_before = showfile::oplog::len(&h.pool).await.unwrap();
    let mut updates = h.broadcast.0.subscribe();

    let running = serde_json::json!({
        "Intensity": {
            "effect_id": Uuid::nil(),
            "curve": { "Shape": "Sine" },
            "rate_hz": 1.0,
            "low": { "type": "Float", "value": 0.0 },
            "high": { "type": "Float", "value": 1.0 },
            "width": 0.5,
            "direction": "Forward",
            "phase": 0.0,
            "t0": 1_000,
            "source": "Programmer",
        }
    });
    h.engine
        .set(
            entity_field_path("fixtures", fixture.id, "live_effects"),
            Lifecycle::Local,
            running.clone(),
        )
        .await
        .unwrap();

    let back = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert_eq!(back["live_effects"], running, "readable back");

    let update = tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
        .await
        .expect("a frontend watching this fixture hears about it")
        .unwrap();
    assert!(format!("{update:?}").contains("live_effects"), "and it names the field");

    assert_eq!(
        showfile::oplog::len(&h.pool).await.unwrap(),
        logged_before,
        "LOCAL must not reach the oplog, and so must never reach a peer",
    );
}

/// The other half of LOCAL: it is not persisted either, so a reload starts with
/// nothing moving rather than with a description of a cycle that stopped hours ago.
#[tokio::test]
async fn running_effects_do_not_survive_a_reload() {
    let mut h = harness().await;
    let fixture = a_fixture("Spot", 1);
    h.engine
        .set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture))
        .await
        .unwrap();
    h.engine
        .set(
            entity_field_path("fixtures", fixture.id, "live_effects"),
            Lifecycle::Local,
            serde_json::json!({ "Intensity": null }),
        )
        .await
        .ok();

    h.reload().await;

    let back = h.engine.get(entity_path("fixtures", fixture.id)).await.unwrap();
    assert_eq!(back["live_effects"], serde_json::json!({}), "nothing is moving yet");
}

// ── Undo, over a real engine ──────────────────────────────────────────────────

/// A drag against a live engine: many writes to one path under one gesture, one
/// press to take them back, and the value that comes back is the one from before
/// the drag started rather than one frame into it.
#[tokio::test]
async fn one_press_takes_back_a_whole_drag() {
    let h = harness().await;
    let sam = Uuid::new_v4();
    let fixture = a_fixture("Spot L", 1);
    h.engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            json(&fixture),
        )
        .await
        .unwrap();

    let name = vec![
        PathSegment::Key("fixtures".into()),
        PathSegment::Id(fixture.id),
        PathSegment::Key("name".into()),
    ];
    let drag = Uuid::new_v4();
    for step in ["a", "b", "c", "d"] {
        h.engine
            .set_as(sam, Some(drag), name.clone(), Lifecycle::Persisted, json(&step))
            .await
            .unwrap();
    }

    let moved = h.engine.undo(sam, false).await;
    assert_eq!(moved.len(), 1, "one path moved, however many writes it took");
    assert_eq!(h.engine.get(name.clone()).await.unwrap(), json(&"Spot L"));

    // And it goes back where it ended, not one step in.
    let moved = h.engine.undo(sam, true).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(h.engine.get(name).await.unwrap(), json(&"d"));
}

/// Two fixtures added in one gesture are two writes to the same `fixtures/__create`
/// path. Collapsing by path would delete the first and leave the second standing.
#[tokio::test]
async fn one_press_takes_back_everything_a_gesture_made() {
    let h = harness().await;
    let sam = Uuid::new_v4();
    let gesture = Uuid::new_v4();

    for name in ["Spot L", "Spot R"] {
        h.engine
            .set_as(
                sam,
                Some(gesture),
                vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
                Lifecycle::Persisted,
                json(&a_fixture(name, 1)),
            )
            .await
            .unwrap();
    }
    let patched: Vec<Fixture> =
        serde_json::from_value(h.engine.get(key("fixtures")).await.unwrap()).unwrap();
    assert_eq!(patched.len(), 2, "both were patched");

    let moved = h.engine.undo(sam, false).await;
    assert_eq!(moved.len(), 2, "and both go away");
    let left: Vec<Fixture> =
        serde_json::from_value(h.engine.get(key("fixtures")).await.unwrap()).unwrap();
    assert!(left.is_empty(), "no half-undone gesture, got {left:?}");
}

/// Writes made without a gesture are still their own act, so nothing about the
/// ordinary single change moved.
#[tokio::test]
async fn a_change_with_no_gesture_is_still_one_press() {
    let h = harness().await;
    let sam = Uuid::new_v4();
    let fixture = a_fixture("Spot L", 1);
    h.engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            json(&fixture),
        )
        .await
        .unwrap();

    let name = vec![
        PathSegment::Key("fixtures".into()),
        PathSegment::Id(fixture.id),
        PathSegment::Key("name".into()),
    ];
    h.engine.set_as(sam, None, name.clone(), Lifecycle::Persisted, json(&"Downstage")).await.unwrap();
    h.engine.set_as(sam, None, name.clone(), Lifecycle::Persisted, json(&"Upstage")).await.unwrap();

    assert_eq!(h.engine.undo(sam, false).await.len(), 1);
    assert_eq!(h.engine.get(name.clone()).await.unwrap(), json(&"Downstage"), "one step back");
    assert_eq!(h.engine.undo(sam, false).await.len(), 1);
    assert_eq!(h.engine.get(name).await.unwrap(), json(&"Spot L"), "and another");
}

/// What a drag costs the log, end to end through a real engine.
///
/// Two seconds of dragging at forty frames a second across a selection of twenty is
/// 2,400 writes, and before folding it was 2,400 rows. The log wants where each
/// fixture ended up, which is twenty.
#[tokio::test]
async fn a_drag_costs_the_log_one_row_per_fixture() {
    let h = harness().await;
    let sam = Uuid::new_v4();
    let drag = Uuid::new_v4();

    let mut ids = Vec::new();
    for i in 0..20 {
        let fixture = a_fixture(&format!("Spot {i}"), i + 1);
        ids.push(fixture.id);
        h.engine
            .set(
                vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
                Lifecycle::Persisted,
                json(&fixture),
            )
            .await
            .unwrap();
    }
    let before = showfile::oplog::len(&h.pool).await.unwrap();

    for frame in 0..80 {
        for id in &ids {
            h.engine
                .set_as(
                    sam,
                    Some(drag),
                    vec![
                        PathSegment::Key("fixtures".into()),
                        PathSegment::Id(*id),
                        PathSegment::Key("name".into()),
                    ],
                    Lifecycle::Persisted,
                    json(&format!("frame {frame}")),
                )
                .await
                .unwrap();
        }
    }

    let written = showfile::oplog::len(&h.pool).await.unwrap() - before;
    assert_eq!(written, 20, "1,600 writes, one row per fixture");

    // And one press still takes the whole thing back to before the drag.
    let moved = h.engine.undo(sam, false).await;
    assert_eq!(moved.len(), 20);
    let after: Vec<Fixture> =
        serde_json::from_value(h.engine.get(key("fixtures")).await.unwrap()).unwrap();
    assert!(
        after.iter().all(|f| f.name.starts_with("Spot ")),
        "every fixture back to its own name, not to a frame of the drag"
    );
}

/// How far back Ctrl-Z reaches is the show's setting, not a constant. Two consoles
/// working one show read the same number because it is show data.
#[tokio::test]
async fn a_show_decides_how_far_back_undo_reaches() {
    let h = harness().await;
    let sam = Uuid::new_v4();
    let mut show = a_show();
    show.history_depth = pult_schema::types::show::HISTORY_DEPTH_MIN;
    h.engine.set(key("show"), Lifecycle::Persisted, json(&show)).await.unwrap();

    let fixture = a_fixture("Spot L", 1);
    h.engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            json(&fixture),
        )
        .await
        .unwrap();
    let name = vec![
        PathSegment::Key("fixtures".into()),
        PathSegment::Id(fixture.id),
        PathSegment::Key("name".into()),
    ];

    // Comfortably more changes than the show keeps, each its own act.
    for i in 0..30 {
        h.engine
            .set_as(sam, None, name.clone(), Lifecycle::Persisted, json(&format!("v{i}")))
            .await
            .unwrap();
    }

    // Half the window's worth of presses, not a whole one. Every undo writes a row
    // of its own into the same window, so a run of them meets itself in the middle:
    // after five, the window holds five reversals and the five changes they reversed,
    // and there is nothing left in it that is still in effect.
    for press in 0..5 {
        assert_eq!(h.engine.undo(sam, false).await.len(), 1, "press {press}");
    }
    assert!(h.engine.undo(sam, false).await.is_empty(), "and no further back than the show keeps");
}

/// The history panel never offers to take back something undo can no longer reach.
#[tokio::test]
async fn the_history_is_never_longer_than_the_show_keeps() {
    let h = harness().await;
    let sam = Uuid::new_v4();
    let mut show = a_show();
    show.history_depth = pult_schema::types::show::HISTORY_DEPTH_MIN;
    h.engine.set(key("show"), Lifecycle::Persisted, json(&show)).await.unwrap();

    for i in 0..40 {
        h.engine
            .set_as(
                sam,
                None,
                field_path_on_singleton("show", "name"),
                Lifecycle::Persisted,
                json(&format!("Act {i}")),
            )
            .await
            .unwrap();
    }

    // Asked for a hundred, given what the show keeps.
    let entries = h.engine.history(100).await;
    assert_eq!(entries.len() as u32, pult_schema::types::show::HISTORY_DEPTH_MIN);
}

// ── The show always has somebody ──────────────────────────────────────────────
//
// Undo is per person and an unattributed write can never be taken back, so a show
// with no users at all is a show where the first thing anybody does is permanent.
// These pin down that a show always has an operator, that seeding it twice is not a
// thing that happens, and that the seed itself is nobody's.

/// The users in the show, as the engine reports them.
async fn users_of(h: &Harness) -> Vec<serde_json::Value> {
    h.engine.get(key("users")).await.unwrap().as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn an_empty_showfile_gains_an_operator() {
    let mut h = harness().await;
    h.reload().await;

    let users = users_of(&h).await;
    assert_eq!(users.len(), 1, "exactly one user, not none and not two");
    assert_eq!(users[0]["id"], json(&User::DEFAULT_ID));
    assert_eq!(users[0]["name"], "Operator");

    // PERSISTED, so it is in the file rather than only in memory.
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(h.pool.as_ref())
        .await
        .unwrap();
    assert_eq!(rows, 1, "and written to the showfile");
}

/// `create_entity` does not check whether the id is already there, so this is the
/// guard rather than an optimisation: without it every load would rewrite the row,
/// and on a second station replicate "Operator" over a name somebody chose.
#[tokio::test]
async fn loading_a_show_that_has_one_writes_nothing() {
    let mut h = harness().await;
    h.reload().await;
    let before = oplog::len(&h.pool).await.unwrap();

    h.reload().await;
    h.reload().await;

    assert_eq!(users_of(&h).await.len(), 1, "still one user after three loads");
    assert_eq!(
        oplog::len(&h.pool).await.unwrap(),
        before,
        "and not one operation was written by the loads that found it"
    );
}

/// The rename is the case this actually protects. A second station loading a show
/// whose operator has been renamed must not put the default name back.
#[tokio::test]
async fn a_renamed_operator_survives_being_loaded_again() {
    let mut h = harness().await;
    h.reload().await;

    h.engine
        .set(
            field_path("users", User::DEFAULT_ID, "name"),
            Lifecycle::Persisted,
            serde_json::json!("Sam"),
        )
        .await
        .unwrap();
    h.reload().await;

    let users = users_of(&h).await;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "Sam", "the name somebody chose, not the one nobody did");
}

/// Nobody asked for the seed, so it is nobody's. An operator pressing Ctrl-Z on a
/// fresh show should reach their own first change, not the console inventing them.
#[tokio::test]
async fn the_seed_is_nobodys_and_cannot_be_taken_back() {
    let mut h = harness().await;
    h.reload().await;

    let all = oplog::since(&h.pool, &VectorClock::default()).await.unwrap();
    let seed: Vec<_> = all
        .iter()
        .filter(|op| matches!(op.path.first(), Some(PathSegment::Key(k)) if k == "users"))
        .collect();
    assert_eq!(seed.len(), 1, "the seed is logged, so a peer catching up learns of it");
    assert!(seed[0].user_id.is_none(), "and carries no author");
    assert!(!seed[0].is_undoable(), "so it cannot be undone");

    assert!(
        h.engine.history(100).await.is_empty(),
        "and it is not in the history of what people did"
    );
}

/// A showfile written before this change: no users, and operations already in the
/// log. It gains an operator on open, and its own history is left alone.
#[tokio::test]
async fn a_showfile_written_before_this_change_gains_one_on_open() {
    let mut h = harness().await;
    // Something somebody did, back when there was nobody to attribute it to.
    h.engine.set(key("show"), Lifecycle::Persisted, json(&a_show())).await.unwrap();
    let before: Vec<_> = oplog::since(&h.pool, &VectorClock::default())
        .await
        .unwrap()
        .into_iter()
        .map(|op| (op.id, op.user_id))
        .collect();
    assert!(!before.is_empty(), "the show was written before anybody existed");
    assert!(before.iter().all(|(_, user)| user.is_none()), "and nobody was attributed");

    h.reload().await;

    assert_eq!(users_of(&h).await.len(), 1, "opening it gives it an operator");
    let after = oplog::since(&h.pool, &VectorClock::default()).await.unwrap();
    for (id, user) in &before {
        let still = after.iter().find(|op| &op.id == id).expect("the old row is still there");
        assert_eq!(&still.user_id, user, "and is not attributed to anybody now");
        assert!(!still.is_undoable(), "so it stays un-undoable, as it always was");
    }
}

/// Deleting the operator is allowed — it is an ordinary row. The show would then
/// have nobody, so the next load gives it somebody again.
#[tokio::test]
async fn deleting_the_operator_and_loading_again_brings_one_back() {
    let mut h = harness().await;
    h.reload().await;

    let delete = vec![
        PathSegment::Key("users".into()),
        PathSegment::Id(User::DEFAULT_ID),
        PathSegment::Key("__delete".into()),
    ];
    h.engine.set(delete, Lifecycle::Persisted, serde_json::Value::Null).await.unwrap();
    assert!(users_of(&h).await.is_empty(), "gone, like any other user");

    h.reload().await;
    assert_eq!(users_of(&h).await.len(), 1, "and the show is not left with nobody");
}

/// Two stations, one operator. The id is a constant rather than something each
/// station invents, so both write the same row and the show ends with one user
/// however many consoles opened it.
#[tokio::test]
async fn two_stations_loading_one_show_end_with_one_operator() {
    let mut a = harness().await;
    let mut b = harness().await;
    a.reload().await;
    b.reload().await;

    let (users_a, users_b) = (users_of(&a).await, users_of(&b).await);
    assert_eq!(users_a.len(), 1, "one on this station");
    assert_eq!(users_b.len(), 1, "one on that station");
    assert_eq!(users_a[0]["id"], users_b[0]["id"], "and they agree which one it is");
    assert_eq!(users_a[0]["id"], json(&User::DEFAULT_ID));
}

/// A station joining a session takes the leader's state. It must not then seed a
/// second time — and because both stations compute the same id, the leader's
/// operator lands on the id the follower already had rather than beside it.
#[tokio::test]
async fn a_station_joining_a_session_keeps_the_leaders_operator() {
    let mut leader = harness().await;
    leader.reload().await;
    leader
        .engine
        .set(
            field_path("users", User::DEFAULT_ID, "name"),
            Lifecycle::Persisted,
            serde_json::json!("Sam"),
        )
        .await
        .unwrap();
    let snapshot = leader.engine.get_snapshot().await;

    let mut follower = harness().await;
    follower.reload().await;
    assert_eq!(users_of(&follower).await[0]["name"], "Operator", "its own, before joining");

    follower.engine.apply_state_snapshot(snapshot).await;
    let _ = follower.engine.get(key("users")).await;

    let users = users_of(&follower).await;
    assert_eq!(users.len(), 1, "not one each");
    assert_eq!(users[0]["name"], "Sam", "the leader's, not the one it made itself");
}

/// The end-to-end version of what 1.3's guard protects: a rename on one station is
/// not undone by another station opening the same showfile afterwards.
#[tokio::test]
async fn a_rename_on_one_station_survives_another_opening_the_show() {
    let mut first = harness().await;
    first.reload().await;
    first
        .engine
        .set(
            field_path("users", User::DEFAULT_ID, "name"),
            Lifecycle::Persisted,
            serde_json::json!("Sam"),
        )
        .await
        .unwrap();

    // A second station on the same showfile — the case where an unconditional seed
    // would write "Operator" back over the name somebody chose.
    let (engine, handle, _broadcast) =
        ShowEngine::new(NodeId(Uuid::new_v4()), first.pool.clone(), None);
    tokio::spawn(engine.run());
    let _ = handle.0.send(EngineCommand::LoadFromShowfile).await;
    let users = handle.get(key("users")).await.unwrap();
    let users = users.as_array().cloned().unwrap_or_default();

    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "Sam", "the second station left the chosen name alone");
}

/// The whole change, end to end: nobody has been asked anything, and the first
/// change is still one that can be taken back.
#[tokio::test]
async fn the_first_change_on_a_fresh_show_can_be_taken_back() {
    let mut h = harness().await;
    h.reload().await;

    let fixture = a_fixture("Spot 3", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    let name = field_path("fixtures", fixture.id, "name");

    // Written the way the WebSocket writes for a client that never said who it is:
    // as the show's operator, which the load above seeded.
    h.engine
        .set_as(User::DEFAULT_ID, None, name.clone(), Lifecycle::Persisted, json(&"Warmer"))
        .await
        .unwrap();
    assert_eq!(h.engine.get(name.clone()).await.unwrap(), json(&"Warmer"));

    let moved = h.engine.undo(User::DEFAULT_ID, false).await;

    assert_eq!(moved.len(), 1, "one change taken back");
    assert_eq!(
        h.engine.get(name).await.unwrap(),
        json(&"Spot 3"),
        "and the name is what it was before"
    );
}

/// And it is in the history, so the panel can show it and offer to take it back.
#[tokio::test]
async fn a_change_by_the_operator_is_in_the_history() {
    let mut h = harness().await;
    h.reload().await;

    let fixture = a_fixture("Spot 3", 1);
    h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
    h.engine
        .set_as(
            User::DEFAULT_ID,
            None,
            field_path("fixtures", fixture.id, "name"),
            Lifecycle::Persisted,
            json(&"Warmer"),
        )
        .await
        .unwrap();

    let history = h.engine.history(100).await;
    assert_eq!(history.len(), 1, "the operator's change, and not the engine's seed");
    assert_eq!(history[0].user_id, Some(User::DEFAULT_ID));
    assert!(history[0].is_undoable());
}

// ── The log has an end ────────────────────────────────────────────────────────
//
// Pruning runs off the actor's loop, so these assert what is in the showfile after
// giving the spawned task a moment rather than awaiting a reply it does not send.

use crate::infra::preferences::testing::own_file;

/// Wait for the log to reach `want` rows, or give up. The prune is spawned, so
/// there is nothing to await; polling is honest about that.
async fn settles_at(pool: &sqlx::SqlitePool, want: u64) -> u64 {
    for _ in 0..100 {
        let n = oplog::len(pool).await.unwrap();
        if n == want {
            return n;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    oplog::len(pool).await.unwrap()
}

/// Fill the log with operations nobody performed, aged past any window.
async fn old_telemetry(pool: &sqlx::SqlitePool, node: NodeId, count: u64) {
    for seq in 1..=count {
        let op = Operation {
            id: Uuid::new_v4(),
            node_id: node,
            seq,
            clock: VectorClock::default(),
            lifecycle: Lifecycle::Synced,
            path: vec![PathSegment::Key("stations".into()), PathSegment::Id(node.0)],
            value: serde_json::json!({ "cpu": 1 }),
            timestamp: Utc::now() - chrono::Duration::hours(6),
            user_id: None,
            previous: None,
            undoes: None,
            gesture: None,
        };
        crate::infra::showfile::oplog::append(pool, &op).await.unwrap();
    }
}

/// A showfile that has been round a long tech week. Opening it is where the largest
/// cut this will ever take happens.
#[tokio::test]
async fn opening_an_oversized_showfile_brings_the_log_within_the_retention() {
    let _own = own_file();
    let mut h = harness().await;
    old_telemetry(&h.pool, NodeId(Uuid::new_v4()), 500).await;
    assert_eq!(oplog::len(&h.pool).await.unwrap(), 500);

    h.reload().await;

    // One row survives: the operator this load seeded, which is unattributed too but
    // written just now. That the window tells those two apart is the point.
    assert_eq!(settles_at(&h.pool, 1).await, 1, "six hours of telemetry is past the hour kept");
    let left = oplog::since(&h.pool, &VectorClock::default()).await.unwrap();
    assert!(
        matches!(left[0].path.first(), Some(PathSegment::Key(k)) if k == "users"),
        "and what is left is the recent write, not the old ones"
    );
    assert!(!oplog::floor(&h.pool).await.unwrap().is_empty(), "and the floor says so");
}

/// The case bounding-at-open misses: a station left up for a fortnight.
#[tokio::test]
async fn a_running_show_prunes_without_being_restarted() {
    let _own = own_file();
    let mut h = harness().await;
    h.reload().await;
    old_telemetry(&h.pool, NodeId(Uuid::new_v4()), 300).await;

    // Enough ordinary writes to cross the threshold while the show is up.
    let seq = a_sequence("Act 1", vec![]);
    h.engine.set(create_path("sequences"), Lifecycle::Persisted, json(&seq)).await.unwrap();
    let name = field_path("sequences", seq.id, "name");
    for i in 0..super::APPENDS_BETWEEN_PRUNES {
        h.engine.set(name.clone(), Lifecycle::Persisted, json(&format!("v{i}"))).await.unwrap();
    }

    // The old telemetry goes; the show's own recent writes stay.
    for _ in 0..100 {
        if !oplog::floor(&h.pool).await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!oplog::floor(&h.pool).await.unwrap().is_empty(), "it pruned while running");
    assert_eq!(
        h.engine.get(name).await.unwrap(),
        json(&format!("v{}", super::APPENDS_BETWEEN_PRUNES - 1)),
        "and the show is exactly where it was left"
    );
}

/// Two triggers arriving together must not become two deletes racing on the floor.
#[tokio::test]
async fn two_triggers_at_once_produce_one_prune() {
    let _own = own_file();
    let mut h = harness().await;
    old_telemetry(&h.pool, NodeId(Uuid::new_v4()), 200).await;

    // Three loads in quick succession, each of which asks for a prune.
    h.reload().await;
    h.reload().await;
    h.reload().await;

    // The 200 old rows go; the operator the first load seeded stays.
    assert_eq!(settles_at(&h.pool, 1).await, 1, "the log is cut");
    let floor = oplog::floor(&h.pool).await.unwrap();
    assert_eq!(floor.len(), 1, "and one floor row, not one per prune");
}

/// A `DELETE` over a long log inside the actor's loop would be a stalled tick.
/// Spawning it is what keeps output running, so that is what this asserts.
#[tokio::test]
async fn output_keeps_running_while_the_log_is_pruned() {
    let _own = own_file();
    let mut h = harness().await;
    h.reload().await;
    old_telemetry(&h.pool, NodeId(Uuid::new_v4()), 2_000).await;

    // The engine answers while the prune is in flight. A stalled actor would leave
    // these hanging until the delete finished.
    let before = std::time::Instant::now();
    for i in 0..super::APPENDS_BETWEEN_PRUNES {
        h.engine.set(key("show"), Lifecycle::Persisted, json(&a_show())).await.unwrap();
        if i % 200 == 0 {
            assert!(h.engine.get(key("show")).await.is_ok(), "the engine is still answering");
        }
    }
    let elapsed = before.elapsed();

    // The old telemetry goes, so what is left is this test's own writes. Polled
    // rather than awaited: the prune is spawned, which is the property under test.
    let ceiling = super::APPENDS_BETWEEN_PRUNES as u64 + 1;
    let mut left = oplog::len(&h.pool).await.unwrap();
    for _ in 0..100 {
        if left <= ceiling {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        left = oplog::len(&h.pool).await.unwrap();
    }
    assert!(left <= ceiling, "the 2,000 old rows went: {left}");
    // Not a benchmark — a canary. The writes above are microseconds each; if the
    // actor had waited on a 2,000-row delete this would be seconds.
    assert!(elapsed.as_secs() < 10, "the loop was blocked by the prune: {elapsed:?}");
}

// ── Saved groups ──────────────────────────────────────────────────────────────

/// A station's answer to "what is in this group" is worked out from the rig as it is
/// now, which is what makes a group survive somebody re-patching the show.
mod groups {
    use pult_schema::types::{
        fixture::Vec3,
        scene::Transform,
        Group, SelectionClause, SelectionCombine, SelectionOrder, SelectionQuery, SelectionTerm,
    };
    use serde_json::json;

    use super::*;
    use crate::api::rpcs::{self, LocalRpcDeps};
    use crate::infra::{devices::DeviceHandle, session::SessionHandle};

    fn deps(engine: &EngineHandle) -> LocalRpcDeps {
        // The session and device channels go nowhere: `group.resolve` reads the show,
        // and a test that reached either of them would be testing something else.
        let (session_tx, _session_rx) = tokio::sync::mpsc::channel(1);
        let (device_tx, _device_rx) = tokio::sync::mpsc::channel(1);
        LocalRpcDeps {
            session: SessionHandle(session_tx),
            devices: DeviceHandle(device_tx),
            engine: engine.clone(),
            // Nothing here asks about the log, and a station without one is a
            // real configuration rather than a test fiction.
            log: None,
            log_watchers: Default::default(),
            sync: None,
            caller: None,
            clients: None,
            ws_registry: None,
        }
    }

    fn at(name: &str, x: f32, type_id: Uuid) -> Fixture {
        let mut fixture = a_fixture(name, 1);
        fixture.fixture_type_id = type_id;
        fixture.position = Some(Transform::at(Vec3 { x, y: 5.0, z: 2.0 }));
        fixture
    }

    fn of_type(type_id: Uuid) -> SelectionQuery {
        SelectionQuery {
            clauses: vec![SelectionClause {
                combine: SelectionCombine::Add,
                term: SelectionTerm::OfType { type_id },
            }],
            order: SelectionOrder::ByName,
        }
    }

    #[tokio::test]
    async fn a_group_resolves_against_the_rig_as_it_is_now() {
        let h = harness().await;
        let movers = Uuid::new_v4();
        let pars = Uuid::new_v4();
        for fixture in [at("Mover B", 1.0, movers), at("Mover A", -1.0, movers), at("Par", 0.0, pars)]
        {
            h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        }

        let group = Group { id: Uuid::new_v4(), name: "Movers".into(), query: of_type(movers) };
        h.engine.set(create_path("groups"), Lifecycle::Persisted, json(&group)).await.unwrap();

        let resolved = rpcs::dispatch(
            "selection.resolve",
            json!({ "groupId": group.id }),
            &deps(&h.engine),
        )
        .await
        .expect("the group resolves");
        let names = |v: &serde_json::Value| -> Vec<String> {
            serde_json::from_value::<Vec<Uuid>>(v.clone()).unwrap().iter().map(|i| i.to_string()).collect()
        };
        assert_eq!(names(&resolved).len(), 2, "two movers, not the par");

        // And a mover hung after the group was saved is in it, with nothing re-saved.
        let late = at("Mover C", 3.0, movers);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&late)).await.unwrap();
        let again = rpcs::dispatch(
            "selection.resolve",
            json!({ "groupId": group.id }),
            &deps(&h.engine),
        )
        .await
        .unwrap();
        let ids: Vec<Uuid> = serde_json::from_value(again).unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&late.id), "a fixture patched afterwards joins the group");
    }

    /// A group that is not there is an error, not an empty answer — a command line has
    /// to be able to say "you have no such group" rather than "that group is empty".
    #[tokio::test]
    async fn resolving_a_group_that_is_not_there_says_so() {
        let h = harness().await;
        let missing = Uuid::new_v4();
        let err = rpcs::dispatch("selection.resolve", json!({ "groupId": missing }), &deps(&h.engine))
            .await
            .expect_err("no such group");
        assert!(err.contains(&missing.to_string()), "the message names the group: {err}");

        // Whereas a group whose query currently matches nothing resolves to nothing.
        let empty =
            Group { id: Uuid::new_v4(), name: "Nothing".into(), query: of_type(Uuid::new_v4()) };
        h.engine.set(create_path("groups"), Lifecycle::Persisted, json(&empty)).await.unwrap();
        let resolved =
            rpcs::dispatch("selection.resolve", json!({ "groupId": empty.id }), &deps(&h.engine))
                .await
                .expect("an empty group is still a group");
        assert_eq!(resolved, json!([]));
    }

    /// Resolving is a read. It must not appear in anybody's history or undo stack,
    /// which is why it is a station RPC rather than a command on the entity.
    #[tokio::test]
    async fn resolving_writes_no_operation() {
        let h = harness().await;
        let type_id = Uuid::new_v4();
        h.engine
            .set(create_path("fixtures"), Lifecycle::Persisted, json(&at("Spot", 0.0, type_id)))
            .await
            .unwrap();
        let group = Group { id: Uuid::new_v4(), name: "Spots".into(), query: of_type(type_id) };
        h.engine.set(create_path("groups"), Lifecycle::Persisted, json(&group)).await.unwrap();

        let before = showfile::oplog::len(&h.pool).await.unwrap();
        for _ in 0..5 {
            rpcs::dispatch("selection.resolve", json!({ "groupId": group.id }), &deps(&h.engine))
                .await
                .unwrap();
        }
        assert_eq!(
            showfile::oplog::len(&h.pool).await.unwrap(),
            before,
            "a read must not write history"
        );
    }

    /// A group written on one station reaches the other and means the same there.
    ///
    /// The point of storing the *query* rather than the ids: both stations run the
    /// same evaluator over their own copy of the rig, so the answer agrees because
    /// the question does, not because anybody replicated an answer.
    #[tokio::test]
    async fn two_stations_resolve_a_group_the_same_way() {
        use pult_schema::events::operation::{Operation, VectorClock};

        let here = harness().await;
        let there = harness().await;
        let type_id = Uuid::new_v4();

        // The same rig on both, in the same order — which is what the engine's
        // per-collection display order guarantees for replicated shows.
        let rig = [at("Mover B", 1.0, type_id), at("Mover A", -1.0, type_id)];
        for station in [&here, &there] {
            for f in &rig {
                station
                    .engine
                    .set(create_path("fixtures"), Lifecycle::Persisted, json(f))
                    .await
                    .unwrap();
            }
        }

        let group = Group {
            id: Uuid::new_v4(),
            name: "Movers".into(),
            query: SelectionQuery {
                clauses: vec![SelectionClause {
                    combine: SelectionCombine::Add,
                    term: SelectionTerm::OfType { type_id },
                }],
                // A hand order, which is exactly the one that used to live in a
                // browser store and could not travel.
                order: SelectionOrder::Manual { order: vec![rig[1].id, rig[0].id] },
            },
        };
        here.engine.set(create_path("groups"), Lifecycle::Persisted, json(&group)).await.unwrap();

        // The other station learns of it the way it learns of anything: an operation.
        there
            .engine
            .0
            .send(EngineCommand::ApplyPeerOperation(Operation {
                id: Uuid::new_v4(),
                node_id: NodeId(Uuid::new_v4()),
                seq: 1,
                clock: VectorClock::default(),
                path: create_path("groups"),
                value: json(&group),
                lifecycle: Lifecycle::Persisted,
                timestamp: Utc::now(),
                user_id: None,
                previous: None,
                undoes: None,
                gesture: None,
            }))
            .await
            .unwrap();

        let ask = |h: &Harness| {
            let deps = deps(&h.engine);
            async move {
                rpcs::dispatch("selection.resolve", json!({ "groupId": group.id }), &deps)
                    .await
                    .expect("both stations resolve it")
            }
        };
        let mine = ask(&here).await;
        let theirs = ask(&there).await;
        assert_eq!(mine, theirs, "the same question, the same answer");
        assert_eq!(
            serde_json::from_value::<Vec<Uuid>>(mine).unwrap(),
            vec![rig[1].id, rig[0].id],
            "and in the order somebody dragged, which travelled inside the query"
        );
    }

    /// Nothing here is special: a group is a PERSISTED row, so it is attributed,
    /// it is in the history of what people did, and it comes back on undo.
    #[tokio::test]
    async fn a_group_is_an_ordinary_show_edit() {
        use crate::infra::showfile::oplog;

        let h = harness().await;
        let user = Uuid::new_v4();
        let group = Group {
            id: Uuid::new_v4(),
            name: "Specials".into(),
            query: of_type(Uuid::new_v4()),
        };
        h.engine
            .set_as(user, None, create_path("groups"), Lifecycle::Persisted, json(&group))
            .await
            .unwrap();
        h.engine
            .set_as(
                user,
                None,
                vec![
                    PathSegment::Key("groups".into()),
                    PathSegment::Id(group.id),
                    PathSegment::Key("name".into()),
                ],
                Lifecycle::Persisted,
                json(&"The specials"),
            )
            .await
            .unwrap();

        let log = oplog::recent_by_people(&h.pool, 100).await.unwrap();
        assert!(
            log.iter().any(|op| op.user_id == Some(user)
                && op.path.iter().any(|s| matches!(s, PathSegment::Key(k) if k == "groups"))),
            "the rename shows up as that user's"
        );

        // Deleted, then taken back — with its name and its query.
        h.engine
            .set_as(
                user,
                None,
                delete_path("groups", group.id),
                Lifecycle::Persisted,
                serde_json::Value::Null,
            )
            .await
            .unwrap();
        assert!(h.engine.get(entity_path("groups", group.id)).await.is_err(), "it went");

        h.engine.undo(user, false).await;
        let back = h.engine.get(entity_path("groups", group.id)).await.expect("it came back");
        assert_eq!(back["name"], "The specials");
        assert_eq!(back["query"], json(&group.query));
    }

    /// The showfile carries the query, so a station that opens the show later — or a
    /// peer catching up — resolves the same fixtures in the same order.
    #[tokio::test]
    async fn a_group_survives_the_showfile() {
        let mut h = harness().await;
        let type_id = Uuid::new_v4();
        for f in [at("B", 1.0, type_id), at("A", -1.0, type_id)] {
            h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&f)).await.unwrap();
        }
        let group = Group { id: Uuid::new_v4(), name: "All".into(), query: of_type(type_id) };
        h.engine.set(create_path("groups"), Lifecycle::Persisted, json(&group)).await.unwrap();

        let before =
            rpcs::dispatch("selection.resolve", json!({ "groupId": group.id }), &deps(&h.engine))
                .await
                .unwrap();

        h.reload().await;

        let after =
            rpcs::dispatch("selection.resolve", json!({ "groupId": group.id }), &deps(&h.engine))
                .await
                .unwrap();
        assert_eq!(before, after, "the same question, the same answer, after a reopen");
        assert_eq!(
            serde_json::from_value::<Vec<Uuid>>(after).unwrap().len(),
            2,
            "and it is not empty, which would make the comparison meaningless"
        );
    }
}


// ── Relative writes ───────────────────────────────────────────────────────────

/// "Ten percent brighter" rather than "at 62%".
///
/// The property every one of these is really about: the delta stops existing at the
/// front door. What the log holds, what a peer receives and what undo reverses are
/// all absolute, because two stations each adding ten percent to whatever they
/// happened to be showing would not end up holding the same number.
mod relative {
    use pult_schema::types::{
        fixture::{ParameterKind, ParameterValue},
        programmer::{programmer_entry_id, ProgrammerValue},
    };
    use serde_json::json;

    use super::*;
    use crate::infra::showfile::oplog;

    fn by_path(mut path: Path) -> Path {
        path.push(PathSegment::Key("__by".into()));
        path
    }

    fn programmer_by(fixture_id: Uuid, kind: ParameterKind, by: f64) -> (Path, serde_json::Value) {
        (
            vec![
                PathSegment::Key("programmer_values".into()),
                PathSegment::Key("__by".into()),
            ],
            json!({ "fixtureId": fixture_id, "parameterKind": kind, "by": by }),
        )
    }

    fn held(h: &Harness, fixture_id: Uuid, kind: &ParameterKind) -> Path {
        let key = crate::model::playback::parameter_key(kind);
        let _ = h;
        vec![
            PathSegment::Key("programmer_values".into()),
            PathSegment::Id(programmer_entry_id(&fixture_id.to_string(), &key).parse().unwrap()),
        ]
    }

    fn a_programmer_value(fixture_id: Uuid, kind: ParameterKind, value: f32) -> ProgrammerValue {
        let key = crate::model::playback::parameter_key(&kind);
        ProgrammerValue {
            id: programmer_entry_id(&fixture_id.to_string(), &key).parse().unwrap(),
            fixture_id,
            parameter_kind: kind,
            value: ParameterValue::Float(value),
            effect: None,
            locked: false,
        }
    }

    fn level(row: &serde_json::Value) -> f32 {
        row["value"]["value"].as_f64().unwrap_or(f64::NAN) as f32
    }

    #[tokio::test]
    async fn an_ordinary_field_moves_by_the_delta() {
        let h = harness().await;
        let cue = a_cue("Act 1", 1.0);
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();

        h.engine
            .set(
                by_path(field_path("cues", cue.id, "fade_in_ms")),
                Lifecycle::Persisted,
                json!(1500),
            )
            .await
            .unwrap();

        assert_eq!(
            h.engine.get(field_path("cues", cue.id, "fade_in_ms")).await.unwrap(),
            json!(4500),
            "3000 and 1500 more"
        );
    }

    #[tokio::test]
    async fn a_held_parameter_moves_from_where_it_is_held() {
        let h = harness().await;
        let fixture = a_fixture("Spot", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        let entry = a_programmer_value(fixture.id, ParameterKind::Intensity, 0.5);
        h.engine
            .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
            .await
            .unwrap();

        let (path, args) = programmer_by(fixture.id, ParameterKind::Intensity, 0.1);
        h.engine.set(path, Lifecycle::Synced, args).await.unwrap();

        let row = h.engine.get(held(&h, fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert!((level(&row) - 0.6).abs() < 1e-5, "{row}");
    }

    /// The ordinary case: nobody is holding the fader yet, so the nudge has to take
    /// the key and start from whatever playback has it at.
    #[tokio::test]
    async fn an_unheld_parameter_is_taken_from_where_playback_has_it() {
        let h = harness().await;
        let fixture = a_fixture("Spot", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        // Playback has it at 0.4: a landed fade, which is where a parameter sits when
        // something has driven it and nothing stores the number.
        h.engine
            .set(
                field_path("fixtures", fixture.id, "live_fades"),
                Lifecycle::Local,
                serde_json::json!({
                    "Intensity": {
                        "from": ParameterValue::Float(0.4),
                        "to": ParameterValue::Float(0.4),
                        "t0": 0,
                        "duration_ms": 0,
                        "easing": "Step",
                        "cue_id": Uuid::nil(),
                    }
                }),
            )
            .await
            .unwrap();

        let (path, args) = programmer_by(fixture.id, ParameterKind::Intensity, 0.1);
        h.engine.set(path, Lifecycle::Synced, args).await.unwrap();

        let row = h.engine.get(held(&h, fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert!((level(&row) - 0.5).abs() < 1e-5, "{row}");
        assert_eq!(row["fixture_id"], json(&fixture.id), "and it names the fixture it took");
    }

    #[tokio::test]
    async fn two_nudges_both_land() {
        let h = harness().await;
        let fixture = a_fixture("Spot", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        let entry = a_programmer_value(fixture.id, ParameterKind::Intensity, 0.5);
        h.engine
            .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
            .await
            .unwrap();

        for _ in 0..2 {
            let (path, args) = programmer_by(fixture.id, ParameterKind::Intensity, 0.1);
            h.engine.set(path, Lifecycle::Synced, args).await.unwrap();
        }

        let row = h.engine.get(held(&h, fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert!((level(&row) - 0.7).abs() < 1e-5, "neither nudge was lost: {row}");
    }

    #[tokio::test]
    async fn a_nudge_past_the_top_comes_to_rest_there() {
        let h = harness().await;
        let fixture = a_fixture("Spot", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        let entry = a_programmer_value(fixture.id, ParameterKind::Intensity, 0.95);
        h.engine
            .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
            .await
            .unwrap();

        let (path, args) = programmer_by(fixture.id, ParameterKind::Intensity, 0.2);
        h.engine.set(path, Lifecycle::Synced, args).await.unwrap();

        let row = h.engine.get(held(&h, fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert_eq!(level(&row), 1.0);
    }

    /// Nudging a shape would have to mean moving its offset, which is a different
    /// feature wearing the same word.
    #[tokio::test]
    async fn a_running_shape_refuses_and_goes_on_running() {
        use pult_schema::types::effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread};

        let h = harness().await;
        let fixture = a_fixture("Spot", 1);
        h.engine.set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture)).await.unwrap();
        let mut entry = a_programmer_value(fixture.id, ParameterKind::Intensity, 0.5);
        entry.effect = Some(EffectSpec {
            effect_id: Uuid::new_v4(),
            curve: Curve::Shape(Shape::Sine),
            rate: Rate::Hz(1.0),
            low: ParameterValue::Float(0.0),
            high: ParameterValue::Float(1.0),
            width: 0.5,
            direction: Direction::Forward,
            phase: 0.0,
            spread: Spread::Even,
            t0: None,
        });
        h.engine
            .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
            .await
            .unwrap();

        let (path, args) = programmer_by(fixture.id, ParameterKind::Intensity, 0.1);
        let err = h.engine.set(path, Lifecycle::Synced, args).await.expect_err("refused");
        assert!(format!("{err}").contains("shape"), "{err}");

        let row = h.engine.get(held(&h, fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert!(!row["effect"].is_null(), "the effect is still running: {row}");
    }

    #[tokio::test]
    async fn the_shapes_that_mean_nothing_say_so() {
        let h = harness().await;
        let cue = a_cue("Act 1", 1.0);
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
        let before = oplog::len(&h.pool).await.unwrap();

        // A whole row.
        let err = h
            .engine
            .set(by_path(entity_path("cues", cue.id)), Lifecycle::Persisted, json!(1))
            .await
            .expect_err("a row is not a number");
        assert!(format!("{err}").contains("one field"), "{err}");

        // A create.
        let err = h
            .engine
            .set(by_path(create_path("cues")), Lifecycle::Persisted, json!(1))
            .await
            .expect_err("there is nothing to be relative to");
        assert!(format!("{err}").contains("one field"), "{err}");

        // A field that is not there.
        let err = h
            .engine
            .set(by_path(field_path("cues", cue.id, "nonesuch")), Lifecycle::Persisted, json!(1))
            .await
            .expect_err("no such field");
        assert!(format!("{err}").contains("path not found"), "{err}");

        // A field that is not a number.
        let err = h
            .engine
            .set(by_path(field_path("cues", cue.id, "name")), Lifecycle::Persisted, json!(1))
            .await
            .expect_err("a name cannot be nudged");
        assert!(format!("{err}").contains("not a number"), "{err}");

        assert_eq!(oplog::len(&h.pool).await.unwrap(), before, "and none of it was written");
    }

    /// The property the whole design rests on: past the front door there is no such
    /// thing as a relative write.
    #[tokio::test]
    async fn what_is_logged_is_absolute() {
        let h = harness().await;
        let user = Uuid::new_v4();
        let cue = a_cue("Act 1", 1.0);
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();

        h.engine
            .set_as(
                user,
                None,
                by_path(field_path("cues", cue.id, "fade_in_ms")),
                Lifecycle::Persisted,
                json!(1500),
            )
            .await
            .unwrap();

        let log = oplog::recent_by_people(&h.pool, 100).await.unwrap();
        let op = log
            .iter()
            .find(|op| op.user_id == Some(user))
            .expect("the nudge is in the history");
        assert!(
            !op.path.iter().any(|s| matches!(s, PathSegment::Key(k) if k == "__by")),
            "the log holds a destination, not a delta: {:?}",
            op.path
        );
        assert_eq!(op.value, json!(4500), "and the destination is the resolved number");
        assert_eq!(op.previous, Some(json!(3000)), "with where it came from");
    }

    #[tokio::test]
    async fn undo_puts_back_what_was_there() {
        let h = harness().await;
        let user = Uuid::new_v4();
        let cue = a_cue("Act 1", 1.0);
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();

        h.engine
            .set_as(
                user,
                None,
                by_path(field_path("cues", cue.id, "fade_in_ms")),
                Lifecycle::Persisted,
                json!(1500),
            )
            .await
            .unwrap();

        h.engine.undo(user, false).await;
        assert_eq!(
            h.engine.get(field_path("cues", cue.id, "fade_in_ms")).await.unwrap(),
            json!(3000),
            "back where it started, not 1500 less than it ended"
        );

        h.engine.undo(user, true).await;
        assert_eq!(
            h.engine.get(field_path("cues", cue.id, "fade_in_ms")).await.unwrap(),
            json!(4500),
            "and redo does not apply the delta a second time"
        );
    }

    /// The resolution step names one collection, `programmer_values`, and this is the
    /// property that must not cost: a collection it has never heard of — this one was
    /// added a task ago — is nudged by the same verb with no edit to it.
    #[tokio::test]
    async fn a_collection_the_resolver_has_never_heard_of() {
        use pult_schema::types::SpeedMaster;

        let h = harness().await;
        let master = SpeedMaster {
            id: Uuid::new_v4(),
            name: "Chases".into(),
            bpm: 120.0,
            multiplier: 1.0,
            running: true,
            t0: 0,
        };
        h.engine
            .set(create_path("speed_masters"), Lifecycle::Persisted, json(&master))
            .await
            .unwrap();

        h.engine
            .set(
                by_path(field_path("speed_masters", master.id, "bpm")),
                Lifecycle::Persisted,
                json!(8),
            )
            .await
            .unwrap();

        let bpm = h.engine.get(field_path("speed_masters", master.id, "bpm")).await.unwrap();
        assert_eq!(bpm.as_f64().unwrap(), 128.0);
    }

    /// Two stations, one of them showing something else entirely. The one that made
    /// the nudge decided the number; the other takes it. If the delta travelled, they
    /// would part company on the first press.
    #[tokio::test]
    async fn a_peer_showing_something_else_still_lands_on_the_same_number() {
        use pult_schema::events::operation::{Operation, VectorClock};

        let here = harness().await;
        let there = harness().await;
        let cue = a_cue("Act 1", 1.0);
        here.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();

        // The other station has the cue with a different fade — mid-edit, or behind.
        let mut theirs = cue.clone();
        theirs.fade_in_ms = 500;
        there.engine.set(create_path("cues"), Lifecycle::Persisted, json(&theirs)).await.unwrap();

        here.engine
            .set(
                by_path(field_path("cues", cue.id, "fade_in_ms")),
                Lifecycle::Persisted,
                json!(1500),
            )
            .await
            .unwrap();

        // What a peer gets is the operation the log holds.
        let ops = oplog::since(&here.pool, &VectorClock::default()).await.unwrap();
        let op: &Operation = ops
            .iter()
            .find(|op| {
                op.path.iter().any(|s| matches!(s, PathSegment::Key(k) if k == "fade_in_ms"))
            })
            .expect("the nudge is in the log");
        there.engine.0.send(EngineCommand::ApplyPeerOperation(op.clone())).await.unwrap();

        assert_eq!(
            there.engine.get(field_path("cues", cue.id, "fade_in_ms")).await.unwrap(),
            json!(4500),
            "the peer takes the number, not 1500 more than its own 500"
        );
    }

    /// A peer receives the number. It has to: it may be showing something else, and
    /// two stations each adding ten percent to their own value diverge on the first
    /// nudge.
    #[tokio::test]
    async fn a_peer_receives_the_number_rather_than_the_delta() {
        let h = harness().await;
        let cue = a_cue("Act 1", 1.0);
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();

        let mut updates = h
            .engine
            .subscribe_pattern(PathPattern::new(&format!("cues/{}/fade_in_ms", cue.id)))
            .await;
        h.engine
            .set(
                by_path(field_path("cues", cue.id, "fade_in_ms")),
                Lifecycle::Persisted,
                json!(1500),
            )
            .await
            .unwrap();

        assert_eq!(
            updates.next().await.unwrap(),
            json!(4500),
            "what goes out on the wire is the resolved value"
        );
    }
}


// ── Going home ────────────────────────────────────────────────────────────────

/// Where a parameter rests when nothing is driving it.
///
/// The same property as a relative write, for the same reason: `__home` stops
/// existing at the front door. What the caller sends is a fixture; what the log and
/// every peer see is a value, resolved here — because a peer resolving "home" against
/// its own copy is a peer that could resolve it differently.
mod home {
    use pult_schema::types::fixture::{
        FixtureType, ParameterBinding, ParameterDefinition, ParameterKind,
        ParameterValue,
    };
    use pult_schema::types::programmer::{programmer_entry_id, ProgrammerValue};
    use serde_json::json;

    use super::*;
    use crate::infra::showfile::oplog;

    fn home_path() -> Path {
        vec![
            PathSegment::Key("programmer_values".into()),
            PathSegment::Key("__home".into()),
        ]
    }

    fn a_parameter(kind: ParameterKind, default: ParameterValue) -> ParameterDefinition {
        ParameterDefinition::new(kind, default)
    }

    /// A fixture of a type that actually exists, which is what home needs and what
    /// `a_fixture` on its own does not give: its type id points at nothing.
    async fn a_patched_fixture(h: &Harness, parameters: Vec<ParameterDefinition>) -> Fixture {
        let ft = FixtureType {
            id: Uuid::new_v4(),
            name: "Source Four".into(),
            manufacturer: "ETC".into(),
            channel_count: 1,
            parameters,
            ..FixtureType::default()
        };
        h.engine.set(create_path("fixture_types"), Lifecycle::Persisted, json(&ft)).await.unwrap();

        let mut fixture = a_fixture("Spot", 1);
        fixture.fixture_type_id = ft.id;
        h.engine
            .set(create_path("fixtures"), Lifecycle::Persisted, json(&fixture))
            .await
            .unwrap();
        fixture
    }

    fn held(fixture_id: Uuid, kind: &ParameterKind) -> Path {
        let key = pult_schema::types::fixture::parameter_key(kind);
        vec![
            PathSegment::Key("programmer_values".into()),
            PathSegment::Id(programmer_entry_id(&fixture_id.to_string(), &key).parse().unwrap()),
        ]
    }

    fn a_programmer_value(fixture_id: Uuid, kind: ParameterKind, value: f32) -> ProgrammerValue {
        let key = pult_schema::types::fixture::parameter_key(&kind);
        ProgrammerValue {
            id: programmer_entry_id(&fixture_id.to_string(), &key).parse().unwrap(),
            fixture_id,
            parameter_kind: kind,
            value: ParameterValue::Float(value),
            effect: None,
            locked: false,
        }
    }

    #[tokio::test]
    async fn a_named_parameter_is_held_at_what_its_type_declares() {
        let h = harness().await;
        let fixture =
            a_patched_fixture(&h, vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.2))])
                .await;

        h.engine
            .set(
                home_path(),
                Lifecycle::Synced,
                json!({ "fixtureId": fixture.id, "parameterKind": ParameterKind::Intensity }),
            )
            .await
            .unwrap();

        let row = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert_eq!(row["value"], json(&ParameterValue::Float(0.2)));
        assert_eq!(row["locked"], json!(false));
    }

    /// The case the override exists for: a house light is on when nothing is
    /// controlling it, and the type — derived from what the node said — cannot know.
    #[tokio::test]
    async fn a_fixtures_own_override_wins_over_its_type() {
        let h = harness().await;
        let mut fixture =
            a_patched_fixture(&h, vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))])
                .await;
        fixture.home_values.insert("Intensity".into(), ParameterValue::Float(1.0));
        h.engine
            .set(
                field_path("fixtures", fixture.id, "home_values"),
                Lifecycle::Persisted,
                json(&fixture.home_values),
            )
            .await
            .unwrap();

        h.engine
            .set(home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        let row = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert_eq!(row["value"], json(&ParameterValue::Float(1.0)), "on, not dark");
    }

    /// No `parameterKind`, so the station enumerates. Which is the point: a caller
    /// that can ask for home does not have to be able to read what a fixture has.
    #[tokio::test]
    async fn a_whole_fixture_goes_home_without_the_caller_naming_anything() {
        let h = harness().await;
        let fixture = a_patched_fixture(
            &h,
            vec![
                a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0)),
                a_parameter(ParameterKind::Pan, ParameterValue::Float(0.5)),
                ParameterDefinition {
                    direction: ParameterDirection::Input,
                    binding: Some(ParameterBinding::Port { index: 0 }),
                    ..ParameterDefinition::new(
                        ParameterKind::Contact(0),
                        ParameterValue::Bool(false),
                    )
                },
            ],
        )
        .await;

        h.engine
            .set(home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        let rows = h.engine.get(key("programmer_values")).await.unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 2, "the two outputs, and not the contact: {rows:?}");
        let pan = h.engine.get(held(fixture.id, &ParameterKind::Pan)).await.unwrap();
        assert_eq!(pan["value"], json(&ParameterValue::Float(0.5)));
    }

    #[tokio::test]
    async fn a_parameter_already_held_is_moved_to_home_rather_than_doubled() {
        let h = harness().await;
        let fixture =
            a_patched_fixture(&h, vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))])
                .await;
        let entry = a_programmer_value(fixture.id, ParameterKind::Intensity, 0.9);
        h.engine
            .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
            .await
            .unwrap();

        h.engine
            .set(home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        let rows = h.engine.get(key("programmer_values")).await.unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1, "one row, because the id is derived");
        let row = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert_eq!(row["value"], json(&ParameterValue::Float(0.0)));
    }

    /// Parking is exactly the ask that a value survive being taken away, and home
    /// takes values away. Swept up with a whole fixture it is left alone; asked for
    /// by name it says so, because being ignored is worse than being refused.
    #[tokio::test]
    async fn a_parked_value_is_left_where_it_was_parked() {
        let h = harness().await;
        let fixture =
            a_patched_fixture(&h, vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))])
                .await;
        let mut entry = a_programmer_value(fixture.id, ParameterKind::Intensity, 0.9);
        entry.locked = true;
        h.engine
            .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
            .await
            .unwrap();

        h.engine
            .set(home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();
        let row = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert_eq!(row["value"], json(&ParameterValue::Float(0.9)), "still parked at 0.9");

        let err = h
            .engine
            .set(
                home_path(),
                Lifecycle::Synced,
                json!({ "fixtureId": fixture.id, "parameterKind": ParameterKind::Intensity }),
            )
            .await
            .expect_err("named on its own, it is refused");
        assert!(format!("{err}").contains("parked"), "{err}");
    }

    #[tokio::test]
    async fn an_input_has_nothing_to_send_home() {
        let h = harness().await;
        let fixture = a_patched_fixture(
            &h,
            vec![ParameterDefinition {
                direction: ParameterDirection::Input,
                binding: Some(ParameterBinding::Port { index: 0 }),
                ..ParameterDefinition::new(ParameterKind::Contact(0), ParameterValue::Bool(false))
            }],
        )
        .await;

        let err = h
            .engine
            .set(
                home_path(),
                Lifecycle::Synced,
                json!({ "fixtureId": fixture.id, "parameterKind": ParameterKind::Contact(0) }),
            )
            .await
            .expect_err("nothing to send home");
        assert!(format!("{err}").contains("the device writes"), "{err}");
        assert!(
            h.engine.get(key("programmer_values")).await.unwrap().as_array().unwrap().is_empty(),
            "and nothing was written"
        );
    }

    #[tokio::test]
    async fn the_shapes_that_mean_nothing_say_so() {
        let h = harness().await;
        let cue = a_cue("Act 1", 1.0);
        h.engine.set(create_path("cues"), Lifecycle::Persisted, json(&cue)).await.unwrap();
        let before = oplog::len(&h.pool).await.unwrap();

        let err = h
            .engine
            .set(
                vec![
                    PathSegment::Key("cues".into()),
                    PathSegment::Id(cue.id),
                    PathSegment::Key("fade_in_ms".into()),
                    PathSegment::Key("__home".into()),
                ],
                Lifecycle::Persisted,
                json!({}),
            )
            .await
            .expect_err("a cue does not rest anywhere");
        assert!(format!("{err}").contains("rest"), "{err}");

        let err = h
            .engine
            .set(home_path(), Lifecycle::Synced, json!({ "fixtureId": Uuid::new_v4() }))
            .await
            .expect_err("no such fixture");
        assert!(format!("{err}").contains("patched"), "{err}");

        assert_eq!(oplog::len(&h.pool).await.unwrap(), before, "and none of it was written");
    }

    /// The property the design rests on: past the front door there is no such thing
    /// as a home write. A peer receives the value this station resolved.
    #[tokio::test]
    async fn what_is_logged_is_the_value() {
        let h = harness().await;
        let user = Uuid::new_v4();
        let fixture =
            a_patched_fixture(&h, vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.2))])
                .await;

        h.engine
            .set_as(user, None, home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        let log = oplog::recent_by_people(&h.pool, 100).await.unwrap();
        let op = log
            .iter()
            .find(|op| op.user_id == Some(user))
            .expect("the home is in the history");
        assert!(
            !op.path.iter().any(|s| matches!(s, PathSegment::Key(k) if k == "__home")),
            "the log holds a value, not a verb: {:?}",
            op.path
        );
        assert!(
            format!("{}", op.value).contains("0.2"),
            "and the value is the one this station resolved: {}",
            op.value
        );
    }

    /// The other reader of the same resolution. A nudge on a parameter nothing has
    /// ever driven starts from where it rests — which since this change means the
    /// fixture's own override, not only what its type declares.
    #[tokio::test]
    async fn a_nudge_on_an_undriven_parameter_starts_from_the_override() {
        let h = harness().await;
        let mut fixture =
            a_patched_fixture(&h, vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))])
                .await;
        fixture.home_values.insert("Intensity".into(), ParameterValue::Float(0.4));
        h.engine
            .set(
                field_path("fixtures", fixture.id, "home_values"),
                Lifecycle::Persisted,
                json(&fixture.home_values),
            )
            .await
            .unwrap();

        h.engine
            .set(
                vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Key("__by".into()),
                ],
                Lifecycle::Synced,
                json!({
                    "fixtureId": fixture.id,
                    "parameterKind": ParameterKind::Intensity,
                    "by": 0.1,
                }),
            )
            .await
            .unwrap();

        let row = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        let level = row["value"]["value"].as_f64().unwrap();
        assert!((level - 0.5).abs() < 1e-5, "0.4 and a tenth more, not 0.1: {row}");
    }

    /// One act, one Ctrl-Z. A fixture with four parameters is four writes, and an
    /// operator who asked for one thing should not press undo four times.
    #[tokio::test]
    async fn undoing_a_home_takes_back_the_whole_fixture() {
        let h = harness().await;
        let user = Uuid::new_v4();
        let fixture = a_patched_fixture(
            &h,
            vec![
                a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0)),
                a_parameter(ParameterKind::Pan, ParameterValue::Float(0.5)),
            ],
        )
        .await;
        for (kind, value) in
            [(ParameterKind::Intensity, 0.9f32), (ParameterKind::Pan, 0.1)]
        {
            let entry = a_programmer_value(fixture.id, kind, value);
            h.engine
                .set(create_path("programmer_values"), Lifecycle::Synced, json(&entry))
                .await
                .unwrap();
        }

        h.engine
            .set_as(user, None, home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        h.engine.undo(user, false).await;

        let intensity = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        let pan = h.engine.get(held(fixture.id, &ParameterKind::Pan)).await.unwrap();
        assert_eq!(intensity["value"], json(&ParameterValue::Float(0.9)));
        assert_eq!(pan["value"], json(&ParameterValue::Float(0.1)), "both, from one undo");
    }

    // ── …and the other way ────────────────────────────────────────────────────

    fn set_home_path() -> Path {
        vec![
            PathSegment::Key("fixtures".into()),
            PathSegment::Key("__set_home".into()),
        ]
    }

    /// Put the station where it is driving these parameters at these levels.
    ///
    /// Landed fades, because that is what "driving" is now: a description anchored in
    /// time, which happens to be constant. Nothing stores the number, so seeding a map
    /// of numbers would be seeding a state the console never has.
    async fn put_out(h: &Harness, fixture: &Fixture, values: &[(&str, f32)]) {
        let driving: std::collections::HashMap<String, RunningFade> = values
            .iter()
            .map(|(key, v)| {
                (
                    key.to_string(),
                    RunningFade {
                        from: ParameterValue::Float(*v),
                        to: ParameterValue::Float(*v),
                        t0: 0,
                        duration_ms: 0,
                        easing: Easing::Step,
                        cue_id: Uuid::nil(),
                    },
                )
            })
            .collect();
        h.engine
            .set(field_path("fixtures", fixture.id, "live_fades"), Lifecycle::Local, json(&driving))
            .await
            .unwrap();
    }

    /// The act a house light exists for: aim it, look at it, keep it. What is stored
    /// is what the station is putting out, which the caller never had to read.
    #[tokio::test]
    async fn taking_the_output_makes_it_the_resting_place() {
        let h = harness().await;
        let fixture = a_patched_fixture(
            &h,
            vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        )
        .await;
        put_out(&h, &fixture, &[("Intensity", 0.65)]).await;

        h.engine
            .set(set_home_path(), Lifecycle::Persisted, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        let home = h.engine.get(field_path("fixtures", fixture.id, "home_values")).await.unwrap();
        assert_eq!(home["Intensity"], json(&ParameterValue::Float(0.65)));

        // And it is now what home resolves to, which is the whole point of storing it.
        h.engine
            .set(home_path(), Lifecycle::Synced, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();
        let row = h.engine.get(held(fixture.id, &ParameterKind::Intensity)).await.unwrap();
        assert_eq!(row["value"], json(&ParameterValue::Float(0.65)));
    }

    /// Only what was named, and everything else left as it was — an operator fixing
    /// one parameter must not have the rest of the fixture written underneath them.
    #[tokio::test]
    async fn naming_a_parameter_takes_only_that_one() {
        let h = harness().await;
        let mut fixture = a_patched_fixture(
            &h,
            vec![
                a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0)),
                a_parameter(ParameterKind::Pan, ParameterValue::Float(0.5)),
            ],
        )
        .await;
        fixture.home_values.insert("Pan".into(), ParameterValue::Float(0.25));
        h.engine
            .set(
                field_path("fixtures", fixture.id, "home_values"),
                Lifecycle::Persisted,
                json(&fixture.home_values),
            )
            .await
            .unwrap();
        put_out(&h, &fixture, &[("Intensity", 0.8), ("Pan", 0.9)]).await;

        h.engine
            .set(
                set_home_path(),
                Lifecycle::Persisted,
                json!({ "fixtureId": fixture.id, "parameterKind": ParameterKind::Intensity }),
            )
            .await
            .unwrap();

        let home = h.engine.get(field_path("fixtures", fixture.id, "home_values")).await.unwrap();
        assert_eq!(home["Intensity"], json(&ParameterValue::Float(0.8)));
        assert_eq!(home["Pan"], json(&ParameterValue::Float(0.25)), "not the output, untouched");
    }

    /// The same property `__by` and `__home` have, and for the same reason: what the
    /// log and every peer see is the map, never the verb. A peer resolving "whatever
    /// it is putting out" against its own copy could resolve it to something else.
    #[tokio::test]
    async fn what_is_recorded_is_the_map_and_not_the_verb() {
        let h = harness().await;
        let user = Uuid::new_v4();
        let fixture = a_patched_fixture(
            &h,
            vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        )
        .await;
        put_out(&h, &fixture, &[("Intensity", 0.3)]).await;

        h.engine
            .set_as(
                user,
                None,
                set_home_path(),
                Lifecycle::Persisted,
                json!({ "fixtureId": fixture.id }),
            )
            .await
            .unwrap();

        let log = oplog::recent_by_people(&h.pool, 10).await.unwrap();
        let op = log.first().expect("somebody's change was written down");
        assert!(
            !op.path.iter().any(|s| matches!(s, PathSegment::Key(k) if k == "__set_home")),
            "the verb stops at the front door: {:?}",
            op.path
        );
        assert_eq!(op.value["Intensity"], json(&ParameterValue::Float(0.3)));
    }

    /// Mid-fade, what is taken is where the parameter is at the moment it is asked
    /// about — not where the fade started and not where it is going.
    #[tokio::test]
    async fn taking_the_output_mid_fade_takes_the_moment_it_was_asked() {
        let h = harness().await;
        let fixture = a_patched_fixture(
            &h,
            vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        )
        .await;

        // A two second fade from dark to full, half a second old.
        let now = pult_schema::types::sequence::now_ms();
        h.engine
            .set(
                field_path("fixtures", fixture.id, "live_fades"),
                Lifecycle::Local,
                serde_json::json!({
                    "Intensity": {
                        "from": ParameterValue::Float(0.0),
                        "to": ParameterValue::Float(1.0),
                        "t0": now - 500,
                        "duration_ms": 2_000,
                        "easing": "Linear",
                        "cue_id": Uuid::nil(),
                    }
                }),
            )
            .await
            .unwrap();

        h.engine
            .set(set_home_path(), Lifecycle::Persisted, json!({ "fixtureId": fixture.id }))
            .await
            .unwrap();

        let home = h.engine.get(field_path("fixtures", fixture.id, "home_values")).await.unwrap();
        let taken = home["Intensity"]["value"].as_f64().unwrap() as f32;
        assert!(
            taken > 0.15 && taken < 0.6,
            "a quarter of the way up, give or take how long the call took: {taken}",
        );
    }

    /// A fixture nothing has ever driven has nothing to take. Not an error when it
    /// was the whole fixture that was asked for — but a named parameter is one an
    /// operator is looking at, and silence there reads as "done".
    #[tokio::test]
    async fn a_parameter_putting_nothing_out_is_not_taken() {
        let h = harness().await;
        let fixture = a_patched_fixture(
            &h,
            vec![a_parameter(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        )
        .await;

        h.engine
            .set(set_home_path(), Lifecycle::Persisted, json!({ "fixtureId": fixture.id }))
            .await
            .expect("nothing on stage is an ordinary state, not a failure");
        let home = h.engine.get(field_path("fixtures", fixture.id, "home_values")).await.unwrap();
        assert_eq!(home, json!({}), "and nothing was written");

        let named = h
            .engine
            .set(
                set_home_path(),
                Lifecycle::Persisted,
                json!({ "fixtureId": fixture.id, "parameterKind": ParameterKind::Intensity }),
            )
            .await;
        assert!(named.is_err(), "asked about one parameter, told about that parameter");
    }

    /// An input is a parameter the device writes and the show reads. There is nothing
    /// to send home and nothing to take from, and both refusals say so.
    #[tokio::test]
    async fn an_input_has_no_resting_place_to_take() {
        let h = harness().await;
        let mut input = a_parameter(ParameterKind::Contact(1), ParameterValue::Bool(false));
        input.direction = ParameterDirection::Input;
        let fixture = a_patched_fixture(&h, vec![input]).await;
        put_out(&h, &fixture, &[("Contact:1", 1.0)]).await;

        let refused = h
            .engine
            .set(
                set_home_path(),
                Lifecycle::Persisted,
                json!({ "fixtureId": fixture.id, "parameterKind": ParameterKind::Contact(1) }),
            )
            .await;
        assert!(refused.is_err(), "{refused:?}");
    }

    /// The verb means something on a fixture and nowhere else, and says so rather
    /// than doing something almost-right — the same answer `__home` gives.
    #[tokio::test]
    async fn the_verb_is_refused_where_nothing_rests() {
        let h = harness().await;
        let refused = h
            .engine
            .set(
                vec![
                    PathSegment::Key("cues".into()),
                    PathSegment::Key("__set_home".into()),
                ],
                Lifecycle::Persisted,
                json!({}),
            )
            .await;
        assert!(refused.is_err(), "{refused:?}");
    }
}

/// Who a station asks for an asset it does not have.
mod peers {
    use super::*;

    fn a_station_row(id: Uuid, http_addr: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "hostname": "console",
            "is_leader": false,
            "sync_addr": "127.0.0.1:1",
            "http_addr": http_addr,
            "cpu_percent": 0.0, "mem_used": 0, "mem_total": 0, "uptime_s": 0,
            "output_plugins": [], "computes_fixtures": 0, "total_fixtures": 0,
            "last_seen": Utc::now().to_rfc3339(),
        })
    }

    /// Every station publishes its own row, so "ask the other stations" has to say
    /// so out loud. A station asking itself already knows the answer — not having
    /// the asset locally is what started the search — and the round trip taught it
    /// nothing, twice over once a fetch learned to retry.
    #[tokio::test]
    async fn a_station_does_not_ask_itself() {
        let h = harness().await;
        let me = Uuid::new_v4();
        let you = Uuid::new_v4();
        for (id, addr) in [(me, "127.0.0.1:7700"), (you, "127.0.0.1:7710")] {
            h.engine
                .set(create_path("stations"), Lifecycle::Synced, a_station_row(id, addr))
                .await
                .unwrap();
        }

        let peers = crate::infra::assets::peer_addresses(&h.engine, me).await;
        assert_eq!(peers, vec!["127.0.0.1:7710".to_string()], "only the other one");
    }

    /// A station that has not said where it serves HTTP cannot be asked, and an
    /// empty address would be a request to `http:///assets/…`.
    #[tokio::test]
    async fn a_station_with_no_address_is_not_asked() {
        let h = harness().await;
        h.engine
            .set(
                create_path("stations"),
                Lifecycle::Synced,
                a_station_row(Uuid::new_v4(), ""),
            )
            .await
            .unwrap();

        assert!(crate::infra::assets::peer_addresses(&h.engine, Uuid::new_v4()).await.is_empty());
    }
}

// ── What a frame costs ────────────────────────────────────────────────────────
//
// Measured where the frame is drawn — in the output manager, per connector — and
// published on the station row from there. The engine has no tick to measure any
// more: a pass happens when the show changes, and a fade in progress is not a pass.
// `infra::connectors` and `infra::stations` hold what is left of these.

