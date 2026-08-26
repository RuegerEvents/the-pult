use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::PathSegment,
    types::session::{DiscoveredSession, SessionState},
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::sync::SyncHandle};

// ── SessionCommand ────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "Stop has no caller until the server shuts down gracefully")]
pub enum SessionCommand {
    /// This node has become the session leader after the old one disappeared.
    Promote,
    Create {
        show_name: String,
        show_id: Uuid,
        reply: oneshot::Sender<Uuid>,
    },
    Join {
        session_id: Uuid,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Leave,
    Stop,
}

// ── SessionHandle ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SessionHandle(pub mpsc::Sender<SessionCommand>);

impl SessionHandle {
    pub async fn create_session(&self, show_name: String, show_id: Uuid) -> Option<Uuid> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(SessionCommand::Create { show_name, show_id, reply: tx }).await;
        rx.await.ok()
    }

    pub async fn join_session(&self, session_id: Uuid) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(SessionCommand::Join { session_id, reply: tx }).await;
        rx.await.unwrap_or(Err("channel closed".into()))
    }
}

// ── SessionManager ────────────────────────────────────────────────────────────

const SERVICE_TYPE: &str = "_pult._tcp.local.";

pub struct SessionManager {
    node_id: NodeId,
    sync_port: u16,
    engine: EngineHandle,
    sync: SyncHandle,
    rx: mpsc::Receiver<SessionCommand>,
    mdns_rx: mpsc::Receiver<ServiceEvent>,
    /// Canonical session state — pushed to the engine whenever it changes.
    state: SessionState,
    /// Internal map keyed by session_id for deduplication; only sync_addr is needed by the engine.
    discovered_addrs: HashMap<Uuid, SocketAddr>,
    discovered_leader_ids: HashMap<Uuid, NodeId>,
    discovered_show_ids: HashMap<Uuid, Uuid>,
    mdns: ServiceDaemon,
    /// A handle to our own command channel, for the promotion bridge.
    self_tx: mpsc::Sender<SessionCommand>,
}

impl SessionManager {
    pub fn new(node_id: NodeId, sync_port: u16, engine: EngineHandle, sync: SyncHandle) -> (Self, SessionHandle) {
        let (tx, rx) = mpsc::channel(16);
        let (mdns_tx, mdns_rx) = mpsc::channel(64);

        let mdns = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => panic!("[session] cannot create mDNS daemon: {e}"),
        };

