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

// ── The prune floor ───────────────────────────────────────────────────────────

#[tokio::test]
async fn an_unpruned_showfile_has_no_floor() {
    let pool = showfile::open_in_memory().await.unwrap();
    assert!(floor(&pool).await.unwrap().is_empty(), "nothing has been cut");
}

#[tokio::test]
async fn a_floor_comes_back_per_node() {
    let pool = showfile::open_in_memory().await.unwrap();
    let (a, b) = (NodeId(Uuid::new_v4()), NodeId(Uuid::new_v4()));

    raise_floor(&pool, a, 10).await.unwrap();
    raise_floor(&pool, b, 3).await.unwrap();

    let mut got = floor(&pool).await.unwrap();
    got.sort_by_key(|(_, seq)| *seq);
    assert_eq!(got, vec![(b, 3), (a, 10)]);
}

#[tokio::test]
async fn raising_a_floor_twice_to_the_same_place_is_one_floor() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());

    raise_floor(&pool, node, 7).await.unwrap();
    raise_floor(&pool, node, 7).await.unwrap();

    assert_eq!(floor(&pool).await.unwrap(), vec![(node, 7)]);
}

/// The floor is a promise that nothing below it can be served. Lowering it would
/// turn a peer that needs a snapshot into one that gets a half-answer, so a lower
/// value is ignored rather than written.
#[tokio::test]
async fn a_floor_never_goes_down() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());

    raise_floor(&pool, node, 100).await.unwrap();
    raise_floor(&pool, node, 40).await.unwrap();

    assert_eq!(floor(&pool).await.unwrap(), vec![(node, 100)], "the higher one stands");

    raise_floor(&pool, node, 140).await.unwrap();
    assert_eq!(floor(&pool).await.unwrap(), vec![(node, 140)], "but it still rises");
}

/// The history read is on the path of every Ctrl-Z and every panel refresh, and the
/// retention delete walks the same rows. Both should reach them through the index
/// rather than by reading the log — which is the whole point of keeping one.
#[tokio::test]
async fn the_history_read_uses_the_index() {
    let pool = showfile::open_in_memory().await.unwrap();
    // EXPLAIN QUERY PLAN's shape is not the query's, so the description is read by
    // name rather than by position.
    let rows = sqlx::query(
        "EXPLAIN QUERY PLAN \
         SELECT seq FROM oplog WHERE user_id IS NOT NULL \
         ORDER BY timestamp DESC, seq DESC LIMIT 10",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let plan = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("detail").ok())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        plan.contains("oplog_by_people"),
        "the history read should use oplog_by_people, but the plan was: {plan}"
    );
    assert!(!plan.contains("SCAN oplog"), "and should not scan the log: {plan}");
}

/// The condition that decides catch-up against snapshot once rows can be missing.
/// Wrong in one direction it sends needless snapshots; wrong in the other it loses
/// other people's writes silently, so both directions are pinned.
mod behind_the_floor {
    use super::*;

    fn clock(entries: &[(NodeId, u64)]) -> VectorClock {
        let mut clock = VectorClock::default();
        for (node, seq) in entries {
            clock.0.insert(*node, *seq);
        }
        clock
    }

    #[test]
    fn nothing_pruned_is_never_behind() {
        let node = NodeId(Uuid::new_v4());
        assert!(!behind_the_floor(&clock(&[(node, 5)]), &[]));
    }

    #[test]
    fn a_peer_past_the_floor_can_be_caught_up() {
        let node = NodeId(Uuid::new_v4());
        assert!(!behind_the_floor(&clock(&[(node, 50)]), &[(node, 10)]));
    }

    /// The floor is the last seq deleted, so a peer that has reached it is missing
    /// nothing that is gone.
    #[test]
    fn a_peer_exactly_on_the_floor_is_not_behind_it() {
        let node = NodeId(Uuid::new_v4());
        assert!(!behind_the_floor(&clock(&[(node, 10)]), &[(node, 10)]));
    }

    #[test]
    fn a_peer_below_the_floor_is_behind_it() {
        let node = NodeId(Uuid::new_v4());
        assert!(behind_the_floor(&clock(&[(node, 9)]), &[(node, 10)]));
    }

    /// It has seen nothing from that node, and nothing is below everything.
    #[test]
    fn a_node_the_peer_has_never_heard_of_counts_as_zero() {
        let (mine, theirs) = (NodeId(Uuid::new_v4()), NodeId(Uuid::new_v4()));
        assert!(behind_the_floor(&clock(&[(mine, 100)]), &[(theirs, 1)]));
    }

