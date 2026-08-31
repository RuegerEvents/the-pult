//! Sync tests.
//!
//! The handshake and convergence tests run two real engines over real TCP.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pult_schema::{
    events::operation::{NodeId, Operation, VectorClock},
    lifecycle::Lifecycle,
    path::{Path, PathSegment},
    types::sequence::Sequence,
};
use uuid::Uuid;

use crate::{
    engine::{EngineCommand, EngineHandle, ShowEngine},
    infra::showfile::{self, oplog},
};

use super::*;

// ── Harness ───────────────────────────────────────────────────────────────────

struct Node {
    id: NodeId,
    engine: EngineHandle,
    sync: SyncHandle,
    addr: SocketAddr,
    /// What this node has measured about its links, as the reporter would read it.
    sync_mgr_links: tokio::sync::watch::Receiver<pult_schema::types::station::PeerLinks>,
    /// Its showfile, for tests that are about what is in the log rather than what
    /// the engine will say about it.
    pool: Arc<sqlx::SqlitePool>,
}

/// A backend node with its own engine, showfile, and sync port.
async fn a_node() -> Node {
    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let id = NodeId(Uuid::new_v4());
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let engine = EngineHandle(tx);

    let (manager, sync, addr) =
        SyncManager::bind(id, 0, engine.clone()).await.expect("bind an ephemeral sync port");
    let sync_mgr_links = manager.peer_links();
    tokio::spawn(manager.run());

    let (show_engine, _broadcast) =
        ShowEngine::new_with_rx(id, rx, pool.clone(), Some(sync.clone()));
    tokio::spawn(show_engine.run());

    Node { id, engine, sync, addr, sync_mgr_links, pool }
}

fn seq_path(id: Uuid, field: &str) -> Path {
    vec![
        PathSegment::Key("sequences".into()),
        PathSegment::Id(id),
        PathSegment::Key(field.into()),
    ]
}

fn a_sequence(name: &str) -> Sequence {
    Sequence { id: Uuid::new_v4(), name: name.into(), cue_ids: vec![], active_cue_index: None, went_at: None }
}