        let browse_receiver = match mdns.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                warn!("[session] mDNS browse failed: {e}");
                return (
                    SessionManager {
                        node_id, sync_port, engine, sync, rx, mdns_rx,
                        state: SessionState::default(),
                        discovered_addrs: HashMap::new(),
                        discovered_leader_ids: HashMap::new(),
                        discovered_show_ids: HashMap::new(),
                        mdns,
                        self_tx: tx.clone(),
                    },
                    SessionHandle(tx),
                );
            }
        };

        std::thread::spawn(move || {
            while let Ok(event) = browse_receiver.recv() {
                if mdns_tx.blocking_send(event).is_err() { break; }
            }
        });

        (
            SessionManager {
                node_id, sync_port, engine, sync, rx, mdns_rx,
                state: SessionState::default(),
                discovered_addrs: HashMap::new(),
                discovered_leader_ids: HashMap::new(),
                discovered_show_ids: HashMap::new(),
                mdns,
                self_tx: tx.clone(),
            },
            SessionHandle(tx),
        )
    }

    /// A channel the sync layer can use to say this node is now the leader.
    pub fn promotion_sender(&self) -> mpsc::Sender<NodeId> {
        let tx = self.self_tx.clone();
        let (promo_tx, mut promo_rx) = mpsc::channel::<NodeId>(4);
        tokio::spawn(async move {
            while promo_rx.recv().await.is_some() {
                let _ = tx.send(SessionCommand::Promote).await;
            }
        });
        promo_tx
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    match cmd {
                        None | Some(SessionCommand::Stop) => break,
                        Some(cmd) => self.handle_command(cmd).await,
                    }
                }
                event = self.mdns_rx.recv() => {
                    if let Some(event) = event { self.handle_mdns_event(event).await; }
                }
            }
        }
    }

    /// Push current session state to the engine so all connected frontends see it.
    async fn push_state(&self) {
        if let Ok(json) = serde_json::to_value(&self.state) {
            let path = vec![PathSegment::Key("session".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }

    async fn handle_command(&mut self, cmd: SessionCommand) {
        match cmd {
            SessionCommand::Create { show_name, show_id, reply } => {
                let session_id = Uuid::new_v4();
                self.advertise(session_id, show_id, show_name).await;
                if self.state.is_advertising {
                    self.state.is_follower = false;
                    self.push_state().await;
                }
                let _ = reply.send(session_id);
            }

            SessionCommand::Join { session_id, reply } => {
                if let Some(addr) = self.discovered_addrs.get(&session_id).copied() {
                    let show_id = self.discovered_show_ids.get(&session_id).copied().unwrap_or_default();
                    let leader = self.discovered_leader_ids.get(&session_id).copied().unwrap_or(self.node_id);
                    self.sync.connect_peer(addr, session_id, show_id).await;
                    self.sync.set_leader(leader).await;
                    self.state.is_advertising = false;
                    self.state.is_follower = true;
                    self.state.session_id = Some(session_id);
                    self.push_state().await;
                    info!("[session] joined session {session_id} at {addr}");
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(format!("session {session_id} not in discovered list")));
                }
            }

            SessionCommand::Leave => {
                if self.state.is_advertising {
                    let instance_name = format!("pult-{}", &self.node_id.0.to_string()[..8]);
                    let fullname = format!("{instance_name}.{SERVICE_TYPE}");
                    if let Err(e) = self.mdns.unregister(&fullname) {
                        debug!("[session] mDNS unregister: {e}");
                    }
                    info!("[session] stopped advertising");
                } else if self.state.is_follower {
                    if let Some(id) = self.state.session_id {
                        info!("[session] leaving session {id}");
                    }
                    self.sync.disconnect_all().await;
                }
                self.state = SessionState { discovered: self.state.discovered.clone(), ..Default::default() };
                self.push_state().await;
            }

            SessionCommand::Promote => {
                if self.state.is_advertising || self.state.session_id.is_none() {
                    return; // already leading, or not in a session at all
                }
                let session_id = self.state.session_id.unwrap_or_else(Uuid::new_v4);
                let show_id = self.discovered_show_ids.get(&session_id).copied().unwrap_or_default();
                let show_name = self
                    .state
                    .discovered
                    .iter()
                    .find(|d| d.session_id == session_id)
                    .map(|d| d.show_name.clone())
                    .unwrap_or_else(|| "Untitled Show".to_string());

                info!("[session] promoted to leader of session {session_id}");
                self.advertise(session_id, show_id, show_name).await;
                self.state.is_follower = false;
                self.push_state().await;
            }

            SessionCommand::Stop => {}
        }
    }

    /// Register this node as the session's mDNS service, so newcomers find it here.
    async fn advertise(&mut self, session_id: Uuid, show_id: Uuid, show_name: String) {
        let instance_name = format!("pult-{}", &self.node_id.0.to_string()[..8]);
        let hostname = format!("{}.local.", gethostname());
        let ip = local_ipv4();

        let mut props = HashMap::new();
        props.insert("session_id".to_string(), session_id.to_string());
        props.insert("show_id".to_string(), show_id.to_string());
        props.insert("show_name".to_string(), show_name);
        props.insert("node_id".to_string(), self.node_id.0.to_string());
        props.insert("leader_node_id".to_string(), self.node_id.0.to_string());

        match ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &hostname,
            IpAddr::V4(ip),
            self.sync_port,
            props,
        ) {
            Ok(info) => match self.mdns.register(info) {
                Ok(()) => {
                    info!("[session] advertising session {session_id}");
                    self.state.is_advertising = true;
                    self.state.session_id = Some(session_id);
                }
                Err(e) => warn!("[session] mDNS register failed: {e}"),
            },
            Err(e) => warn!("[session] ServiceInfo::new failed: {e}"),
        }
    }

    async fn handle_mdns_event(&mut self, event: ServiceEvent) {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let props = info.get_properties();
                let session_id = props.get("session_id").and_then(|v| v.val_str().parse::<Uuid>().ok());
                let show_id = props.get("show_id").and_then(|v| v.val_str().parse::<Uuid>().ok());
                let show_name = props.get("show_name").map(|v| v.val_str().to_string()).unwrap_or_default();
                let node_id = props.get("node_id").and_then(|v| v.val_str().parse::<Uuid>().ok()).map(NodeId);
                let leader_node_id = props.get("leader_node_id").and_then(|v| v.val_str().parse::<Uuid>().ok()).map(NodeId);

                if let (Some(session_id), Some(show_id), Some(node_id), Some(leader_node_id)) =
                    (session_id, show_id, node_id, leader_node_id)
                {
                    if node_id == self.node_id { return; }

                    let sync_ip = info.get_addresses().iter().next().copied().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
                    let sync_addr = SocketAddr::new(sync_ip, info.get_port());

                    info!("[session] discovered session {session_id} ({show_name}) at {sync_addr}");

                    self.discovered_addrs.insert(session_id, sync_addr);
                    self.discovered_show_ids.insert(session_id, show_id);
                    self.discovered_leader_ids.insert(session_id, leader_node_id);

                    // Upsert into discovered list
                    if let Some(existing) = self.state.discovered.iter_mut().find(|d| d.session_id == session_id) {
                        existing.show_name = show_name;
                        existing.sync_addr = sync_addr.to_string();
                    } else {
                        self.state.discovered.push(DiscoveredSession {
                            session_id,
                            show_id,
                            show_name,
                            sync_addr: sync_addr.to_string(),
                        });
                    }
                    self.push_state().await;
                }
            }

            ServiceEvent::ServiceRemoved(_ty, fullname) => {
                let before = self.state.discovered.len();
                self.state.discovered.retain(|d| {
                    let instance = format!("pult-{}", &d.session_id.to_string()[..8]);
                    !fullname.contains(&instance)
                });
                if self.state.discovered.len() != before {
                    debug!("[session] removed session from discovered list");
                    self.push_state().await;
                }
            }

            _ => {}
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn local_ipv4() -> Ipv4Addr {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok();
    socket
        .and_then(|s| s.connect("8.8.8.8:80").ok().map(|_| s))
        .and_then(|s| s.local_addr().ok())
        .and_then(|a| match a.ip() { IpAddr::V4(v4) => Some(v4), _ => None })
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

fn gethostname() -> String {
    hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "pult-node".to_string())
}