    /// A floor of zero deletes nothing, so an unheard-of node does not trip it.
    #[test]
    fn a_floor_of_zero_is_not_something_to_be_behind() {
        let (mine, theirs) = (NodeId(Uuid::new_v4()), NodeId(Uuid::new_v4()));
        assert!(!behind_the_floor(&clock(&[(mine, 100)]), &[(theirs, 0)]));
    }

    /// One node's cut is enough: the peer is missing writes whoever made them.
    #[test]
    fn being_current_with_one_node_does_not_excuse_the_other() {
        let (a, b) = (NodeId(Uuid::new_v4()), NodeId(Uuid::new_v4()));
        let known = clock(&[(a, 100), (b, 2)]);
        assert!(behind_the_floor(&known, &[(a, 10), (b, 20)]));
    }
}

// ── Cutting ───────────────────────────────────────────────────────────────────

/// An op with an author and an age, for the retention tests.
fn an_old_op(node: NodeId, seq: u64, user: Option<Uuid>, minutes_ago: i64) -> Operation {
    let mut op = an_op(node, seq, "name", "v");
    op.user_id = user;
    op.previous = Some(serde_json::json!("before"));
    op.timestamp = Utc::now() - Duration::minutes(minutes_ago);
    op
}

const HOUR: Duration = Duration::minutes(60);

#[tokio::test]
async fn the_newest_authored_operations_are_kept_and_the_rest_go() {
    let pool = showfile::open_in_memory().await.unwrap();
    let (node, sam) = (NodeId(Uuid::new_v4()), Uuid::new_v4());

    // Twenty edits, newest last.
    for seq in 1..=20u64 {
        append(&pool, &an_old_op(node, seq, Some(sam), 20 - seq as i64)).await.unwrap();
    }

    prune(&pool, 5, HOUR).await.unwrap();

    let kept = recent_by_people(&pool, 100).await.unwrap();
    assert_eq!(kept.len(), 5, "exactly what the show says it keeps");
    let mut seqs: Vec<u64> = kept.iter().map(|op| op.seq).collect();
    seqs.sort();
    assert_eq!(seqs, vec![16, 17, 18, 19, 20], "and they are the newest five");
    assert!(kept.iter().all(|op| op.is_undoable()), "each still undoable");
}

/// The console's own writes are the bulk of the table — a station writes its row
/// every two seconds, forever — and nothing undoes them.
#[tokio::test]
async fn unattributed_operations_older_than_the_window_go() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());

    for seq in 1..=10u64 {
        // Half of them from before the window, half from inside it.
        append(&pool, &an_old_op(node, seq, None, if seq <= 5 { 120 } else { 5 })).await.unwrap();
    }

    prune(&pool, 500, HOUR).await.unwrap();

    assert_eq!(len(&pool).await.unwrap(), 5, "the old telemetry is gone");
    let left = since(&pool, &VectorClock::default()).await.unwrap();
    assert!(left.iter().all(|op| op.seq > 5), "and what is left is what is recent");
}

/// The two retentions are counted differently on purpose. An authored row inside the
/// depth survives however old it is, because `history_depth` counts changes rather
/// than minutes — an operator who renamed a fixture and then did something else for
/// an hour still expects Ctrl-Z to reach it.
#[tokio::test]
async fn an_old_authored_operation_is_not_cut_by_the_telemetry_window() {
    let pool = showfile::open_in_memory().await.unwrap();
    let (node, sam) = (NodeId(Uuid::new_v4()), Uuid::new_v4());

    append(&pool, &an_old_op(node, 1, Some(sam), 60 * 24)).await.unwrap();
    append(&pool, &an_old_op(node, 2, None, 60 * 24)).await.unwrap();

    prune(&pool, 500, HOUR).await.unwrap();

    let kept = recent_by_people(&pool, 100).await.unwrap();
    assert_eq!(kept.len(), 1, "a day-old edit is still somebody's change");
    assert_eq!(len(&pool).await.unwrap(), 1, "and the day-old telemetry beside it is not");
}