async fn create(node: &Node, seq: &Sequence) {
    node.engine
        .set(
            vec![PathSegment::Key("sequences".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::to_value(seq).unwrap(),
        )
        .await
        .unwrap();
}

async fn name_of(node: &Node, id: Uuid) -> Option<String> {
    node.engine
        .get(vec![PathSegment::Key("sequences".into()), PathSegment::Id(id)])
        .await
        .ok()?["name"]
        .as_str()
        .map(str::to_owned)
}

/// Poll until `check` passes, or give up. Sync crosses tasks and a socket, so there
/// is no single await to hang the assertion on.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Send an operation to an engine as though it arrived from a peer.
async fn peer_write(node: &Node, from: NodeId, clock: VectorClock, path: Path, value: &str) {
    let op = Operation {
        id: Uuid::new_v4(),
        node_id: from,
        seq: 1,
        clock,
        lifecycle: Lifecycle::Persisted,
        path,
        value: serde_json::json!(value),
        timestamp: Utc::now(),
        user_id: None,
        previous: None,
        undoes: None,
        gesture: None,
    };
    node.engine.0.send(EngineCommand::ApplyPeerOperation(op)).await.unwrap();
    // Ordered behind the operation on the engine's queue.
    let _ = node.engine.get(vec![PathSegment::Key("show".into())]).await;
}

// ── Handshake ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_joining_node_receives_the_leader_s_state() {
    let leader = a_node().await;
    let follower = a_node().await;

    let seq = a_sequence("Act 1");
    create(&leader, &seq).await;

    follower.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    eventually("the snapshot to arrive", || async {
        name_of(&follower, seq.id).await.as_deref() == Some("Act 1")
    })
    .await;
}

#[tokio::test]
async fn each_side_learns_the_other_s_node_id() {
    let leader = a_node().await;
    let follower = a_node().await;

    // Point both at a leader that is neither of them, so a node that reported the
    // leader's id instead of its own would be caught.
    let elsewhere = NodeId(Uuid::new_v4());
    leader.sync.set_leader(elsewhere).await;
    follower.sync.set_leader(elsewhere).await;

    follower.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;
    eventually("the peers to register", || async {
        peer_count(&leader.sync).await == 1 && peer_count(&follower.sync).await == 1
    })
    .await;

    assert_eq!(peer_ids(&follower.sync).await, vec![leader.id]);
    assert_eq!(peer_ids(&leader.sync).await, vec![follower.id]);
}

/// The connecting side used to key a peer by the leader's id rather than the peer's
/// own, because HelloAck carried no id of its own. Dialling two peers in a session
/// led by a third node then wrote both into the same slot and one connection was
/// silently lost.
#[tokio::test]
async fn a_node_dialling_two_peers_keeps_both() {
    let first = a_node().await;
    let second = a_node().await;
    let dialler = a_node().await;

    let elsewhere = NodeId(Uuid::new_v4());
    first.sync.set_leader(elsewhere).await;
    second.sync.set_leader(elsewhere).await;

    let session = Uuid::new_v4();
    let show = Uuid::new_v4();
    dialler.sync.connect_peer(first.addr, session, show).await;
    dialler.sync.connect_peer(second.addr, session, show).await;

    eventually("both connections to register", || async {
        peer_count(&dialler.sync).await == 2
    })
    .await;

    let mut ids = peer_ids(&dialler.sync).await;
    ids.sort();
    let mut expected = vec![first.id, second.id];
    expected.sort();
    assert_eq!(ids, expected, "each peer must be keyed by its own id");
}

/// Two followers on one leader must both stay reachable from the leader.
#[tokio::test]
async fn a_leader_reaches_every_follower() {
    let leader = a_node().await;
    let first = a_node().await;
    let second = a_node().await;

    let session = Uuid::new_v4();
    let show = Uuid::new_v4();
    first.sync.connect_peer(leader.addr, session, show).await;
    second.sync.connect_peer(leader.addr, session, show).await;

    eventually("both followers to register", || async { peer_count(&leader.sync).await == 2 })
        .await;

    let seq = a_sequence("Act 1");
    create(&leader, &seq).await;
    eventually("both followers to see the write", || async {
        name_of(&first, seq.id).await.is_some() && name_of(&second, seq.id).await.is_some()
    })
    .await;
}

#[tokio::test]
async fn a_write_on_one_node_reaches_the_other() {
    let one = a_node().await;
    let two = a_node().await;

    let seq = a_sequence("Act 1");
    create(&one, &seq).await;
    two.sync.connect_peer(one.addr, Uuid::new_v4(), Uuid::new_v4()).await;
    eventually("the snapshot", || async { name_of(&two, seq.id).await.is_some() }).await;

    one.engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 2"))
        .await
        .unwrap();

    eventually("the rename to replicate", || async {
        name_of(&two, seq.id).await.as_deref() == Some("Act 2")
    })
    .await;
}

// ── Conflict resolution ───────────────────────────────────────────────────────

/// Two nodes writing the same field at the same time have to end up agreeing, and
/// they cannot agree by arrival order because the order differs on each of them.
#[tokio::test]
async fn concurrent_writes_to_one_field_converge() {
    let node = a_node().await;
    let seq = a_sequence("Original");
    create(&node, &seq).await;

    let low = NodeId(Uuid::from_u128(1));
    let high = NodeId(Uuid::from_u128(2));

    // Two writes neither of which knows about the other.
    let mut low_clock = VectorClock::default();
    low_clock.increment(low);
    let mut high_clock = VectorClock::default();
    high_clock.increment(high);

    peer_write(&node, low, low_clock.clone(), seq_path(seq.id, "name"), "From low").await;
    peer_write(&node, high, high_clock.clone(), seq_path(seq.id, "name"), "From high").await;
    assert_eq!(name_of(&node, seq.id).await.as_deref(), Some("From high"));

    // The same pair in the other order has to land on the same value.
    let other = a_node().await;
    create(&other, &seq).await;
    peer_write(&other, high, high_clock, seq_path(seq.id, "name"), "From high").await;
    peer_write(&other, low, low_clock, seq_path(seq.id, "name"), "From low").await;

    assert_eq!(
        name_of(&other, seq.id).await.as_deref(),
        Some("From high"),
        "arrival order must not decide which node's edit survives",
    );
}

#[tokio::test]
async fn a_write_that_already_happened_is_not_applied_again() {
    let node = a_node().await;
    let seq = a_sequence("Original");
    create(&node, &seq).await;

    let peer = NodeId(Uuid::from_u128(9));
    let mut early = VectorClock::default();
    early.increment(peer);
    let mut later = early.clone();
    later.increment(peer);

    peer_write(&node, peer, later, seq_path(seq.id, "name"), "Newer").await;
    peer_write(&node, peer, early, seq_path(seq.id, "name"), "Older").await;

    assert_eq!(
        name_of(&node, seq.id).await.as_deref(),
        Some("Newer"),
        "a delayed older write must not undo a newer one",
    );
}

#[tokio::test]
async fn a_causally_later_write_wins_regardless_of_node_id() {
    let node = a_node().await;
    let seq = a_sequence("Original");
    create(&node, &seq).await;

    let high = NodeId(Uuid::from_u128(2));
    let low = NodeId(Uuid::from_u128(1));

    let mut high_clock = VectorClock::default();
    high_clock.increment(high);
    peer_write(&node, high, high_clock.clone(), seq_path(seq.id, "name"), "From high").await;

    // The low node saw the high node's write before making its own.
    let mut low_clock = high_clock.clone();
    low_clock.increment(low);
    peer_write(&node, low, low_clock, seq_path(seq.id, "name"), "Low, but later").await;

    assert_eq!(
        name_of(&node, seq.id).await.as_deref(),
        Some("Low, but later"),
        "the id tie-break is only for writes that are genuinely simultaneous",
    );
}

#[tokio::test]
async fn a_local_write_supersedes_an_earlier_peer_write() {
    let node = a_node().await;
    let seq = a_sequence("Original");
    create(&node, &seq).await;

    // A peer id above anything this node could tie-break against.
    let peer = NodeId(Uuid::max());
    let mut peer_clock = VectorClock::default();
    peer_clock.increment(peer);
    peer_write(&node, peer, peer_clock, seq_path(seq.id, "name"), "From peer").await;

    node.engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Mine, after"))
        .await
        .unwrap();

    assert_eq!(
        name_of(&node, seq.id).await.as_deref(),
        Some("Mine, after"),
        "an operator's edit must stick when it is the most recent one",
    );
}

// ── Helpers that reach into SyncManager ───────────────────────────────────────

async fn peer_count(sync: &SyncHandle) -> usize {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = sync.0.send(SyncCommand::PeerCount { reply: tx }).await;
    rx.await.unwrap_or(0)
}

async fn peer_ids(sync: &SyncHandle) -> Vec<NodeId> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = sync.0.send(SyncCommand::PeerIds { reply: tx }).await;
    rx.await.unwrap_or_default()
}

