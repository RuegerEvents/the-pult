use anyhow::Result;
use chrono::Utc;
use pult_schema::{
    events::operation::{NodeId, Operation, VectorClock},
    path::Path,
};
use serde_json::Value;
use tokio::{
    net::TcpStream,
    sync::{mpsc, watch},
};
use tracing::{debug, info};
use uuid::Uuid;

use crate::engine::{EngineCommand, EngineHandle};

use super::{
    protocol::{read_frame, write_frame, SyncMessage, PROTOCOL_VERSION},
    SyncCommand,
};

/// Heartbeats sent and not yet answered, with when they went out.
///
/// Bounded on purpose: a link that has stopped answering does not need its whole
/// history remembered, only that it is not answering. The oldest are dropped, which
/// also means a heartbeat answered absurdly late is ignored rather than reported as
/// a minute of latency.
#[derive(Default)]
struct Outstanding(Vec<(u64, tokio::time::Instant)>);

const MAX_OUTSTANDING: usize = 8;

impl Outstanding {
    fn sent(&mut self, seq: u64, at: tokio::time::Instant) {
        self.0.push((seq, at));
        if self.0.len() > MAX_OUTSTANDING {
            self.0.remove(0);
        }
    }

    /// Round-trip time for an answered heartbeat, and forget everything older —
    /// those were not answered and never will be.
    fn answered(&mut self, seq: u64, at: tokio::time::Instant) -> Option<std::time::Duration> {
        let index = self.0.iter().position(|(s, _)| *s == seq)?;
        let (_, sent_at) = self.0[index];
        self.0.drain(..=index);
        Some(at.duration_since(sent_at).into())
    }

    fn unanswered(&self) -> u32 {
        self.0.len() as u32
    }
}

pub struct PeerSender(pub mpsc::Sender<SyncMessage>);

/// Spawns an outbound peer connection task.
/// Returns a `PeerSender` for sending messages and a `NodeId` for the remote peer.
pub async fn spawn_outbound(
    stream: TcpStream,
    our_node_id: NodeId,
    session_id: Uuid,
    show_id: Uuid,
    engine: EngineHandle,
    on_lost: mpsc::Sender<SyncCommand>,
) -> Result<(NodeId, PeerSender)> {
    let addr = stream.peer_addr()?;
    let (tx, outgoing) = mpsc::channel::<SyncMessage>(64);
    let (mut read_half, mut write_half) = stream.into_split();

    // Send Hello
    write_frame(
        &mut write_half,
        &SyncMessage::Hello {
            node_id: our_node_id,
            protocol_version: PROTOCOL_VERSION,
            session_id,
            show_id,
            // What we already know, so the peer can send us only the difference.
            clock: engine.get_clock().await,
        },
    )
    .await?;

    // Wait for HelloAck
    let (peer_node_id, peer_clock) = match read_frame(&mut read_half).await? {
        SyncMessage::HelloAck { accepted: true, node_id, leader_node_id: remote_leader, clock, .. } => {
            info!("[sync] connected to peer {}, leader={}", node_id.0, remote_leader.0);
            // The peer we are talking to, not the leader. Keying the connection by the
            // leader made every node connected to one leader collide in the peer map.
            (node_id, clock)
        }
        SyncMessage::HelloAck { accepted: false, rejection_reason, .. } => {
            anyhow::bail!("peer rejected handshake: {:?}", rejection_reason);
        }
        other => anyhow::bail!("expected HelloAck, got {:?}", other),
    };

    // Catch-up runs both ways. A node that reconnects is not always the stale one:
    // it may have been writing while it was away.
    if let Some(operations) = engine.operations_since(peer_clock).await {
        if !operations.is_empty() {
            debug!("[sync] replaying {} operations to peer {}", operations.len(), peer_node_id.0);
            write_frame(&mut write_half, &SyncMessage::OperationBatch { operations }).await?;
        }
    }

    let to_manager = on_lost.clone();
    tokio::spawn(async move {
        if let Err(e) = run_peer_loop(
            read_half,
            write_half,
            outgoing,
            engine,
            our_node_id,
            peer_node_id,
            to_manager,
        )
        .await
        {
            debug!("[sync] peer {addr} disconnected: {e}");
        }
        // Report our own exit, so a lost leader is noticed at once rather than
        // whenever the next broadcast happens to fail.
        let _ = on_lost.send(SyncCommand::PeerLost(peer_node_id)).await;
    });

    Ok((peer_node_id, PeerSender(tx)))
}

