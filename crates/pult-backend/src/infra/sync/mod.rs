use std::{collections::HashMap, net::SocketAddr};

use pult_schema::{
    events::operation::{Authorship, NodeId, VectorClock},
    path::Path,
};
use anyhow::Result;
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::engine::EngineHandle;

pub mod protocol;
pub mod peer;

use peer::{spawn_inbound, spawn_outbound, PeerSender};
use pult_schema::types::station::{PeerLink, PeerLinks};
use protocol::SyncMessage;

// ── SyncCommand ───────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "PeerCount, PeerIds, and Stop are used by the tests only")]
pub enum SyncCommand {
    /// Fan out an operation to all connected peers.
    BroadcastSynced {
        path: Path,
        value: serde_json::Value,
        clock: VectorClock,
        authorship: Authorship,
    },
    /// Connect to a new peer (called by SessionManager on peer discovery).
    /// Dial a peer, at the first of these addresses that answers.
    ///
    /// A list because a station advertises every address it has, and which of them
    /// reaches *this* machine is not something either end can work out on its own —
    /// see `session::reachable_at`, which is what puts them in this order.
    ///
    /// `reply` carries what happened, so a caller can tell a session it joined from
    /// one it only asked to join.
    ConnectPeer {
        addrs: Vec<SocketAddr>,
        session_id: Uuid,
        show_id: Uuid,
        reply: oneshot::Sender<Result<SocketAddr, String>>,
    },
    /// Update which node is the current leader (for HelloAck).
    SetLeader(NodeId),
    /// Query how many peers are connected.
    PeerCount { reply: oneshot::Sender<usize> },
    /// Query which peers are connected.
    PeerIds { reply: oneshot::Sender<Vec<NodeId>> },
    /// A peer connection ended. Sent by the peer task as it exits.
    PeerLost(NodeId),
    /// A heartbeat came back. The only measurement of the link to this peer, and it
    /// belongs to this node: the same link measured from the other end is a
    /// different path and a different number.
    PeerLatency { node_id: NodeId, rtt: std::time::Duration, unanswered: u32 },
    /// The leader told us who is in the session.
    SetMembers(Vec<NodeId>),
    /// Query who this node currently believes leads the session.
    Leader { reply: oneshot::Sender<NodeId> },
    /// Ask one peer to publish its log at `level`, or `None` to withdraw the ask.
    ///
    /// Sent whenever the set of browsers watching that peer changes, including to
    /// nothing — which is the unwind. Nothing expires, because the ask lives on the
    /// connection and a connection that ends takes it with it.
    RaisePeerLog { node_id: NodeId, level: Option<pult_schema::ws::LogLevel> },
    /// Drop all peer connections (called on session Leave).
    DisconnectAll,
    Stop,
}

// ── SyncHandle ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SyncHandle(pub mpsc::Sender<SyncCommand>);

impl SyncHandle {
    /// Replicate a write, with who made it and what it replaced.
    ///
    /// The two extras ride along so a peer's oplog is as complete as this one's:
    /// without them the history panel on another console could say what changed but
    /// not who changed it, and a user whose two clients are on different stations
    /// would find half their work unundoable.
    pub async fn broadcast_synced(
        &self,
        path: Path,
        value: serde_json::Value,
        clock: VectorClock,
        authorship: Authorship,
    ) {
        let _ = self
            .0
            .send(SyncCommand::BroadcastSynced { path, value, clock, authorship })
            .await;
    }

    /// Dial a peer, at the first of `addrs` that answers, and say which it was.
    ///
    /// Waits for the handshake, so `Ok` means this node reached that station, agreed a
    /// protocol version and a show with it, and sent it whatever it was missing. What
    /// it does not wait for is the peer being registered, which happens a moment later
    /// on this manager's own loop — the connection is real either way.
    pub async fn connect_peer(
        &self,
        addrs: Vec<SocketAddr>,
        session_id: Uuid,
        show_id: Uuid,
    ) -> Result<SocketAddr, String> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(SyncCommand::ConnectPeer { addrs, session_id, show_id, reply: tx })
            .await
            .map_err(|_| "the sync manager has stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("the sync manager dropped the question".into()))
    }

    pub async fn set_leader(&self, node_id: NodeId) {
        let _ = self.0.send(SyncCommand::SetLeader(node_id)).await;
    }

    /// Ask one peer to publish its log at `level`, or `None` to withdraw the ask.
    ///
    /// Called whenever the browsers watching that peer change, including to none.
    /// A peer that is not connected is simply not told, which is correct: a raise
    /// that outlived its connection is a raise that no longer exists.
    pub async fn raise_peer_log(&self, node_id: NodeId, level: Option<pult_schema::ws::LogLevel>) {
        let _ = self.0.send(SyncCommand::RaisePeerLog { node_id, level }).await;
    }