// ── Catch-up ──────────────────────────────────────────────────────────────────

/// A node that has never heard from anyone knows nothing to build a delta against,
/// so it gets the whole show.
#[tokio::test]
async fn a_brand_new_node_is_sent_a_snapshot() {
    let leader = a_node().await;
    let seq = a_sequence("Act 1");
    create(&leader, &seq).await;

    let joiner = a_node().await;
    joiner.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    eventually("the joiner to have the show", || async {
        name_of(&joiner, seq.id).await.as_deref() == Some("Act 1")
    })
    .await;
}

/// The point of the oplog: a node that was connected, missed a few writes, and came
/// back should be told those writes rather than re-sent the whole show.
#[tokio::test]
async fn a_returning_node_is_replayed_only_what_it_missed() {
    let leader = a_node().await;
    let returning = a_node().await;

    // Both start from the same place.
    let seq = a_sequence("Act 1");
    create(&leader, &seq).await;
    create(&returning, &seq).await;

    // The returning node makes a write of its own, so its clock is not empty and it
    // can describe what it knows.
    returning
        .engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 1"))
        .await
        .unwrap();

    // While it was away, the leader renamed the sequence.
    leader
        .engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 2"))
        .await
        .unwrap();

    returning.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    eventually("the missed write to be replayed", || async {
        name_of(&returning, seq.id).await.as_deref() == Some("Act 2")
    })
    .await;
}