/// Handles an inbound peer connection (we are the server side).
pub fn spawn_inbound(
    stream: TcpStream,
    our_node_id: NodeId,
    leader: watch::Receiver<NodeId>,
    engine: EngineHandle,
    on_connected: mpsc::Sender<(NodeId, PeerSender)>,
    on_lost: mpsc::Sender<SyncCommand>,
) {
    tokio::spawn(async move {
        match handle_inbound(stream, our_node_id, leader, engine, on_connected, on_lost.clone()).await {
            Ok(peer_node_id) => {
                let _ = on_lost.send(SyncCommand::PeerLost(peer_node_id)).await;
            }
            Err(e) => debug!("[sync] inbound peer ended: {e}"),
        }
    });
}

async fn handle_inbound(
    stream: TcpStream,
    our_node_id: NodeId,
    leader: watch::Receiver<NodeId>,
    engine: EngineHandle,
    on_connected: mpsc::Sender<(NodeId, PeerSender)>,
    to_manager: mpsc::Sender<SyncCommand>,
) -> Result<NodeId> {
    let addr = stream.peer_addr()?;
    // Read the leader at handshake time, not at startup.
    let leader_node_id = *leader.borrow();
    let (mut read_half, mut write_half) = stream.into_split();

    // Wait for Hello
    let (peer_node_id, peer_clock) = match read_frame(&mut read_half).await? {
        SyncMessage::Hello { node_id, protocol_version, clock, .. } => {
            if protocol_version != PROTOCOL_VERSION {
                write_frame(
                    &mut write_half,
                    &SyncMessage::HelloAck {
                        accepted: false,
                        node_id: our_node_id,
                        leader_node_id,
                        clock: Default::default(),
                        rejection_reason: Some(format!(
                            "protocol version mismatch: got {protocol_version}, expected {PROTOCOL_VERSION}"
                        )),
                    },
                )
                .await?;
                anyhow::bail!("protocol version mismatch from {addr}");
            }
            (node_id, clock)
        }
        other => anyhow::bail!("expected Hello from {addr}, got {:?}", other),
    };

    // Send HelloAck
    write_frame(
        &mut write_half,
        &SyncMessage::HelloAck {
            accepted: true,
            node_id: our_node_id,
            leader_node_id,
            clock: engine.get_clock().await,
            rejection_reason: None,
        },
    )
    .await?;

    // Bring the joiner up to date, replaying what it missed where that is cheaper
    // than sending the whole show.
    match engine.operations_since(peer_clock).await {
        Some(operations) => {
            info!(
                "[sync] inbound peer {} from {addr}, replaying {} operations",
                peer_node_id.0,
                operations.len(),
            );
            write_frame(&mut write_half, &SyncMessage::OperationBatch { operations }).await?;
        }
        None => {
            info!("[sync] inbound peer {} from {addr}, sending a snapshot", peer_node_id.0);
            write_frame(
                &mut write_half,
                &SyncMessage::StateSnapshot { state: engine.get_snapshot().await },
            )
            .await?;
        }
    }

    let (tx, outgoing) = mpsc::channel::<SyncMessage>(64);
    let sender = PeerSender(tx);
    let _ = on_connected.send((peer_node_id, sender)).await;

    let result = run_peer_loop(
        read_half,
        write_half,
        outgoing,
        engine,
        our_node_id,
        peer_node_id,
        to_manager,
    )
    .await;
    if let Err(e) = result {
        debug!("[sync] peer {} disconnected: {e}", peer_node_id.0);
    }
    Ok(peer_node_id)
}

/// How often a heartbeat goes out.
pub const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a peer can stay silent before the connection is considered dead.
/// Three missed heartbeats: long enough to ride out a hiccup, short enough that a
/// pulled cable does not leave a ghost in the peer map for a whole show.
pub const PEER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(16);