    /// How many peers this station is connected to.
    pub async fn peer_count(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(SyncCommand::PeerCount { reply: tx }).await;
        rx.await.unwrap_or(0)
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
    /// Peers that finished their handshake, from either direction.
    connected_rx: mpsc::Receiver<(NodeId, PeerSender)>,
    connected_tx: mpsc::Sender<(NodeId, PeerSender)>,
    peers: HashMap<NodeId, PeerSender>,
    /// Everyone in the session, as last published by the leader. Includes this node.
    members: Vec<NodeId>,
    /// Told when this node takes over as leader, so the session can start advertising.
    promoted: Option<mpsc::Sender<NodeId>>,
    /// A handle to ourselves, for peer tasks to report their own exit.
    self_tx: mpsc::Sender<SyncCommand>,
    /// Link latencies, measured here and published by the station reporter.
    links: watch::Sender<PeerLinks>,
    /// This station's log, so each peer connection can publish from it. `None`
    /// where no subscriber was installed, which is every test that does not care.
    log: Option<crate::logging::LogHandle>,
}

impl SyncManager {
    /// Bind the sync port. Port 0 picks a free one, which is what the tests use;
    /// the bound address comes back so the caller can find out which.
    pub async fn bind(
        node_id: NodeId,
        sync_port: u16,
        engine: EngineHandle,
        log: Option<crate::logging::LogHandle>,
    ) -> Result<(Self, SyncHandle, SocketAddr)> {
        let listener = TcpListener::bind(format!("0.0.0.0:{sync_port}")).await?;
        let addr = listener.local_addr()?;
        info!("[sync] listening on {addr}");

        let (tx, rx) = mpsc::channel(64);
        let (connected_tx, connected_rx) = mpsc::channel(16);
        let (leader, _) = watch::channel(node_id);
        let (links, _) = watch::channel(PeerLinks::default());
        let mgr = SyncManager {
            node_id,
            log,
            leader,
            listener: Some(listener),
            engine,
            rx,
            connected_rx,
            connected_tx,
            peers: HashMap::new(),
            members: vec![node_id],
            promoted: None,
            self_tx: tx.clone(),
            links,
        };
        Ok((mgr, SyncHandle(tx), addr))
    }

    /// Watch the measured latency to every connected peer.
    pub fn peer_links(&self) -> watch::Receiver<PeerLinks> {
        self.links.subscribe()
    }

    /// Be told when this node is promoted to leader.
    pub fn on_promotion(&mut self, tx: mpsc::Sender<NodeId>) {
        self.promoted = Some(tx);
    }