#[tokio::test]
async fn catching_up_replays_several_writes_in_order() {
    let leader = a_node().await;
    let returning = a_node().await;

    let seq = a_sequence("Act 1");
    create(&leader, &seq).await;
    create(&returning, &seq).await;
    returning
        .engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 1"))
        .await
        .unwrap();

    for name in ["Second", "Third", "Fourth", "Final"] {
        leader
            .engine
            .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!(name))
            .await
            .unwrap();
    }

    returning.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    eventually("the last write to win", || async {
        name_of(&returning, seq.id).await.as_deref() == Some("Final")
    })
    .await;

    // Replayed out of order, an earlier rename would have landed last.
    assert_eq!(name_of(&returning, seq.id).await.as_deref(), Some("Final"));
}

#[tokio::test]
async fn a_node_that_missed_nothing_is_still_connected_and_current() {
    let leader = a_node().await;
    let peer = a_node().await;

    let seq = a_sequence("Act 1");
    create(&leader, &seq).await;
    create(&peer, &seq).await;
    peer.engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 1"))
        .await
        .unwrap();

    peer.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;
    eventually("the peers to register", || async { peer_count(&leader.sync).await == 1 }).await;

    // Live replication still works after a catch-up handshake.
    leader
        .engine
        .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 2"))
        .await
        .unwrap();
    eventually("live replication to continue", || async {
        name_of(&peer, seq.id).await.as_deref() == Some("Act 2")
    })
    .await;
}

#[tokio::test]
async fn local_writes_never_reach_the_operation_log() {
    let node = a_node().await;
    node.engine
        .set(
            vec![PathSegment::Key("session".into())],
            Lifecycle::Local,
            serde_json::json!({
                "is_advertising": true, "is_follower": false,
                "session_id": null, "discovered": [],
            }),
        )
        .await
        .unwrap();

    // A joiner asking for everything must not be handed this node's session state.
    let ops = node.engine.operations_since(node.engine.get_clock().await).await;
    assert!(
        ops.is_none_or(|o| o.iter().all(|op| op.lifecycle != Lifecycle::Local)),
        "LOCAL state is this node's own and must not be replicated",
    );
}

/// The behavioural catch-up tests above would also pass if every peer were sent a
/// snapshot, so these pin the decision itself.
mod catch_up_decision {
    use super::*;

    async fn a_node_with_writes(count: usize) -> (Node, Sequence) {
        let node = a_node().await;
        let seq = a_sequence("Act 1");
        create(&node, &seq).await;
        for i in 0..count {
            node.engine
                .set(
                    seq_path(seq.id, "name"),
                    Lifecycle::Persisted,
                    serde_json::json!(format!("v{i}")),
                )
                .await
                .unwrap();
        }
        (node, seq)
    }

    #[tokio::test]
    async fn a_peer_that_knows_nothing_gets_a_snapshot() {
        let (node, _) = a_node_with_writes(4).await;

        assert!(
            node.engine.operations_since(VectorClock::default()).await.is_none(),
            "an empty clock means every operation ever, which is worse than a snapshot",
        );
    }

    #[tokio::test]
    async fn a_peer_that_is_nearly_current_gets_the_operations_it_missed() {
        let (node, seq) = a_node_with_writes(20).await;

        // Its clock as of now, then two more writes it does not know about.
        let known = node.engine.get_clock().await;
        for name in ["missed one", "missed two"] {
            node.engine
                .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!(name))
                .await
                .unwrap();
        }

        let missing = node
            .engine
            .operations_since(known)
            .await
            .expect("a nearly current peer should get a delta, not a snapshot");

