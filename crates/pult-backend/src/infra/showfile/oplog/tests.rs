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
