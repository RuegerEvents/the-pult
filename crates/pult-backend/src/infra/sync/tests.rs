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
    infra::showfile,
};

use super::*;

// ── Harness ───────────────────────────────────────────────────────────────────

struct Node {
    id: NodeId,
    engine: EngineHandle,
    sync: SyncHandle,
    addr: SocketAddr,
}

/// A backend node with its own engine, showfile, and sync port.
async fn a_node() -> Node {
    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let id = NodeId(Uuid::new_v4());
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let engine = EngineHandle(tx);

    let (manager, sync, addr) =
        SyncManager::bind(id, 0, engine.clone()).await.expect("bind an ephemeral sync port");
    tokio::spawn(manager.run());

    let (show_engine, _broadcast) = ShowEngine::new_with_rx(id, rx, pool, Some(sync.clone()));
    tokio::spawn(show_engine.run());

    Node { id, engine, sync, addr }
}

fn seq_path(id: Uuid, field: &str) -> Path {
    vec![
        PathSegment::Key("sequences".into()),
        PathSegment::Id(id),
        PathSegment::Key(field.into()),
    ]
}

fn a_sequence(name: &str) -> Sequence {
    Sequence { id: Uuid::new_v4(), name: name.into(), cue_ids: vec![], active_cue_index: None }
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