        assert_eq!(missing.len(), 2);
        assert_eq!(missing[0].value, "missed one");
        assert_eq!(missing[1].value, "missed two");
    }

    #[tokio::test]
    async fn a_peer_that_is_current_is_sent_an_empty_batch() {
        let (node, _) = a_node_with_writes(5).await;
        let known = node.engine.get_clock().await;

        let missing = node.engine.operations_since(known).await.expect("a delta, not a snapshot");
        assert!(missing.is_empty());
    }

    // ── Once the log has an end ───────────────────────────────────────────────
    //
    // A pruned row a peer never saw is the one way pruning can corrupt a session
    // rather than merely cost it a snapshot, so the guard is pinned from both
    // sides: it has to fire when there is something missing, and it must not fire
    // when there is not — a guard that always says "snapshot" would pass the first
    // test and quietly defeat catch-up altogether.

    #[tokio::test]
    async fn a_peer_behind_the_prune_floor_gets_a_snapshot() {
        let (node, _) = a_node_with_writes(6).await;
        let known = node.engine.get_clock().await;

        // Everything this peer knows about has been cut away since it last spoke.
        let seq = known.0.get(&node.id).copied().unwrap_or(0);
        oplog::raise_floor(&node.pool, node.id, seq + 1).await.unwrap();

        assert!(
            node.engine.operations_since(known).await.is_none(),
            "what survives is not the whole answer, so the peer must be sent the show",
        );
    }

    #[tokio::test]
    async fn a_peer_within_the_retention_still_gets_its_operations() {
        let (node, seq) = a_node_with_writes(20).await;
        let known = node.engine.get_clock().await;

        // Pruned, but only up to what this peer already has.
        let floor = known.0.get(&node.id).copied().unwrap_or(0);
        oplog::raise_floor(&node.pool, node.id, floor).await.unwrap();

        for name in ["missed one", "missed two"] {
            node.engine
                .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!(name))
                .await
                .unwrap();
        }

        let missing = node
            .engine
            .operations_since(known)
            .await
            .expect("a peer that missed nothing pruned should still get a delta");
        assert_eq!(missing.len(), 2, "and the guard has not collapsed into always-snapshot");
        assert_eq!(missing[1].value, "missed two");
    }

    /// The property the whole change risks. A write made while a peer was away, then
    /// pruned, must still reach it — by the snapshot, since the operation is gone.
    #[tokio::test]
    async fn a_write_made_while_a_peer_was_away_survives_being_pruned() {
        let (leader, seq) = a_node_with_writes(3).await;
        let known = leader.engine.get_clock().await;

        // While the peer is away.
        leader
            .engine
            .set(seq_path(seq.id, "name"), Lifecycle::Persisted, serde_json::json!("Act 2"))
            .await
            .unwrap();

        // And then the log is cut past it, so the operation itself no longer exists.
        let after = leader.engine.get_clock().await;
        let cut = after.0.get(&leader.id).copied().unwrap_or(0);
        sqlx::query("DELETE FROM oplog WHERE node_id = ?1 AND seq <= ?2")
            .bind(leader.id.0.to_string())
            .bind(cut as i64)
            .execute(leader.pool.as_ref())
            .await
            .unwrap();
        oplog::raise_floor(&leader.pool, leader.id, cut).await.unwrap();

        // The peer asks for what it missed and is told to take the whole show.
        assert!(
            leader.engine.operations_since(known).await.is_none(),
            "the operation is gone, so a delta would lose the write",
        );

        let follower = a_node().await;
        follower.engine.apply_state_snapshot(leader.engine.get_snapshot().await).await;
        let _ = follower.engine.get(seq_path(seq.id, "name")).await;

        assert_eq!(
            follower.engine.get(seq_path(seq.id, "name")).await.unwrap(),
            serde_json::json!("Act 2"),
            "and the write made while it was away is what it holds",
        );
    }

    #[tokio::test]
    async fn a_peer_missing_most_of_the_log_gets_a_snapshot() {
        let node = a_node().await;
        let seq = a_sequence("Act 1");
        create(&node, &seq).await;

        // Know about the first write only, then fall a long way behind.
        let known = node.engine.get_clock().await;
        for i in 0..20 {
            node.engine
                .set(
                    seq_path(seq.id, "name"),
                    Lifecycle::Persisted,
                    serde_json::json!(format!("v{i}")),
                )
                .await
                .unwrap();
        }

        assert!(
            node.engine.operations_since(known).await.is_none(),
            "replaying most of the log is a slow snapshot, so send the real one",
        );
    }
}

