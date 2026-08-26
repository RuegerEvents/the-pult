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
    let mut heartbeat_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut liveness_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut last_heard = tokio::time::Instant::now();

    loop {
        tokio::select! {
            // Outgoing message from SyncManager
            msg = outgoing.recv() => {
                match msg {
                    Some(msg) => write_frame(&mut write_half, &msg).await?,
                    None => break, // SyncManager dropped us
                }
            }
            // Incoming message from peer
            frame = read_frame(&mut read_half) => {
                let msg = frame?;
                last_heard = tokio::time::Instant::now();
                // A heartbeat has to be answered from the read side, where the write
                // half is in scope; handle_incoming has no way to reply.
                if let SyncMessage::Heartbeat { seq } = msg {
                    debug!("[sync] heartbeat seq={seq} from peer {}", peer_node_id.0);
                    write_frame(&mut write_half, &SyncMessage::HeartbeatAck { seq }).await?;
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
                    write_frame(&mut write_half, &reply).await?;
                    continue;
                }
                handle_incoming(msg, &engine, peer_node_id).await;
            }
            // Periodic heartbeat
            _ = heartbeat_tick.tick() => {
                write_frame(&mut write_half, &SyncMessage::Heartbeat { seq: heartbeat_seq }).await?;
                heartbeat_seq += 1;
            }
            // Liveness. A TCP connection can stay open long after the node behind it
            // has stopped answering, so silence is what we watch, not the socket.
            _ = liveness_tick.tick() => {
                if last_heard.elapsed() > PEER_TIMEOUT {
                    anyhow::bail!(
                        "peer {} silent for {:?}",
                        peer_node_id.0,
                        last_heard.elapsed(),
                    );
                }
            }
        }
    }
    Ok(())
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
