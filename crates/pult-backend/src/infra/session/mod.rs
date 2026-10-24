use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::PathSegment,
    types::session::{DiscoveredSession, NearbyStation, SessionState},
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::sync::SyncHandle};

// ── SessionCommand ────────────────────────────────────────────────────────────

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
    /// Join the station at a sync address, with no advert to go by.
    JoinAt {
        addr: SocketAddr,
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

    /// Join the station at `addr` — its sync address — without having discovered it.
    ///
    /// The way in when mDNS is not: a venue switch that drops multicast, or a Mac whose
    /// Local Network permission was never given. Discovery is a convenience for finding
    /// a station; joining one is a TCP connection, and nothing about a hot spare should
    /// rest on the convenience.
    pub async fn join_at(&self, addr: SocketAddr) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(SessionCommand::JoinAt { addr, reply: tx }).await;
        rx.await.unwrap_or(Err("channel closed".into()))
    }
}

// ── SessionManager ────────────────────────────────────────────────────────────

const SERVICE_TYPE: &str = "_pult._tcp.local.";
/// Every station, in a session or not, at its HTTP port: what an invitation is sent to.
/// A service type of its own rather than more fields on the session's, because that
/// advert exists only while a station leads one, and everything that reads it takes
/// an advert to be a session somebody can join.
const STATION_SERVICE: &str = "_pult-station._tcp.local.";

/// What an mDNS full name was discovered as.
#[derive(Clone, Copy)]
enum Advert {
    Session(Uuid),
    Station(NodeId),
}

pub struct SessionManager {
    node_id: NodeId,
    sync_port: u16,
    engine: EngineHandle,
    sync: SyncHandle,
    rx: mpsc::Receiver<SessionCommand>,
    mdns_rx: mpsc::Receiver<ServiceEvent>,
    /// Canonical session state — pushed to the engine whenever it changes.
    state: SessionState,
    /// Where each discovered session might be reached, best first.
    ///
    /// A list rather than an address, because a station advertises every address it
    /// has and only some of them are any use from here — see [`reachable_at`]. Keyed
    /// by session_id for deduplication; the engine only ever sees the first one.
    discovered_addrs: HashMap<Uuid, Vec<SocketAddr>>,
    discovered_leader_ids: HashMap<Uuid, NodeId>,
    discovered_show_ids: HashMap<Uuid, Uuid>,
    mdns: ServiceDaemon,
    /// The discovered sessions and stations by mDNS full name, because a removal names
    /// only that — and the session list used to match a removal against an instance
    /// name built from the *session* id, when adverts are named by the node's, so a
    /// session that went away was never taken out of the list.
    advert_names: HashMap<String, Advert>,
    /// Where this station's own API answers, and what it has open, for its station
    /// advert.
    http_port: u16,
    show_name: String,
    /// The session id the station advert last said, so it is announced again only when
    /// that moves. Re-announcing on every state push would answer each discovery with
    /// an announcement, which is a discovery on every other station.
    announced: Option<Option<Uuid>>,
    /// The address this station advertises itself at, where the session was told a
    /// cable; `None` advertises each interface's own address on that interface.
    ///
    /// Not `infra::local_ipv4`, twice over. It asks the route table, so it answered with
    /// the *house* LAN on a console that has one beside a lighting network — the
    /// address a peer was told to dial for the show was decided by which cable reaches
    /// the internet — and on a lighting network with no route at all it answered
    /// loopback, so the peer was told to dial itself.
    advertise_at: Option<std::net::Ipv4Addr>,
    /// A handle to our own command channel, for the promotion bridge.
    self_tx: mpsc::Sender<SessionCommand>,
    /// Where "am I the leader" is read by everything that acts on it. Told the moment
    /// the session changes, not on the reporter's tick: see [`Self::push_state`].
    net: crate::infra::net::NetHandle,
}