// ── Leader failover ───────────────────────────────────────────────────────────

/// Wire up a leader with two followers, all of them agreeing on who leads.
async fn a_session_of_three() -> (Node, Node, Node) {
    let leader = a_node().await;
    let first = a_node().await;
    let second = a_node().await;

    let session = Uuid::new_v4();
    let show = Uuid::new_v4();
    for follower in [&first, &second] {
        follower.sync.set_leader(leader.id).await;
        follower.sync.connect_peer(leader.addr, session, show).await;
    }
    eventually("both followers to register", || async { peer_count(&leader.sync).await == 2 })
        .await;

    (leader, first, second)
}

#[tokio::test]
async fn losing_the_leader_promotes_a_survivor() {
    let (leader, first, second) = a_session_of_three().await;
    let expected = first.id.min(second.id);

    // The leader's machine goes away.
    leader.sync.disconnect_all().await;

    eventually("both survivors to agree on a new leader", || async {
        first.sync.leader().await == Some(expected) && second.sync.leader().await == Some(expected)
    })
    .await;
}

/// Agreement is the whole point: two nodes both believing they lead is two consoles
/// driving one rig.
#[tokio::test]
async fn every_survivor_picks_the_same_new_leader() {
    let (leader, first, second) = a_session_of_three().await;
    leader.sync.disconnect_all().await;

    eventually("the election to settle", || async {
        first.sync.leader().await.is_some_and(|l| l != leader.id)
            && second.sync.leader().await.is_some_and(|l| l != leader.id)
    })
    .await;

    assert_eq!(
        first.sync.leader().await,
        second.sync.leader().await,
        "survivors must not disagree about who is leading",
    );
}

#[tokio::test]
async fn the_new_leader_is_one_of_the_survivors() {
    let (leader, first, second) = a_session_of_three().await;
    leader.sync.disconnect_all().await;

    eventually("the election to settle", || async {
        first.sync.leader().await.is_some_and(|l| l != leader.id)
    })
    .await;

    let elected = first.sync.leader().await.unwrap();
    assert!(elected == first.id || elected == second.id);
    assert_ne!(elected, leader.id, "the node that went away must not stay leader");
}

#[tokio::test]
async fn a_follower_leaving_does_not_change_the_leader() {
    let (leader, first, second) = a_session_of_three().await;

    // A follower's machine goes away rather than the leader's.
    second.sync.disconnect_all().await;
    eventually("the leader to notice", || async { peer_count(&leader.sync).await == 1 }).await;

    assert_eq!(leader.sync.leader().await, Some(leader.id));
    assert_eq!(first.sync.leader().await, Some(leader.id), "nothing about leadership changed");
}

#[tokio::test]
async fn the_last_node_standing_leads_itself() {
    let leader = a_node().await;
    let follower = a_node().await;
    follower.sync.set_leader(leader.id).await;
    follower.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;
    eventually("the peer to register", || async { peer_count(&leader.sync).await == 1 }).await;

    leader.sync.disconnect_all().await;

    eventually("the follower to take over", || async {
        follower.sync.leader().await == Some(follower.id)
    })
    .await;
}

// ── Live values from a device ─────────────────────────────────────────────────