async fn run_peer_loop(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    mut outgoing: mpsc::Receiver<SyncMessage>,
    engine: EngineHandle,
    _our_node_id: NodeId,
    peer_node_id: NodeId,
    to_manager: mpsc::Sender<SyncCommand>,
) -> Result<()> {
    let mut heartbeat_seq: u64 = 0;
    let mut outstanding = Outstanding::default();
    let mut heartbeat_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut liveness_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut last_heard = tokio::time::Instant::now();

    // Reading happens in its own task rather than in the `select!` below.
    //
    // `read_frame` takes a four-byte length and then the body, so it is not
    // cancel-safe: when a heartbeat or liveness tick wins the race, the half-finished
    // read is dropped and the bytes it already took off the socket go with it. Every
    // frame after that is read at the wrong offset, the connection dies, and it never
    // comes back — which is exactly the failure this once produced, rarely enough to
    // look like flakiness. `Receiver::recv` is cancel-safe, so the loop selects on
    // that and the socket is only ever read from one place.
    let (incoming_tx, mut incoming) = mpsc::channel::<SyncMessage>(64);
    let reader = tokio::spawn(async move {
        loop {
            match read_frame(&mut read_half).await {
                Ok(msg) => {
                    if incoming_tx.send(msg).await.is_err() {
                        break; // the loop below is gone
                    }
                }
                Err(e) => {
                    debug!("[sync] read from peer ended: {e}");
                    break;
                }
            }
        }
    });

    let result = loop {
        tokio::select! {
            // Outgoing message from SyncManager
            msg = outgoing.recv() => {
                match msg {
                    Some(msg) => {
                        if let Err(e) = write_frame(&mut write_half, &msg).await {
                            break Err(e);
                        }
                    }
                    None => break Ok(()), // SyncManager dropped us
                }
            }
            // Incoming message from peer
            frame = incoming.recv() => {
                // The reader stopping means the peer is gone or the stream broke.
                let Some(msg) = frame else { break Ok(()) };
                last_heard = tokio::time::Instant::now();
                // A heartbeat has to be answered from the read side, where the write
                // half is in scope; handle_incoming has no way to reply.
                if let SyncMessage::Heartbeat { seq } = msg {
                    debug!("[sync] heartbeat seq={seq} from peer {}", peer_node_id.0);
                    if let Err(e) = write_frame(&mut write_half, &SyncMessage::HeartbeatAck { seq }).await {
                        break Err(e);
                    }
                    continue;
                }
                // The ack for one of ours: the only place the round-trip time to
                // this peer can be known, because it is the only place that knows
                // when the heartbeat went out.
                if let SyncMessage::HeartbeatAck { seq } = msg {
                    if let Some(rtt) = outstanding.answered(seq, tokio::time::Instant::now()) {
                        let _ = to_manager
                            .send(SyncCommand::PeerLatency {
                                node_id: peer_node_id,
                                rtt,
                                unanswered: outstanding.unanswered(),
                            })
                            .await;
                    }
                    continue;
                }
                // Leadership messages go to SyncManager, which owns that state.
                match &msg {
                    SyncMessage::LeaderChanged { new_leader_node_id } => {
                        info!("[sync] leader is now {}", new_leader_node_id.0);
                        let _ = to_manager.send(SyncCommand::SetLeader(*new_leader_node_id)).await;
                        continue;
                    }
                    SyncMessage::SessionMembers { members } => {
                        debug!("[sync] session has {} members", members.len());
                        let _ = to_manager.send(SyncCommand::SetMembers(members.clone())).await;
                        continue;
                    }
                    _ => {}
                }
                if let SyncMessage::OperationRequest { known } = msg {
                    let reply = match engine.operations_since(known).await {
                        Some(operations) => SyncMessage::OperationBatch { operations },
                        None => SyncMessage::StateSnapshot { state: engine.get_snapshot().await },
                    };
                    if let Err(e) = write_frame(&mut write_half, &reply).await {
                        break Err(e);
                    }
                    continue;
                }
                handle_incoming(msg, &engine, peer_node_id).await;
            }
            // Periodic heartbeat
            _ = heartbeat_tick.tick() => {
                let beat = SyncMessage::Heartbeat { seq: heartbeat_seq };
                if let Err(e) = write_frame(&mut write_half, &beat).await {
                    break Err(e);
                }
                outstanding.sent(heartbeat_seq, tokio::time::Instant::now());
                heartbeat_seq += 1;
            }
            // Liveness. A TCP connection can stay open long after the node behind it
            // has stopped answering, so silence is what we watch, not the socket.
            _ = liveness_tick.tick() => {
                if last_heard.elapsed() > PEER_TIMEOUT {
                    break Err(anyhow::anyhow!(
                        "peer {} silent for {:?}",
                        peer_node_id.0,
                        last_heard.elapsed(),
                    ));
                }
            }
        }
    };

    reader.abort();
    result
}