impl SessionManager {
    pub fn new(
        node_id: NodeId,
        sync_port: u16,
        http_port: u16,
        show_name: String,
        engine: EngineHandle,
        sync: SyncHandle,
        net: crate::infra::net::NetHandle,
    ) -> (Self, SessionHandle) {
        let (tx, rx) = mpsc::channel(16);
        let (mdns_tx, mdns_rx) = mpsc::channel(64);

        let mdns = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => panic!("[session] cannot create mDNS daemon: {e}"),
        };
        // Told nothing is every interface, which is what this always did. Told a
        // cable that is not here is a fault on the row and every interface anyway —
        // a station that cannot find its peers at all is a session that silently
        // stops existing, and unlike an output there is no wrong wire to put bytes
        // on: the worst case is being found somewhere as well as here.
        let address = net
            .bind(
                pult_schema::types::network::NetService::Session,
                "session discovery",
                net.prefs().session.as_deref(),
            )
            .unwrap_or(None);
        let advertise_at = crate::infra::net::restrict_mdns(&mdns, address);

        let browse_receiver = match mdns.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                warn!("[session] mDNS browse failed: {e}");
                return (
                    SessionManager {
                        node_id, sync_port, engine, sync, rx, mdns_rx,
                        state: SessionState { node_id: Some(node_id), ..Default::default() },
                        discovered_addrs: HashMap::new(),
                        discovered_leader_ids: HashMap::new(),
                        discovered_show_ids: HashMap::new(),
                        mdns,
                        advert_names: HashMap::new(),
                        http_port,
                        show_name,
                        announced: None,
                        advertise_at,
                        self_tx: tx.clone(),
                        net,
                    },
                    SessionHandle(tx),
                );
            }
        };

        let stations_tx = mdns_tx.clone();
        std::thread::spawn(move || {
            while let Ok(event) = browse_receiver.recv() {
                if mdns_tx.blocking_send(event).is_err() { break; }
            }
        });
        match mdns.browse(STATION_SERVICE) {
            Ok(stations) => {
                std::thread::spawn(move || {
                    while let Ok(event) = stations.recv() {
                        if stations_tx.blocking_send(event).is_err() { break; }
                    }
                });
            }
            Err(e) => warn!("[session] mDNS browse for stations failed: {e}"),
        }

        (
            SessionManager {
                node_id, sync_port, engine, sync, rx, mdns_rx,
                state: SessionState { node_id: Some(node_id), ..Default::default() },
                discovered_addrs: HashMap::new(),
                discovered_leader_ids: HashMap::new(),
                discovered_show_ids: HashMap::new(),
                mdns,
                advert_names: HashMap::new(),
                http_port,
                show_name,
                announced: None,
                advertise_at,
                self_tx: tx.clone(),
                net,
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
        // Say who this node is before anything happens to it. Nothing else will:
        // state is pushed on change, and this node's own id never changes.
        self.push_state().await;
        self.announce_station();

        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    match cmd {
                        // The channel closing and being told to stop are the same
                        // thing here: this station is going away, and what it has to
                        // do about that is stop claiming to be somewhere.
                        None | Some(SessionCommand::Stop) => {
                            self.stop_advertising();
                            break;
                        }
                        Some(cmd) => {
                            self.handle_command(cmd).await;
                            self.announce_station();
                        }
                    }
                }
                event = self.mdns_rx.recv() => {
                    if let Some(event) = event { self.handle_mdns_event(event).await; }
                }
            }
        }
        // The daemon runs on a thread of its own, so dropping the manager is not
        // enough to stop it. A console that opens three shows in a row would
        // otherwise be three responders on the network, two of them answering for
        // ports nothing is listening on.
        let _ = self.mdns.unregister(&self.station_fullname());
        if let Err(e) = self.mdns.shutdown() {
            debug!("[session] mDNS shutdown: {e}");
        }
    }

    /// Push current session state to the engine so all connected frontends see it.
    ///
    /// And tell the station's standing at once. The reporter works it out too, but on
    /// its own two-second tick and only after the machine's first probe, which on a
    /// slow disk is seconds: a spare that had joined went on believing it led for that
    /// long, and played every timeline beside the real leader.
    async fn push_state(&self) {
        self.net.set_leader(!self.state.is_follower);
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
                let Some(addrs) = self.discovered_addrs.get(&session_id).cloned() else {
                    let _ = reply.send(Err(format!("session {session_id} not in discovered list")));
                    return;
                };
                let show_id = self.discovered_show_ids.get(&session_id).copied().unwrap_or_default();
                let leader = self.discovered_leader_ids.get(&session_id).copied().unwrap_or(self.node_id);

                // Every address it advertised that could be dialed from here, in the
                // order they are worth trying. The sync side works down the list,
                // because "the address it gave" is several addresses and which of them
                // reaches this machine is not something either end can know on its own.
                //
                // **Waited for.** A join that answers yes and did not is worse than one
                // that takes a moment to answer: the console then shows a session it is
                // not in, the panel it opens is empty for a reason nothing states, and
                // the only trace of the truth is a line in a log. Both callers already
                // have somewhere to put a failure — a toast in the Sessions panel, a
                // non-zero exit in `demo-session.mjs` — and until now neither could
                // ever fire.
                //
                // Safe to wait on despite the deadlock this actor's opposite number
                // guards against: the sync manager spawns the dial and goes on
                // draining, so nothing this waits for is waiting on it.
                let addr = match self.sync.connect_peer(addrs, session_id, show_id).await {
                    Ok(addr) => addr,
                    Err(why) => {
                        warn!("[session] could not join session {session_id}: {why}");
                        let _ = reply.send(Err(why));
                        return;
                    }
                };

                self.sync.set_leader(leader).await;
                self.state.is_advertising = false;
                self.state.is_follower = true;
                self.state.session_id = Some(session_id);
                self.push_state().await;
                info!("[session] joined session {session_id} at {addr}");
                let _ = reply.send(Ok(()));
            }

            SessionCommand::JoinAt { addr, reply } => {
                // No advert, so no session id and no show id to offer: a fresh id
                // names this session from here on, and the show is whatever the
                // handshake agrees. The node that answers is the one being joined,
                // and so the leader.
                let session_id = Uuid::new_v4();
                match self.sync.dial_peer(vec![addr], session_id, Uuid::nil()).await {
                    Ok((addr, leader)) => {
                        self.sync.set_leader(leader).await;
                        self.state.is_advertising = false;
                        self.state.is_follower = true;
                        self.state.session_id = Some(session_id);
                        self.push_state().await;
                        info!("[session] joined the station at {addr} by its address");
                        let _ = reply.send(Ok(()));
                    }
                    Err(why) => {
                        warn!("[session] could not join the station at {addr}: {why}");
                        let _ = reply.send(Err(why));
                    }
                }
            }

            SessionCommand::Leave => {
                if self.state.is_advertising {
                    self.stop_advertising();
                } else if self.state.is_follower {
                    if let Some(id) = self.state.session_id {
                        info!("[session] leaving session {id}");
                    }
                    self.sync.disconnect_all().await;
                }
                self.state = SessionState {
                    node_id: Some(self.node_id),
                    discovered: self.state.discovered.clone(),
                    ..Default::default()
                };
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

            // Handled in `run`, which is where breaking out of the loop is possible.
            SessionCommand::Stop => {}
        }
    }

    fn station_fullname(&self) -> String {
        format!("pult-station-{}.{STATION_SERVICE}", &self.node_id.0.to_string()[..8])
    }

    /// Say this station is here, at its HTTP port, and which session it is in —
    /// again whenever that moves, so a console listing who could be invited does not
    /// offer the one that already came.
    fn announce_station(&mut self) {
        if self.announced == Some(self.state.session_id) {
            return;
        }
        let instance_name = format!("pult-station-{}", &self.node_id.0.to_string()[..8]);
        let hostname = format!("{}.local.", gethostname());
        let mut props = HashMap::new();
        props.insert("node_id".to_string(), self.node_id.0.to_string());
        props.insert("hostname".to_string(), gethostname());
        props.insert("show_name".to_string(), self.show_name.clone());
        props.insert(
            "session_id".to_string(),
            self.state.session_id.map(|id| id.to_string()).unwrap_or_default(),
        );
        let info = match self.advertise_at {
            Some(ip) => ServiceInfo::new(STATION_SERVICE, &instance_name, &hostname, IpAddr::V4(ip), self.http_port, props),
            None => ServiceInfo::new(STATION_SERVICE, &instance_name, &hostname, (), self.http_port, props)
                .map(ServiceInfo::enable_addr_auto),
        };
        match info.map(|info| self.mdns.register(info)) {
            Ok(Ok(())) => self.announced = Some(self.state.session_id),
            Ok(Err(e)) => warn!("[session] mDNS register of this station failed: {e}"),
            Err(e) => warn!("[session] ServiceInfo::new for this station failed: {e}"),
        }
    }

    /// A station advert, resolved: listed as nearby unless it is this one.
    async fn station_resolved(&mut self, info: &ServiceInfo) {
        let props = info.get_properties();
        let Some(node_id) = props.get("node_id").and_then(|v| v.val_str().parse::<Uuid>().ok()).map(NodeId) else {
            return;
        };
        if node_id == self.node_id {
            return;
        }
        let Some(http_addr) = reachable_at(info.get_addresses().iter().copied(), info.get_port()).first().copied() else {
            return;
        };
        let station = NearbyStation {
            node_id,
            hostname: props.get("hostname").map(|v| v.val_str().to_string()).unwrap_or_default(),
            http_addr: http_addr.to_string(),
            show_name: props.get("show_name").map(|v| v.val_str().to_string()).unwrap_or_default(),
            session_id: props.get("session_id").and_then(|v| v.val_str().parse::<Uuid>().ok()),
        };
        self.advert_names.insert(info.get_fullname().to_string(), Advert::Station(node_id));
        match self.state.nearby.iter_mut().find(|s| s.node_id == node_id) {
            Some(known) if *known == station => return,
            Some(known) => *known = station,
            None => {
                info!("[session] station {} on the network at {http_addr}", station.hostname);
                self.state.nearby.push(station);
            }
        }
        self.push_state().await;
    }

    /// Take this node's service off the network.
    ///
    /// Both an operator leaving a session and the station shutting down have to do
    /// this, and the second is why the daemon is shut down too: opening a show is
    /// this station stopping and another one starting in its place, and a browser
    /// that went on finding the old one at the old port would offer to join a console
    /// that is no longer there.
    fn stop_advertising(&mut self) {
        if self.state.is_advertising {
            let instance_name = format!("pult-{}", &self.node_id.0.to_string()[..8]);
            let fullname = format!("{instance_name}.{SERVICE_TYPE}");
            if let Err(e) = self.mdns.unregister(&fullname) {
                debug!("[session] mDNS unregister: {e}");
            }
            info!("[session] stopped advertising");
        }
    }

    /// Register this node as the session's mDNS service, so newcomers find it here.
    async fn advertise(&mut self, session_id: Uuid, show_id: Uuid, show_name: String) {
        let instance_name = format!("pult-{}", &self.node_id.0.to_string()[..8]);
        let hostname = format!("{}.local.", gethostname());

        let mut props = HashMap::new();
        props.insert("session_id".to_string(), session_id.to_string());
        props.insert("show_id".to_string(), show_id.to_string());
        props.insert("show_name".to_string(), show_name);
        props.insert("node_id".to_string(), self.node_id.0.to_string());
        props.insert("leader_node_id".to_string(), self.node_id.0.to_string());

        let info = match self.advertise_at {
            Some(ip) => ServiceInfo::new(SERVICE_TYPE, &instance_name, &hostname, IpAddr::V4(ip), self.sync_port, props),
            None => ServiceInfo::new(SERVICE_TYPE, &instance_name, &hostname, (), self.sync_port, props)
                .map(ServiceInfo::enable_addr_auto),
        };
        match info {
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
            ServiceEvent::ServiceResolved(info) if info.get_fullname().ends_with(STATION_SERVICE) => {
                self.station_resolved(&info).await;
            }
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

                    let reachable =
                        reachable_at(info.get_addresses().iter().copied(), info.get_port());
                    let Some(sync_addr) = reachable.first().copied() else {
                        // Nothing it advertised can be dialed from here. Not listed,
                        // because a session in the list is one an operator can press
                        // Join on — but said out loud, because a station that is
                        // plainly there and cannot be reached is worth a line in the
                        // log rather than silence.
                        warn!(
                            "[session] {show_name} is advertising {session_id} at nothing \
                             reachable from here: {:?}",
                            info.get_addresses()
                        );
                        return;
                    };

                    info!(
                        "[session] discovered session {session_id} ({show_name}) at {sync_addr}{}",
                        match reachable.len() {
                            1 => String::new(),
                            n => format!(" (and {} other{})", n - 1, if n == 2 { "" } else { "s" }),
                        }
                    );

                    self.advert_names.insert(info.get_fullname().to_string(), Advert::Session(session_id));
                    self.discovered_addrs.insert(session_id, reachable);
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

            ServiceEvent::ServiceRemoved(_ty, fullname) => match self.advert_names.remove(&fullname) {
                Some(Advert::Session(id)) => {
                    self.state.discovered.retain(|d| d.session_id != id);
                    self.discovered_addrs.remove(&id);
                    debug!("[session] session {id} went away");
                    self.push_state().await;
                }
                Some(Advert::Station(node)) => {
                    self.state.nearby.retain(|s| s.node_id != node);
                    debug!("[session] station {} went away", node.0);
                    self.push_state().await;
                }
                None => {}
            },

            _ => {}
        }
    }
}