#[tokio::test]
async fn a_sensor_reading_on_the_leader_reaches_the_follower() {
    // Playback output is derived from cue state, so every node works it out for
    // itself. An input cannot be: it came off a wire attached to one node, and the
    // only way the rest of the show learns about it is for that node to send it.
    use pult_schema::types::fixture::{Fixture, FixtureAddress};

    let leader = a_node().await;
    let follower = a_node().await;
    follower.sync.connect_peer(leader.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    let fixture = Fixture {
        id: Uuid::new_v4(),
        name: "Doorbell".into(),
        fixture_type_id: Uuid::new_v4(),
        address: FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: None },
        position: None,
        live_values: Default::default(),
        live_effects: Default::default(),
        live_fades: Default::default(),
    };
    leader
        .engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::to_value(&fixture).unwrap(),
        )
        .await
        .unwrap();

    eventually("the follower to have the fixture", || async {
        follower
            .engine
            .get(vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture.id)])
            .await
            .is_ok()
    })
    .await;

    leader
        .engine
        .set_live_value(
            fixture.id,
            "Contact:3".into(),
            serde_json::json!({ "type": "Bool", "value": true }),
        )
        .await
        .unwrap();

    eventually("the reading to cross to the follower", || async {
        let Ok(fixture) = follower
            .engine
            .get(vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture.id)])
            .await
        else {
            return false;
        };
        fixture["live_values"]["Contact:3"]["value"] == serde_json::json!(true)
    })
    .await;
}

// ── Stations ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_station_row_reaches_the_other_console() {
    // Each station publishes one row about itself and nobody arbitrates: the rows
    // converge because no two nodes ever write the same one.
    use pult_schema::types::station::Station;

    let one = a_node().await;
    let two = a_node().await;
    two.sync.connect_peer(one.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    let station = Station {
        id: one.id.0,
        hostname: "booth".into(),
        is_leader: true,
        sync_addr: one.addr.to_string(),
        http_addr: String::new(),
        cpu_percent: 3.5,
        mem_used: 100,
        mem_total: 1000,
        uptime_s: 5,
        output_plugins: vec!["House".into()],
        computes_fixtures: 0,
        total_fixtures: 0,
        last_seen: Utc::now(),
    };
    one.engine
        .set(
            vec![PathSegment::Key("stations".into()), PathSegment::Key("__create".into())],
            Lifecycle::Synced,
            serde_json::to_value(&station).unwrap(),
        )
        .await
        .unwrap();

    eventually("the station to appear on the other console", || async {
        two.engine
            .get(vec![PathSegment::Key("stations".into()), PathSegment::Id(one.id.0)])
            .await
            .map(|s| s["hostname"] == serde_json::json!("booth"))
            .unwrap_or(false)
    })
    .await;
}

#[tokio::test]
async fn a_station_row_is_not_written_to_the_showfile() {
    // Which machines are on tonight is not part of the show.
    use pult_schema::registry::EntityMeta;

    let meta = EntityMeta::by_table("stations").expect("stations is a registered entity");
    assert!(meta.upsert_one.is_none(), "SYNCED-only, so there is nothing to persist");
    assert!(meta.load_all.is_none());
}

#[tokio::test]
async fn a_measured_latency_shows_up_against_the_peer_that_answered() {
    // Heartbeats are five seconds apart, so rather than wait for one, the ack path
    // is driven directly — the arithmetic itself is covered in peer::tests.
    let node = a_node().await;
    let mut links = node.sync_mgr_links.clone();

    node.sync
        .0
        .send(SyncCommand::PeerLatency {
            node_id: NodeId(Uuid::new_v4()),
            rtt: Duration::from_micros(2500),
            unanswered: 0,
        })
        .await
        .unwrap();

    eventually("the latency to be published", || {
        let links = links.clone();
        async move { !links.borrow().is_empty() }
    })
    .await;

    let measured = links.borrow_and_update().clone();
    let link = measured.values().next().expect("one link");
    assert_eq!(link.rtt_ms, Some(2.5), "microseconds become milliseconds");
    assert_eq!(link.unanswered, 0);
    assert!(link.measured_at.is_some());
}

#[tokio::test]
async fn losing_a_peer_forgets_the_latency_to_it() {
    // A number that stops updating is worse than no number: it reads as a healthy
    // link long after the cable is out.
    let node = a_node().await;
    let mut links = node.sync_mgr_links.clone();
    let peer = NodeId(Uuid::new_v4());

    node.sync
        .0
        .send(SyncCommand::PeerLatency { node_id: peer, rtt: Duration::from_millis(1), unanswered: 0 })
        .await
        .unwrap();
    eventually("the latency to appear", || {
        let links = links.clone();
        async move { !links.borrow().is_empty() }
    })
    .await;

    node.sync.0.send(SyncCommand::PeerLost(peer)).await.unwrap();

    eventually("the latency to be forgotten", || {
        let links = links.clone();
        async move { links.borrow().is_empty() }
    })
    .await;
    let _ = links.borrow_and_update();
}

