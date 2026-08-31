use pult_schema::path::PathSegment;

use super::*;
use crate::infra::showfile;

fn an_op(node: NodeId, seq: u64, field: &str, value: &str) -> Operation {
    let mut clock = VectorClock::default();
    for _ in 0..seq {
        clock.increment(node);
    }
    Operation {
        id: Uuid::new_v4(),
        node_id: node,
        seq,
        clock,
        lifecycle: Lifecycle::Persisted,
        path: vec![PathSegment::Key("sequences".into()), PathSegment::Key(field.into())],
        value: serde_json::json!(value),
        timestamp: Utc::now(),
        user_id: None,
        previous: None,
        undoes: None,
        gesture: None,
    }
}

#[tokio::test]
async fn an_appended_operation_comes_back() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    append(&pool, &an_op(node, 1, "name", "Act 1")).await.unwrap();

    let all = since(&pool, &VectorClock::default()).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].value, "Act 1");
    assert_eq!(all[0].node_id, node);
}

#[tokio::test]
async fn a_node_is_only_told_what_it_has_not_seen() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    for seq in 1..=5 {
        append(&pool, &an_op(node, seq, "name", &format!("v{seq}"))).await.unwrap();
    }

    let mut known = VectorClock::default();
    for _ in 0..3 {
        known.increment(node);
    }
    let missing = since(&pool, &known).await.unwrap();

    assert_eq!(missing.len(), 2);
    assert_eq!(missing[0].value, "v4");
    assert_eq!(missing[1].value, "v5");
}

#[tokio::test]
async fn a_node_that_is_up_to_date_is_told_nothing() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    append(&pool, &an_op(node, 1, "name", "Act 1")).await.unwrap();

    let mut known = VectorClock::default();
    known.increment(node);
    assert!(since(&pool, &known).await.unwrap().is_empty());
}

#[tokio::test]
async fn each_node_is_tracked_separately() {
    let pool = showfile::open_in_memory().await.unwrap();
    let one = NodeId(Uuid::from_u128(1));
    let two = NodeId(Uuid::from_u128(2));
    append(&pool, &an_op(one, 1, "name", "from one")).await.unwrap();
    append(&pool, &an_op(two, 1, "name", "from two")).await.unwrap();

    // Caught up with one, never heard from two.
    let mut known = VectorClock::default();
    known.increment(one);
    let missing = since(&pool, &known).await.unwrap();

    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].value, "from two");
}

#[tokio::test]
async fn appending_the_same_operation_twice_does_not_duplicate_it() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let op = an_op(node, 1, "name", "Act 1");

    append(&pool, &op).await.unwrap();
    append(&pool, &op).await.unwrap();

    assert_eq!(len(&pool).await.unwrap(), 1);
}

#[tokio::test]
async fn the_log_reports_its_size() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    assert_eq!(len(&pool).await.unwrap(), 0);
    for seq in 1..=3 {
        append(&pool, &an_op(node, seq, "name", "x")).await.unwrap();
    }
    assert_eq!(len(&pool).await.unwrap(), 3);
}

#[tokio::test]
async fn operations_come_back_oldest_first() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    for seq in 1..=4 {
        append(&pool, &an_op(node, seq, "name", &format!("v{seq}"))).await.unwrap();
    }

    let all = since(&pool, &VectorClock::default()).await.unwrap();
    let order: Vec<u64> = all.iter().map(|o| o.seq).collect();
    assert_eq!(order, vec![1, 2, 3, 4], "replaying out of order would undo later writes");
}

/// The station writes its own telemetry into the log twice a second. If those rows
/// counted against the window, a change made twenty minutes ago would have fallen out
/// of reach of Ctrl-Z while its author was still thinking about it.
#[tokio::test]
async fn the_stations_own_writes_are_not_part_of_anybodys_history() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let sam = Uuid::new_v4();

    let mut theirs = an_op(node, 1, "name", "Act 1");
    theirs.user_id = Some(sam);
    theirs.previous = Some(serde_json::json!("Untitled"));
    append(&pool, &theirs).await.unwrap();
    for seq in 2..=20 {
        append(&pool, &an_op(node, seq, "cpu", "busy")).await.unwrap();
    }

    let log = recent_by_people(&pool, 500).await.unwrap();
    assert_eq!(log.len(), 1, "only what somebody asked for");
    assert_eq!(log[0].user_id, Some(sam));
}

/// Newest first, because undo wants the most recent qualifying operation and the
/// history panel reads top down.
#[tokio::test]
async fn a_persons_changes_come_back_newest_first() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let sam = Uuid::new_v4();

    for seq in 1..=3 {
        let mut op = an_op(node, seq, "name", &format!("v{seq}"));
        op.user_id = Some(sam);
        op.timestamp = Utc::now() + chrono::Duration::seconds(seq as i64);
        append(&pool, &op).await.unwrap();
    }

    let log = recent_by_people(&pool, 2).await.unwrap();
    assert_eq!(log.len(), 2, "the limit is honoured");
    assert_eq!(log[0].value, "v3");
    assert_eq!(log[1].value, "v2");
}