// ── Where a discovered station can actually be reached ────────────────────────

/// The addresses a discovered station can be dialed at, best first.
///
/// mDNS hands over a `HashSet`, so "the first address" is not the first the other
/// station advertised — it is whatever the hash order happens to give, and it can be a
/// different one on the next run of the same two consoles. Something has to choose.
/// This chooses, and it chooses the same way twice.
///
/// The order is by how far an address reaches. An ordinary IPv4 or IPv6 address works
/// from anywhere on the network. An IPv4 link-local address (`169.254/16`) works only
/// on the segment, which is a crossover cable or a room with no DHCP — real, and worth
/// trying after the addresses that are not. Loopback works only for a second station on
/// this machine, which `scripts/demo.sh --two` is and a rig never is.
///
/// IPv4 before IPv6 where both are offered, because a console advertises an IPv4
/// address for itself and preferring the family it named keeps two
/// stations agreeing about which path they are on.
///
/// **An IPv6 link-local address is dropped rather than ranked last**, because it cannot
/// be dialed at all: `fe80::/10` is only meaningful together with the interface it was
/// learned on, and mDNS does not say which that was — so the `SocketAddr` built from it
/// has no scope id and the connect fails with "No route to host". That is the bug this
/// function exists for. Keeping it as a last resort would only spend a timeout before
/// reaching the address that was going to work anyway.
fn reachable_at(addresses: impl IntoIterator<Item = IpAddr>, port: u16) -> Vec<SocketAddr> {
    let mut ranked: Vec<(u8, IpAddr)> =
        addresses.into_iter().filter_map(|ip| reach(&ip).map(|rank| (rank, ip))).collect();
    // By address as well as by rank, so a station with two equally good addresses is
    // dialed at the same one every time and the log is worth reading.
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked.dedup();
    ranked.into_iter().map(|(_, ip)| SocketAddr::new(ip, port)).collect()
}