    pub async fn run(mut self) {
        let Some(listener) = self.listener.take() else { return };
        let connected_tx = self.connected_tx.clone();
        let node_id = self.node_id;
        let engine = self.engine.clone();
        let log = self.log.clone();
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
                            log.clone(),
                            connected_tx.clone(),
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
                // A peer finished its handshake, dialled or accepted.
                connected = self.connected_rx.recv() => {
                    if let Some((peer_id, sender)) = connected {
                        info!("[sync] registered peer {}", peer_id.0);
                        self.peers.insert(peer_id, sender);
                        self.publish_members();
                    }
                }
            }
        }
    }

    async fn handle_command(&mut self, cmd: SyncCommand) {
        match cmd {
            SyncCommand::BroadcastSynced { path, value, clock, authorship } => {
                let msg = SyncMessage::SyncedBroadcast {
                    node_id: self.node_id,
                    path,
                    value,
                    clock,
                    authorship,
                };
                self.fan_out(msg);
            }
            SyncCommand::RaisePeerLog { node_id, level } => {
                if let Some(sender) = self.peers.get(&node_id) {
                    // Dropped rather than queued if the link is full: an ask about
                    // logging must never be what holds up a show's replication.
                    let _ = sender.0.try_send(SyncMessage::LogRaise { level });
                }
            }
            SyncCommand::ConnectPeer { addrs, session_id, show_id, reply } => {
                // Spawned, never awaited *here*. Dialling asks this node's own engine
                // for its clock and its missed operations, and the engine reaches
                // back into this command channel to broadcast every SYNCED write. Do
                // the handshake inline and that becomes a cycle: the engine blocks on
                // a full channel, the handshake blocks on the engine, and the loop
                // that would drain the channel is the one waiting for the handshake.
                //
                // The *caller* may await it, and does — `session.join` answers what
                // actually happened. That is safe for the same reason: this loop goes
                // on draining while the spawned task and the caller wait on each other.
                //
                // The connection comes back through the same channel an accepted peer
                // uses, so there is one place where a peer is registered.
                let node_id = self.node_id;
                let engine = self.engine.clone();
                let on_lost = self.self_tx.clone();
                let log = self.log.clone();
                let connected = self.connected_tx.clone();
                tokio::spawn(async move {
                    let Some((addr, stream)) = dial(&addrs).await else {
                        let _ = reply.send(Err(format!("nothing answered at any of {addrs:?}")));
                        return;
                    };
                    match spawn_outbound(
                        stream, node_id, session_id, show_id, engine, log, on_lost,
                    )
                    .await
                    {
                        Ok((peer_id, sender)) => {
                            let _ = connected.send((peer_id, sender)).await;
                            let _ = reply.send(Ok(addr));
                        }
                        Err(e) => {
                            let _ =
                                reply.send(Err(format!("{addr} did not finish the handshake: {e}")));
                        }
                    }
                });
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
            SyncCommand::PeerLatency { node_id, rtt, unanswered } => {
                self.links.send_modify(|links| {
                    links.insert(
                        node_id.0.to_string(),
                        PeerLink {
                            node_id: Some(node_id),
                            rtt_ms: Some(rtt.as_secs_f32() * 1000.0),
                            measured_at: Some(chrono::Utc::now()),
                            unanswered,
                        },
                    );
                });
            }
            SyncCommand::PeerLost(node_id) => {
                self.links.send_modify(|links| {
                    links.remove(&node_id.0.to_string());
                });
                if self.peers.remove(&node_id).is_some() {
                    info!("[sync] lost peer {}", node_id.0);
                }
                if node_id == *self.leader.borrow() {
                    self.elect_leader(node_id).await;
                } else if self.node_id == *self.leader.borrow() {
                    self.publish_members();
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
            self.fan_out(SyncMessage::LeaderChanged { new_leader_node_id: winner });
            self.publish_members();
            if let Some(tx) = &self.promoted {
                let _ = tx.send(winner).await;
            }
        } else {
            info!("[sync] leader {} is gone; {} takes over", lost.0, winner.0);
        }
    }

    /// Tell every peer who is in the session. Only the leader does this: two nodes
    /// publishing different lists is exactly what the list exists to prevent.
    fn publish_members(&mut self) {
        if self.node_id != *self.leader.borrow() {
            return;
        }
        let mut members: Vec<NodeId> = self.peers.keys().copied().collect();
        members.push(self.node_id);
        members.sort();
        self.members = members.clone();
        self.fan_out(SyncMessage::SessionMembers { members });
    }

    /// Hand a message to every peer's outbox.
    ///
    /// Never waits for one. A peer's outbox is drained by its own task, which applies
    /// what it receives through the engine — and the engine broadcasts back into this
    /// manager's command channel. Waiting here for a full outbox closes that into a
    /// deadlock: engine waits on the command channel, this loop waits on the outbox,
    /// the peer task waits on the engine, and nothing drains anything.
    ///
    /// A peer whose outbox is full is a peer that is not keeping up, and one that far
    /// behind is not synchronised whatever we do next. Dropping the *message* would
    /// leave the two nodes quietly disagreeing, so the connection goes instead: the
    /// peer reconnects and catches up from the oplog, which is a path that already
    /// exists and is already tested.
    fn fan_out(&mut self, msg: SyncMessage) {
        let mut dead = vec![];
        for (peer_id, sender) in &self.peers {
            match sender.0.try_send(msg.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!("[sync] peer {} is too far behind to keep up; dropping it", peer_id.0);
                    dead.push(*peer_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => dead.push(*peer_id),
            }
        }
        for id in dead {
            info!("[sync] removing disconnected peer {}", id.0);
            self.peers.remove(&id);
        }
    }
}

// ── Dialling ──────────────────────────────────────────────────────────────────

/// How long reaching a peer may take, across every address it offered.
///
/// Bounded, and bounded as a *total*, because somebody is waiting on the answer:
/// `session.join` reports what actually happened, and an answer that arrives after the
/// caller has given up is the same as no answer.
///
/// A station on the same network answers in milliseconds, and an address that is not
/// there refuses immediately. What this is for is the third case — a firewall dropping
/// the packet, where the operating system's own timeout is over a minute. Three seconds
/// is far longer than a local network ever needs and short enough to sit through.
const DIAL_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// The first of these addresses that answers, and which one it was.
///
/// A station advertises every address it has, and which of them reaches this machine is
/// not something either end can work out on its own: a link-local IPv6 address is
/// useless without the interface it was learned on, an IPv4 link-local one works only
/// on the segment, and loopback works only if the two are the same machine. So the
/// caller ranks them by how far they reach and this works down the list.
/// The budget is split evenly between them, so one address that neither answers nor
/// refuses cannot spend the whole of it before the one that works is tried.
async fn dial(addrs: &[SocketAddr]) -> Option<(SocketAddr, tokio::net::TcpStream)> {
    let each = DIAL_BUDGET / addrs.len().max(1) as u32;
    for addr in addrs {
        match tokio::time::timeout(each, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Some((*addr, stream)),
            Ok(Err(e)) => debug!("[sync] {addr} did not answer: {e}"),
            Err(_) => debug!("[sync] {addr} said nothing in {each:?}"),
        }
    }
    None
}

#[cfg(test)]
mod tests;