async fn handle_incoming(msg: SyncMessage, engine: &EngineHandle, peer_node_id: NodeId) {
    match msg {
        SyncMessage::StateSnapshot { state } => {
            info!("[sync] received state snapshot from peer {}", peer_node_id.0);
            engine.apply_state_snapshot(state).await;
        }
        SyncMessage::SyncedBroadcast { path, value, clock, .. } => {
            apply_synced(engine, peer_node_id, path, value, clock).await;
        }
        // Heartbeat is answered in run_peer_loop, which holds the write half.
        SyncMessage::Heartbeat { .. } | SyncMessage::HeartbeatAck { .. } => {}
        SyncMessage::LeaderChanged { .. } | SyncMessage::SessionMembers { .. } => {
            // Handled in run_peer_loop, which can reach SyncManager.
        }
        SyncMessage::OperationBatch { operations } => {
            debug!("[sync] {} operations from peer {}", operations.len(), peer_node_id.0);
            engine.apply_operation_batch(operations).await;
        }
        // Answered in run_peer_loop, which holds the write half.
        SyncMessage::OperationRequest { .. } => {}
        other => {
            debug!("[sync] unexpected message: {:?}", other);
        }
    }
}

async fn apply_synced(
    engine: &EngineHandle,
    peer_node_id: NodeId,
    path: Path,
    value: Value,
    clock: VectorClock,
) {
    let lifecycle = pult_schema::registry::path_lifecycle(&path);
    let op = Operation {
        id: Uuid::new_v4(),
        node_id: peer_node_id,
        seq: 0,
        clock,
        lifecycle,
        path,
        value,
        timestamp: Utc::now(),
    };
    let _ = engine.0.send(EngineCommand::ApplyPeerOperation(op)).await;
}


#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> tokio::time::Instant {
        // A fixed origin, so the arithmetic is the thing under test.
        tokio::time::Instant::now() + std::time::Duration::from_millis(ms)
    }

    #[test]
    fn an_answered_heartbeat_gives_its_round_trip_time() {
        let mut outstanding = Outstanding::default();
        let start = tokio::time::Instant::now();
        outstanding.sent(1, start);

        let rtt = outstanding.answered(1, start + std::time::Duration::from_millis(12));

        assert_eq!(rtt, Some(std::time::Duration::from_millis(12)));
        assert_eq!(outstanding.unanswered(), 0);
    }

    #[test]
    fn an_ack_for_something_never_sent_measures_nothing() {
        let mut outstanding = Outstanding::default();
        assert_eq!(outstanding.answered(7, at(0)), None);
    }

    #[test]
    fn answering_a_later_heartbeat_gives_up_on_the_earlier_ones() {
        // Heartbeats are answered in order or not at all, so an ack for seq 3 means
        // 1 and 2 are lost — keeping them would report them as unanswered forever.
        let mut outstanding = Outstanding::default();
        let start = tokio::time::Instant::now();
        outstanding.sent(1, start);
        outstanding.sent(2, start);
        outstanding.sent(3, start);

        outstanding.answered(3, start + std::time::Duration::from_millis(5));

        assert_eq!(outstanding.unanswered(), 0);
    }

    #[test]
    fn heartbeats_that_go_unanswered_are_counted() {
        let mut outstanding = Outstanding::default();
        for seq in 0..3 {
            outstanding.sent(seq, tokio::time::Instant::now());
        }
        assert_eq!(outstanding.unanswered(), 3);
    }

    #[test]
    fn a_link_that_never_answers_does_not_grow_without_bound() {
        let mut outstanding = Outstanding::default();
        for seq in 0..100 {
            outstanding.sent(seq, tokio::time::Instant::now());
        }
        assert_eq!(outstanding.unanswered(), MAX_OUTSTANDING as u32);
    }
}