// ── Plugin stores ─────────────────────────────────────────────────────────────

/// A show-scoped plugin store is an ordinary entity, so it replicates like one.
///
/// This is the whole payoff of not writing a bespoke table for it: nothing in
/// the sync layer knows what a plugin is, and a macro written on one console is
/// on the other because `plugin_data` is a collection like any other.
#[tokio::test]
async fn a_plugins_show_scoped_write_reaches_the_other_station() {
    use pult_schema::types::PluginDatum;

    let one = a_node().await;
    let two = a_node().await;
    two.sync.connect_peer(one.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    let id = PluginDatum::id_for("macros", "saved", "opening");
    let datum = PluginDatum {
        id,
        plugin_id: "macros".into(),
        store: "saved".into(),
        key: "opening".into(),
        value: serde_json::json!("what the operator saved"),
    };
    one.engine
        .set(
            vec![
                PathSegment::Key("plugin_data".into()),
                PathSegment::Key("__create".into()),
            ],
            Lifecycle::Persisted,
            serde_json::to_value(&datum).unwrap(),
        )
        .await
        .unwrap();

    let value_on_two = || async {
        two.engine
            .get(vec![
                PathSegment::Key("plugin_data".into()),
                PathSegment::Id(id),
                PathSegment::Key("value".into()),
            ])
            .await
            .ok()
    };
    eventually("the plugin's data to replicate", || async {
        value_on_two().await == Some(serde_json::json!("what the operator saved"))
    })
    .await;
}

/// Two stations writing one key write one row.
///
/// The id is a UUIDv5 over `(plugin_id, store, key)`, so both sides create the
/// *same* entity and the existing per-path conflict resolution applies. With a
/// fresh id each, this would be two rows holding one key — not a conflict the
/// vector clock resolves, but a duplicate it has no reason to notice, and a
/// plugin reading back two values for one key.
#[tokio::test]
async fn two_stations_writing_one_key_converge_on_one_row() {
    use pult_schema::types::PluginDatum;

    let one = a_node().await;
    let two = a_node().await;

    // Each writes the same key before they have ever spoken, which is the
    // split-brain the derived id exists to survive.
    let id = PluginDatum::id_for("macros", "saved", "opening");
    for (node, what) in [(&one, "one's answer"), (&two, "two's answer")] {
        let datum = PluginDatum {
            id,
            plugin_id: "macros".into(),
            store: "saved".into(),
            key: "opening".into(),
            value: serde_json::json!(what),
        };
        node.engine
            .set(
                vec![
                    PathSegment::Key("plugin_data".into()),
                    PathSegment::Key("__create".into()),
                ],
                Lifecycle::Persisted,
                serde_json::to_value(&datum).unwrap(),
            )
            .await
            .unwrap();
    }

    two.sync.connect_peer(one.addr, Uuid::new_v4(), Uuid::new_v4()).await;

    async fn row_count(node: &Node) -> Option<usize> {
        node.engine
            .get(vec![PathSegment::Key("plugin_data".into())])
            .await
            .ok()
            .and_then(|v| v.as_array().map(|rows| rows.len()))
    }
    async fn value_at(node: &Node, id: Uuid) -> Option<serde_json::Value> {
        node.engine
            .get(vec![
                PathSegment::Key("plugin_data".into()),
                PathSegment::Id(id),
                PathSegment::Key("value".into()),
            ])
            .await
            .ok()
    }

    eventually("both sides to settle on one row", || async {
        row_count(&one).await == Some(1) && row_count(&two).await == Some(1)
    })
    .await;

    // And on the same value, whichever it is: the point is one key, one answer.
    eventually("the two to agree", || async {
        value_at(&one, id).await == value_at(&two, id).await
    })
    .await;
}