/// The floor is what makes the delete safe, so it has to be raised to cover
/// everything that went — and it is raised before the delete runs, so an interrupted
/// prune over-reports rather than under-reports.
#[tokio::test]
async fn pruning_raises_the_floor_over_everything_it_cut() {
    let pool = showfile::open_in_memory().await.unwrap();
    let node = NodeId(Uuid::new_v4());
    for seq in 1..=10u64 {
        append(&pool, &an_old_op(node, seq, None, if seq <= 6 { 120 } else { 1 })).await.unwrap();
    }

    prune(&pool, 500, HOUR).await.unwrap();

    let floor = floor(&pool).await.unwrap();
    assert_eq!(floor.len(), 1);
    assert_eq!(floor[0].0, node);
    assert!(floor[0].1 >= 6, "the floor covers the highest seq that went, got {}", floor[0].1);

    // What survives is above the floor, so a peer that has reached it misses nothing.
    let left = since(&pool, &VectorClock::default()).await.unwrap();
    assert!(left.iter().all(|op| op.seq > floor[0].1));
}

#[tokio::test]
async fn pruning_a_log_that_is_already_short_does_nothing() {
    let pool = showfile::open_in_memory().await.unwrap();
    let (node, sam) = (NodeId(Uuid::new_v4()), Uuid::new_v4());
    append(&pool, &an_old_op(node, 1, Some(sam), 1)).await.unwrap();
    append(&pool, &an_old_op(node, 2, None, 1)).await.unwrap();

    assert_eq!(prune(&pool, 500, HOUR).await.unwrap(), 0, "nothing to cut");
    assert_eq!(len(&pool).await.unwrap(), 2);
    assert!(floor(&pool).await.unwrap().is_empty(), "and no floor, because nothing went");
}

/// A plugin's store write is an operation like any other. An attributed one — a
/// store that declared itself undoable — is kept by `history_depth`; an unattributed
/// one by the window. Nothing in the retention knows what a plugin is.
#[tokio::test]
async fn a_plugins_writes_are_retained_by_the_same_two_rules() {
    let pool = showfile::open_in_memory().await.unwrap();
    let (node, sam) = (NodeId(Uuid::new_v4()), Uuid::new_v4());

    let datum = |seq: u64, user: Option<Uuid>, minutes: i64| {
        let mut op = an_old_op(node, seq, user, minutes);
        op.path = vec![
            PathSegment::Key("plugin_data".into()),
            PathSegment::Id(Uuid::new_v4()),
            PathSegment::Key("value".into()),
        ];
        op
    };

    append(&pool, &datum(1, None, 120)).await.unwrap(); // old, nobody's
    append(&pool, &datum(2, Some(sam), 120)).await.unwrap(); // old, somebody's
    append(&pool, &datum(3, None, 1)).await.unwrap(); // recent, nobody's

    prune(&pool, 500, HOUR).await.unwrap();

    let left = since(&pool, &VectorClock::default()).await.unwrap();
    let seqs: Vec<u64> = left.iter().map(|op| op.seq).collect();
    assert!(seqs.contains(&2), "the operator's saved macro is kept by history_depth");
    assert!(seqs.contains(&3), "and recent unattributed data by the window");
    assert!(!seqs.contains(&1), "while old unattributed data goes, like any telemetry");
}

/// What the first open of a long-running showfile costs.
///
/// The migration case: a file that has been used all season arrives past both
/// retentions, and this is the largest cut it will ever take. Ignored by default —
/// it is a measurement, not a threshold, and a number asserted here would fail on
/// somebody's slower disk without telling anybody anything true.
///
/// Run with: `cargo test -p pult-backend --lib the_first_open_of_a_long_show -- --ignored --nocapture`
#[tokio::test]
#[ignore = "a measurement, not an assertion"]
async fn the_first_open_of_a_long_show_is_measured() {
    let pool = showfile::open_in_memory().await.unwrap();
    let (node, sam) = (NodeId(Uuid::new_v4()), Uuid::new_v4());

    // A fortnight of two stations' telemetry, and a season of edits beside it.
    let telemetry = 2 * 14 * 24 * 60 * 30 / 60; // ~one row per station per 2s
    for seq in 1..=telemetry {
        append(&pool, &an_old_op(node, seq, None, 60 * 24)).await.unwrap();
    }
    for seq in telemetry + 1..=telemetry + 5_000 {
        append(&pool, &an_old_op(node, seq, Some(sam), 60 * 24)).await.unwrap();
    }
    let before = len(&pool).await.unwrap();

    let start = std::time::Instant::now();
    let cut = prune(&pool, 500, HOUR).await.unwrap();
    let took = start.elapsed();

    println!(
        "first-open prune: {before} rows in the log, {cut} cut, {} left, took {took:?}",
        len(&pool).await.unwrap()
    );
}