/// The gesture has to survive the round trip or a drag comes back as four hundred
/// separate changes on the next station to read the log.
#[tokio::test]
async fn a_gesture_survives_the_showfile() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let sam = Uuid::new_v4();
    let drag = Uuid::new_v4();

    // Three fixtures under one drag — different paths, so nothing folds and the
    // column is what is being tested rather than the folding.
    for seq in 1..=3 {
        let mut written = an_op(node, seq, &format!("fixture{seq}"), "aimed");
        written.user_id = Some(sam);
        written.previous = Some(serde_json::json!("before"));
        written.gesture = Some(drag);
        append(&pool, &written).await.unwrap();
    }

    let log = recent_by_people(&pool, 500).await.unwrap();
    assert_eq!(log.len(), 3);
    assert!(log.iter().all(|op| op.gesture == Some(drag)), "all one act");
}

// ── Folding a drag into one row ───────────────────────────────────────────────

/// A drag writes the same path forty times a second and the log needs the last of
/// them. Without this a two-second drag across twenty fixtures is 2,400 rows.
#[tokio::test]
async fn a_gesture_keeps_one_row_per_path() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let sam = Uuid::new_v4();
    let drag = Uuid::new_v4();

    for seq in 1..=50 {
        let mut written = an_op(node, seq, "name", &format!("v{seq}"));
        written.user_id = Some(sam);
        written.previous = Some(serde_json::json!("before the drag"));
        written.gesture = Some(drag);
        append(&pool, &written).await.unwrap();
    }

    assert_eq!(len(&pool).await.unwrap(), 1, "fifty writes, one row");
    let log = recent_by_people(&pool, 500).await.unwrap();
    assert_eq!(log[0].value, serde_json::json!("v50"), "where the drag ended");
    assert_eq!(
        log[0].previous,
        Some(serde_json::json!("before the drag")),
        "and where it started, which is what undo needs"
    );
}

/// The part that makes folding safe rather than merely smaller. A peer asks for
/// everything past a sequence number, so a row that kept its first `seq` would be
/// invisible to one that had already caught up mid-drag.
#[tokio::test]
async fn a_folded_row_moves_to_the_front_of_the_queue() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let drag = Uuid::new_v4();

    for seq in 1..=5 {
        let mut written = an_op(node, seq, "name", &format!("v{seq}"));
        written.user_id = Some(Uuid::new_v4());
        written.previous = Some(serde_json::json!(""));
        written.gesture = Some(drag);
        written.timestamp = Utc::now() + chrono::Duration::milliseconds(seq as i64);
        append(&pool, &written).await.unwrap();
    }

    // A peer that had seen the first three writes still learns where the drag ended.
    let mut known = VectorClock::default();
    for _ in 0..3 {
        known.increment(node);
    }
    let missing = since(&pool, &known).await.unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].value, "v5");
}

/// Two separate edits to the same path are two things somebody did. Folding them
/// would swallow the first, and there would be nothing to take the second back to.
#[tokio::test]
async fn two_gestures_on_one_path_stay_two_rows() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let sam = Uuid::new_v4();

    for (seq, gesture) in [(1, Uuid::new_v4()), (2, Uuid::new_v4())] {
        let mut written = an_op(node, seq, "name", &format!("v{seq}"));
        written.user_id = Some(sam);
        written.previous = Some(serde_json::json!("x"));
        written.gesture = Some(gesture);
        append(&pool, &written).await.unwrap();
    }
    assert_eq!(len(&pool).await.unwrap(), 2);
}

/// A write outside a gesture folds into nothing, which is most writes.
#[tokio::test]
async fn writes_with_no_gesture_are_never_folded() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    for seq in 1..=3 {
        append(&pool, &an_op(node, seq, "name", &format!("v{seq}"))).await.unwrap();
    }
    assert_eq!(len(&pool).await.unwrap(), 3);
}

/// Every create in a collection is written to the same `<table>/__create` path, so
/// folding by path would make one gesture that patched two fixtures forget one.
#[tokio::test]
async fn two_things_made_in_one_gesture_stay_two_rows() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    let sam = Uuid::new_v4();
    let gesture = Uuid::new_v4();

    for (seq, id) in [(1, Uuid::new_v4()), (2, Uuid::new_v4())] {
        let mut written = an_op(node, seq, "name", "ignored");
        written.path = vec![
            PathSegment::Key("fixtures".into()),
            PathSegment::Key("__create".into()),
        ];
        written.value = serde_json::json!({ "id": id, "name": "Spot" });
        written.user_id = Some(sam);
        written.previous = Some(serde_json::Value::Null);
        written.gesture = Some(gesture);
        append(&pool, &written).await.unwrap();
    }
    assert_eq!(len(&pool).await.unwrap(), 2, "both fixtures are still in the log");
}
