use std::{collections::HashMap, net::SocketAddr};

use pult_schema::{
    events::operation::{NodeId, VectorClock},
    path::Path,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::engine::EngineHandle;

pub mod protocol;
pub mod peer;

use peer::{spawn_inbound, spawn_outbound, PeerSender};
use protocol::SyncMessage;

// ── SyncCommand ───────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "PeerCount and Stop have no caller yet")]
pub enum SyncCommand {
    /// Fan out an operation to all connected peers.
    BroadcastSynced {
        path: Path,
        value: serde_json::Value,
        clock: VectorClock,
    },
    /// Connect to a new peer (called by SessionManager on peer discovery).
    ConnectPeer {
        addr: SocketAddr,
        session_id: Uuid,
        show_id: Uuid,
    },
    /// Update which node is the current leader (for HelloAck).
    SetLeader(NodeId),
    /// Query how many peers are connected.
    PeerCount { reply: oneshot::Sender<usize> },
    /// Drop all peer connections (called on session Leave).
    DisconnectAll,
    Stop,
}

// ── SyncHandle ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SyncHandle(pub mpsc::Sender<SyncCommand>);

impl SyncHandle {
    pub async fn broadcast_synced(&self, path: Path, value: serde_json::Value, clock: VectorClock) {
        let _ = self.0.send(SyncCommand::BroadcastSynced { path, value, clock }).await;
    }

    pub async fn connect_peer(&self, addr: SocketAddr, session_id: Uuid, show_id: Uuid) {
        let _ = self.0.send(SyncCommand::ConnectPeer { addr, session_id, show_id }).await;
    }

    pub async fn set_leader(&self, node_id: NodeId) {
        let _ = self.0.send(SyncCommand::SetLeader(node_id)).await;
    }

    pub async fn disconnect_all(&self) {
        let _ = self.0.send(SyncCommand::DisconnectAll).await;
    }
}

// ── SyncManager ───────────────────────────────────────────────────────────────

pub struct SyncManager {
    node_id: NodeId,
    leader_node_id: NodeId,
    sync_port: u16,
    engine: EngineHandle,
    rx: mpsc::Receiver<SyncCommand>,
    /// Inbound peer connection notifications (from `spawn_inbound` tasks).
    inbound_rx: mpsc::Receiver<(NodeId, PeerSender)>,
    inbound_tx: mpsc::Sender<(NodeId, PeerSender)>,
    peers: HashMap<NodeId, PeerSender>,
}

impl SyncManager {
    pub fn new(
        node_id: NodeId,
        sync_port: u16,
        engine: EngineHandle,
    ) -> (Self, SyncHandle) {
        let (tx, rx) = mpsc::channel(64);
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let mgr = SyncManager {
            node_id,
            leader_node_id: node_id,
            sync_port,
            engine,
            rx,
            inbound_rx,
            inbound_tx,
            peers: HashMap::new(),
        };
        (mgr, SyncHandle(tx))
    }

    pub async fn run(mut self) {
        // Start TCP listener
        let bind = format!("0.0.0.0:{}", self.sync_port);
        let listener = match TcpListener::bind(&bind).await {
            Ok(l) => {
                info!("[sync] listening on {bind}");
                l
            }
            Err(e) => {
                warn!("[sync] failed to bind {bind}: {e}");
                return;
            }
        };

        let inbound_tx = self.inbound_tx.clone();
        let node_id = self.node_id;
        let engine = self.engine.clone();
        let leader_id = self.leader_node_id;

        // Accept loop in a separate task
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        info!("[sync] inbound connection from {addr}");
                        spawn_inbound(stream, node_id, leader_id, engine.clone(), inbound_tx.clone());
                    }
                    Err(e) => {
                        warn!("[sync] accept error: {e}");
                    }
                }
            }
        });

        self.event_loop().await;
    }

    async fn event_loop(&mut self) {
        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    match cmd {
                        None | Some(SyncCommand::Stop) => break,
                        Some(cmd) => self.handle_command(cmd).await,
                    }
                }
                // A newly accepted inbound peer completed its handshake
                connected = self.inbound_rx.recv() => {
                    if let Some((peer_id, sender)) = connected {
                        info!("[sync] registered inbound peer {}", peer_id.0);
                        self.peers.insert(peer_id, sender);
                    }
                }
            }
        }
    }

    async fn handle_command(&mut self, cmd: SyncCommand) {
        match cmd {
            SyncCommand::BroadcastSynced { path, value, clock } => {
                let msg = SyncMessage::SyncedBroadcast {
                    node_id: self.node_id,
                    path,
                    value,
                    clock,
                };
                self.fan_out(msg).await;
            }
            SyncCommand::ConnectPeer { addr, session_id, show_id } => {
                let node_id = self.node_id;
                let leader_id = self.leader_node_id;
                let engine = self.engine.clone();
                match tokio::net::TcpStream::connect(addr).await {
                    Ok(stream) => {
                        match spawn_outbound(stream, node_id, session_id, show_id, leader_id, engine).await {
                            Ok((peer_id, sender)) => {
                                info!("[sync] outbound peer connected: {}", peer_id.0);
                                self.peers.insert(peer_id, sender);
                            }
                            Err(e) => warn!("[sync] outbound handshake to {addr} failed: {e}"),
                        }
                    }
                    Err(e) => warn!("[sync] connect to {addr} failed: {e}"),
                }
            }
            SyncCommand::SetLeader(node_id) => {
                self.leader_node_id = node_id;
            }
            SyncCommand::PeerCount { reply } => {
                let _ = reply.send(self.peers.len());
            }
            SyncCommand::DisconnectAll => {
                // Drop all senders — peer tasks will exit when their channel closes.
                self.peers.clear();
                info!("[sync] disconnected all peers");
            }
            SyncCommand::Stop => {}
        }
    }

    async fn fan_out(&mut self, msg: SyncMessage) {
        let mut dead = vec![];
        for (peer_id, sender) in &self.peers {
            if sender.0.send(msg.clone()).await.is_err() {
                dead.push(*peer_id);
            }
        }
        for id in dead {
            info!("[sync] removing disconnected peer {}", id.0);
            self.peers.remove(&id);
        }
    }
}