/// How far an address reaches, lower being further. `None` cannot be dialed at all.
fn reach(ip: &IpAddr) -> Option<u8> {
    match ip {
        // Not destinations: nothing is listening at "any address", and a console does
        // not sync to a broadcast or a group.
        IpAddr::V4(v4) if v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() => None,
        IpAddr::V6(v6) if v6.is_unspecified() || v6.is_multicast() => None,
        // `fe80::/10`. See above: undialable without the interface, which is not on offer.
        IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80 => None,

        IpAddr::V4(v4) if v4.is_loopback() => Some(3),
        IpAddr::V6(v6) if v6.is_loopback() => Some(3),
        IpAddr::V4(v4) if v4.is_link_local() => Some(2),
        IpAddr::V6(_) => Some(1),
        IpAddr::V4(_) => Some(0),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn gethostname() -> String {
    hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "pult-node".to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("an address")
    }

    fn at(addresses: &[&str]) -> Vec<String> {
        reachable_at(addresses.iter().map(|s| ip(s)), 7701)
            .into_iter()
            .map(|a| a.to_string())
            .collect()
    }

    /// The bug, in one line. A station on macOS advertises an ordinary IPv4 address and
    /// a link-local IPv6 one; mDNS hands them over as a `HashSet`, so taking "the
    /// first" took whichever the hash order gave — and half the time that was the
    /// `fe80::` one, which cannot be dialed without the interface it was learned on.
    /// Two consoles found each other and never synced.
    #[test]
    fn a_link_local_ipv6_address_is_not_offered_at_all() {
        assert_eq!(
            at(&["fe80::3a:a030:7b26:cc32", "192.168.11.184"]),
            vec!["192.168.11.184:7701"],
        );
        assert!(at(&["fe80::1", "fe80::2"]).is_empty(), "there is nowhere to dial");
    }

    /// And it is the same answer twice, whatever order the set yields — which is the
    /// other half of what went wrong: the old code could pick differently on two runs
    /// of the same two consoles, so the failure looked intermittent.
    #[test]
    fn the_same_addresses_give_the_same_answer_whichever_order_they_arrive_in() {
        let forwards = at(&["10.0.0.5", "10.0.0.9", "2001:db8::1", "127.0.0.1"]);
        let backwards = at(&["127.0.0.1", "2001:db8::1", "10.0.0.9", "10.0.0.5"]);
        assert_eq!(forwards, backwards);
        assert_eq!(forwards.first().map(String::as_str), Some("10.0.0.5:7701"));
    }

    /// Ordered by how far each one reaches: the network, then the segment, then this
    /// machine. Every one of them is kept, because a rank is a guess about which will
    /// work and the dialler tries the next when a guess is wrong.
    #[test]
    fn addresses_are_ordered_by_how_far_they_reach() {
        assert_eq!(
            at(&["127.0.0.1", "169.254.7.7", "2001:db8::1", "10.0.0.5"]),
            vec!["10.0.0.5:7701", "[2001:db8::1]:7701", "169.254.7.7:7701", "127.0.0.1:7701"],
        );
    }

    /// A second station on this machine is `scripts/demo.sh --two`, and loopback is the
    /// only way to reach it. Worth keeping, and worth keeping last.
    #[test]
    fn loopback_is_a_last_resort_rather_than_no_resort() {
        assert_eq!(at(&["127.0.0.1"]), vec!["127.0.0.1:7701"]);
        assert_eq!(at(&["::1"]), vec!["[::1]:7701"]);
    }

    /// Addresses that are not destinations at all. A console does not sync to a
    /// broadcast, and nothing is listening at "any address".
    #[test]
    fn an_address_nothing_could_be_listening_at_is_dropped() {
        assert!(at(&["0.0.0.0", "255.255.255.255", "224.0.0.251", "::", "ff02::fb"]).is_empty());
    }
}