use anyhow::Result;
use chrono::Utc;
use pult_schema::{
    events::operation::{NodeId, Operation, VectorClock},
    path::Path,
};
use serde_json::Value;
use tokio::{
    net::TcpStream,
    sync::mpsc,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::engine::{EngineCommand, EngineHandle};

use super::protocol::{read_frame, write_frame, SyncMessage, PROTOCOL_VERSION};

pub struct PeerSender(pub mpsc::Sender<SyncMessage>);

/// Spawns an outbound peer connection task.
/// Returns a `PeerSender` for sending messages and a `NodeId` for the remote peer.
pub async fn spawn_outbound(
    stream: TcpStream,
    our_node_id: NodeId,
    session_id: Uuid,
    show_id: Uuid,
    // Who we think the leader is. Not sent: the peer tells us in its HelloAck.
    _leader_node_id: NodeId,
    engine: EngineHandle,
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
        },
    )
    .await?;

    // Wait for HelloAck
    let peer_node_id = match read_frame(&mut read_half).await? {
        SyncMessage::HelloAck { accepted: true, leader_node_id: remote_leader, .. } => {
            info!("[sync] connected to peer {addr}, leader={}", remote_leader.0);
            remote_leader
        }
        SyncMessage::HelloAck { accepted: false, rejection_reason, .. } => {
            anyhow::bail!("peer rejected handshake: {:?}", rejection_reason);
        }
        other => anyhow::bail!("expected HelloAck, got {:?}", other),
    };

    tokio::spawn(async move {
        if let Err(e) = run_peer_loop(
            read_half,
            write_half,
            outgoing,
            engine,
            our_node_id,
            peer_node_id,
        )
        .await
        {
            debug!("[sync] peer {addr} disconnected: {e}");
        }
    });

    Ok((peer_node_id, PeerSender(tx)))
}

/// Handles an inbound peer connection (we are the server side).
pub fn spawn_inbound(
    stream: TcpStream,
    our_node_id: NodeId,
    leader_node_id: NodeId,
    engine: EngineHandle,
    on_connected: mpsc::Sender<(NodeId, PeerSender)>,
) {
    tokio::spawn(async move {
        if let Err(e) = handle_inbound(
            stream,
            our_node_id,
            leader_node_id,
            engine,
            on_connected,
        )
        .await
        {
            debug!("[sync] inbound handshake failed: {e}");
        }
    });
}

async fn handle_inbound(
    stream: TcpStream,
    our_node_id: NodeId,
    leader_node_id: NodeId,
    engine: EngineHandle,
    on_connected: mpsc::Sender<(NodeId, PeerSender)>,
) -> Result<()> {
    let addr = stream.peer_addr()?;
    let (mut read_half, mut write_half) = stream.into_split();

    // Wait for Hello
    let peer_node_id = match read_frame(&mut read_half).await? {
        SyncMessage::Hello { node_id, protocol_version, .. } => {
            if protocol_version != PROTOCOL_VERSION {
                write_frame(
                    &mut write_half,
                    &SyncMessage::HelloAck {
                        accepted: false,
                        leader_node_id,
                        rejection_reason: Some(format!(
                            "protocol version mismatch: got {protocol_version}, expected {PROTOCOL_VERSION}"
                        )),
                    },
                )
                .await?;
                anyhow::bail!("protocol version mismatch from {addr}");
            }
            node_id
        }
        other => anyhow::bail!("expected Hello from {addr}, got {:?}", other),
    };

    // Send HelloAck
    write_frame(
        &mut write_half,
        &SyncMessage::HelloAck { accepted: true, leader_node_id, rejection_reason: None },
    )
    .await?;

    // Immediately send the full current state so the joiner starts up-to-date.
    write_frame(
        &mut write_half,
        &SyncMessage::StateSnapshot { state: engine.get_snapshot().await },
    )
    .await?;

    info!("[sync] inbound peer connected: {} from {addr}, snapshot sent", peer_node_id.0);

    let (tx, outgoing) = mpsc::channel::<SyncMessage>(64);
    let sender = PeerSender(tx);
    let _ = on_connected.send((peer_node_id, sender)).await;

    run_peer_loop(read_half, write_half, outgoing, engine, our_node_id, peer_node_id).await
}

async fn run_peer_loop(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    mut outgoing: mpsc::Receiver<SyncMessage>,
    engine: EngineHandle,
    _our_node_id: NodeId,
    peer_node_id: NodeId,
) -> Result<()> {
    let mut heartbeat_seq: u64 = 0;
    let heartbeat_interval = std::time::Duration::from_secs(5);
    let mut heartbeat_tick = tokio::time::interval(heartbeat_interval);

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
                match frame {
                    Ok(msg) => handle_incoming(msg, &engine, peer_node_id).await,
                    Err(e) => return Err(e),
                }
            }
            // Periodic heartbeat
            _ = heartbeat_tick.tick() => {
                write_frame(&mut write_half, &SyncMessage::Heartbeat { seq: heartbeat_seq }).await?;
                heartbeat_seq += 1;
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
        SyncMessage::Heartbeat { seq } => {
            // HeartbeatAck is sent from the write side; we just log receipt here.
            // A full implementation would track liveness state.
            debug!("[sync] heartbeat seq={seq} from peer {}", peer_node_id.0);
        }
        SyncMessage::HeartbeatAck { .. } => {}
        SyncMessage::LeaderChanged { new_leader_node_id } => {
            info!("[sync] leader changed to {}", new_leader_node_id.0);
        }
        SyncMessage::OperationRequest { .. } | SyncMessage::OperationBatch { .. } => {
            warn!("[sync] PERSISTED oplog replication not yet implemented");
        }
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
