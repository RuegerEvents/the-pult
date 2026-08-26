use std::{collections::HashMap, net::SocketAddr};

use pult_schema::{
    events::operation::{NodeId, VectorClock},
    path::Path,
};
use anyhow::Result;
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::engine::EngineHandle;

pub mod protocol;
pub mod peer;

use peer::{spawn_inbound, spawn_outbound, PeerSender};
use protocol::SyncMessage;

// ── SyncCommand ───────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "PeerCount, PeerIds, and Stop are used by the tests only")]
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
    /// Query which peers are connected.
    PeerIds { reply: oneshot::Sender<Vec<NodeId>> },
    /// A peer connection ended. Sent by the peer task as it exits.
    PeerLost(NodeId),
    /// The leader told us who is in the session.
    SetMembers(Vec<NodeId>),
    /// Query who this node currently believes leads the session.
    Leader { reply: oneshot::Sender<NodeId> },
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

    /// Who this node currently believes leads the session.
    #[allow(dead_code, reason = "used by the tests")]
    pub async fn leader(&self) -> Option<NodeId> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(SyncCommand::Leader { reply: tx }).await;
        rx.await.ok()
    }
}

// ── SyncManager ───────────────────────────────────────────────────────────────

pub struct SyncManager {
    node_id: NodeId,
    /// Who the session leader is. A watch rather than a copy: the accept loop runs in
    /// its own task, and a copy taken at startup meant SetLeader never reached an
    /// inbound handshake, which then reported the wrong leader for the rest of the show.
    leader: watch::Sender<NodeId>,
    listener: Option<TcpListener>,
    engine: EngineHandle,
    rx: mpsc::Receiver<SyncCommand>,
    /// Inbound peer connection notifications (from `spawn_inbound` tasks).
    inbound_rx: mpsc::Receiver<(NodeId, PeerSender)>,
    inbound_tx: mpsc::Sender<(NodeId, PeerSender)>,
    peers: HashMap<NodeId, PeerSender>,
    /// Everyone in the session, as last published by the leader. Includes this node.
    members: Vec<NodeId>,
    /// Told when this node takes over as leader, so the session can start advertising.
    promoted: Option<mpsc::Sender<NodeId>>,
    /// A handle to ourselves, for peer tasks to report their own exit.
    self_tx: mpsc::Sender<SyncCommand>,
}

impl SyncManager {
    /// Bind the sync port. Port 0 picks a free one, which is what the tests use;
    /// the bound address comes back so the caller can find out which.
    pub async fn bind(
        node_id: NodeId,
        sync_port: u16,
        engine: EngineHandle,
    ) -> Result<(Self, SyncHandle, SocketAddr)> {
        let listener = TcpListener::bind(format!("0.0.0.0:{sync_port}")).await?;
        let addr = listener.local_addr()?;
        info!("[sync] listening on {addr}");

        let (tx, rx) = mpsc::channel(64);
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let (leader, _) = watch::channel(node_id);
        let mgr = SyncManager {
            node_id,
            leader,
            listener: Some(listener),
            engine,
            rx,
            inbound_rx,
            inbound_tx,
            peers: HashMap::new(),
            members: vec![node_id],
            promoted: None,
            self_tx: tx.clone(),
        };
        Ok((mgr, SyncHandle(tx), addr))
    }

    /// Be told when this node is promoted to leader.
    pub fn on_promotion(&mut self, tx: mpsc::Sender<NodeId>) {
        self.promoted = Some(tx);
    }

    pub async fn run(mut self) {
        let Some(listener) = self.listener.take() else { return };
        let inbound_tx = self.inbound_tx.clone();
        let node_id = self.node_id;
        let engine = self.engine.clone();
        let leader = self.leader.subscribe();
        let on_lost = self.self_tx.clone();

        // Accept loop in a separate task
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        info!("[sync] inbound connection from {addr}");
                        spawn_inbound(
                            stream,
                            node_id,
                            leader.clone(),
                            engine.clone(),
                            inbound_tx.clone(),
                            on_lost.clone(),
                        );
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
                        self.publish_members().await;
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
                let engine = self.engine.clone();
                let on_lost = self.self_tx.clone();
                match tokio::net::TcpStream::connect(addr).await {
                    Ok(stream) => {
                        match spawn_outbound(stream, node_id, session_id, show_id, engine, on_lost).await {
                            Ok((peer_id, sender)) => {
                                info!("[sync] outbound peer connected: {}", peer_id.0);
                                self.peers.insert(peer_id, sender);
                                self.publish_members().await;
                            }
                            Err(e) => warn!("[sync] outbound handshake to {addr} failed: {e}"),
                        }
                    }
                    Err(e) => warn!("[sync] connect to {addr} failed: {e}"),
                }
            }
            SyncCommand::SetLeader(node_id) => {
                let _ = self.leader.send(node_id);
                if !self.members.contains(&node_id) {
                    self.members.push(node_id);
                }
            }
            SyncCommand::SetMembers(members) => {
                self.members = members;
                if !self.members.contains(&self.node_id) {
                    self.members.push(self.node_id);
                }
            }
            SyncCommand::Leader { reply } => {
                let _ = reply.send(*self.leader.borrow());
            }
            SyncCommand::PeerLost(node_id) => {
                if self.peers.remove(&node_id).is_some() {
                    info!("[sync] lost peer {}", node_id.0);
                }
                if node_id == *self.leader.borrow() {
                    self.elect_leader(node_id).await;
                } else if self.node_id == *self.leader.borrow() {
                    self.publish_members().await;
                }
            }
            SyncCommand::PeerCount { reply } => {
                let _ = reply.send(self.peers.len());
            }
            SyncCommand::PeerIds { reply } => {
                let _ = reply.send(self.peers.keys().copied().collect());
            }
            SyncCommand::DisconnectAll => {
                // Drop all senders — peer tasks will exit when their channel closes.
                self.peers.clear();
                info!("[sync] disconnected all peers");
            }
            SyncCommand::Stop => {}
        }
    }

    /// Choose a new leader after losing the old one.
    ///
    /// Every survivor runs this over the same membership list and picks the lowest
    /// surviving node id, so they agree without exchanging a single message. Lowest
    /// id rather than freshest state, because agreement is what prevents two nodes
    /// both driving the rig, and catch-up runs in both directions on reconnect so
    /// whoever leads ends up with everything either of them had.
    async fn elect_leader(&mut self, lost: NodeId) {
        let winner = self
            .members
            .iter()
            .copied()
            .filter(|id| *id != lost)
            .min()
            .unwrap_or(self.node_id);

        self.members.retain(|id| *id != lost);
        let _ = self.leader.send(winner);

        if winner == self.node_id {
            info!("[sync] leader {} is gone; taking over", lost.0);
            self.fan_out(SyncMessage::LeaderChanged { new_leader_node_id: winner }).await;
            self.publish_members().await;
            if let Some(tx) = &self.promoted {
                let _ = tx.send(winner).await;
            }
        } else {
            info!("[sync] leader {} is gone; {} takes over", lost.0, winner.0);
        }
    }

    /// Tell every peer who is in the session. Only the leader does this: two nodes
    /// publishing different lists is exactly what the list exists to prevent.
    async fn publish_members(&mut self) {
        if self.node_id != *self.leader.borrow() {
            return;
        }
        let mut members: Vec<NodeId> = self.peers.keys().copied().collect();
        members.push(self.node_id);
        members.sort();
        self.members = members.clone();
        self.fan_out(SyncMessage::SessionMembers { members }).await;
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

#[cfg(test)]
mod tests;
