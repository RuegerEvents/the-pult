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

pub mod remembered;

/// How often a station that came up remembering a session tries to get back into it.
const REJOIN_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

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
    /// A page has the stations on the network on screen, or no longer has.
    WatchNearby { session: Uuid, on: bool },
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
    /// This page is showing the stations on the network, or has stopped. Browsing for
    /// them runs only while some page is.
    pub async fn watch_nearby(&self, session: Uuid, on: bool) {
        let _ = self.0.send(SessionCommand::WatchNearby { session, on }).await;
    }

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
/// How often a station on the network is asked again whether it is still there.
const PROBE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// How long it has to answer. A station on the same LAN answers in a millisecond; one
/// that takes a second is not one an invitation would reach either.
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// What an mDNS full name was discovered as.
#[derive(Clone, Copy)]
enum Advert {
    Session(Uuid),
    Station(NodeId),
}

pub struct SessionManager {
    /// Whether joins are written down and gone back to after a restart
    /// (`Config::rejoin`). Off in tests, which start stations by the dozen.
    remembers: bool,
    /// The leader to get back to, while this station is not yet in a session.
    rejoin: Option<SocketAddr>,
    /// The leader to hand the lead back to: set when this station was promoted out of
    /// a session it had joined, cleared once it is that leader's follower again.
    handback: Option<SocketAddr>,
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
    announced: Option<(Option<Uuid>, bool)>,
    /// Every station advert resolved, whether or not it answers. `state.nearby` is the
    /// ones that did at their last probe: an advert says a station was there, and one
    /// that stopped without withdrawing it — a crash, a pulled cable, a test run —
    /// goes on being advertised until its record expires, which is the best part of an
    /// hour. Listing those offered an Invite to a dozen consoles that did not exist.
    candidates: HashMap<NodeId, NearbyStation>,
    /// The pages showing the stations on the network. Browsing for them runs while
    /// there is one and not otherwise: a view is asked for, never published, and a
    /// station browsing for ever answered every other station's announcements for
    /// ever — a few dozen of them in one test run starved each other's runtimes.
    watchers: std::collections::HashSet<Uuid>,
    browsing: bool,
    /// Where the station browse's events are fed, once it is started.
    station_events: mpsc::Sender<ServiceEvent>,
    probes_tx: mpsc::Sender<(NodeId, String, bool)>,
    probes_rx: mpsc::Receiver<(NodeId, String, bool)>,
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
    /// Ticks when this machine's addresses move, which a daemon does not survive — see
    /// [`Self::renew_mdns`].
    moved: tokio::sync::watch::Receiver<u64>,
    /// The session advert as last registered, so a renewed daemon can say it again.
    session_advert: Option<ServiceInfo>,
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
        remembers: bool,
    ) -> (Self, SessionHandle) {
        let rejoin = if remembers { remembered::load(node_id) } else { None };
        let (tx, rx) = mpsc::channel(16);
        let (mdns_tx, mdns_rx) = mpsc::channel(64);
        let (probes_tx, probes_rx) = mpsc::channel(64);

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
                        remembers, rejoin, handback: None,
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
                        candidates: HashMap::new(),
                        watchers: Default::default(),
                        browsing: false,
                        station_events: mdns_tx.clone(),
                        probes_tx,
                        probes_rx,
                        advertise_at,
                        self_tx: tx.clone(),
                        moved: net.address_changes(),
                        session_advert: None,
                        net,
                    },
                    SessionHandle(tx),
                );
            }
        };

        let station_events = mdns_tx.clone();
        std::thread::spawn(move || {
            while let Ok(event) = browse_receiver.recv() {
                if mdns_tx.blocking_send(event).is_err() { break; }
            }
        });

        (
            SessionManager {
                remembers, rejoin, handback: None,
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
                candidates: HashMap::new(),
                watchers: Default::default(),
                browsing: false,
                station_events,
                probes_tx,
                probes_rx,
                advertise_at,
                self_tx: tx.clone(),
                moved: net.address_changes(),
                session_advert: None,
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
        let mut reprobe = tokio::time::interval(PROBE_EVERY);
        // The first tick is at once: a station that remembers a leader tries it as soon
        // as it is up, and then every REJOIN_EVERY until it is in a session again.
        let mut rejoin_tick = tokio::time::interval(REJOIN_EVERY);
        let mut handback_tick = tokio::time::interval(REJOIN_EVERY);
        if let Some(addr) = self.rejoin {
            info!("[session] going back to the session last joined, at {addr}");
        }

        loop {
            tokio::select! {
                _ = rejoin_tick.tick(), if self.rejoin.is_some() => self.try_rejoin().await,
                _ = handback_tick.tick(), if self.handback.is_some() => self.try_handback().await,
                _ = reprobe.tick(), if self.browsing => {
                    for station in self.candidates.values() {
                        self.probe(station);
                    }
                }
                Some((node, http_addr, answered)) = self.probes_rx.recv() => {
                    self.probed(node, http_addr, answered).await;
                }
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
                Ok(()) = self.moved.changed() => self.renew_mdns(),
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

    /// One attempt at the remembered leader. Stops trying once this station is in a
    /// session by any route, so a join somebody made by hand in the meantime wins.
    async fn try_rejoin(&mut self) {
        if self.state.is_follower || self.state.is_advertising {
            self.rejoin = None;
            return;
        }
        let Some(addr) = self.rejoin else { return };
        let (reply, answer) = oneshot::channel();
        self.handle_command(SessionCommand::JoinAt { addr, reply }).await;
        match answer.await {
            Ok(Ok(())) => self.rejoin = None,
            _ => debug!("[session] {addr} did not answer yet; trying again in {REJOIN_EVERY:?}"),
        }
    }

    /// Give the lead back to the leader this station was promoted away from, if it is
    /// answering again.
    ///
    /// A plain TCP connect first, so the ten-second tick costs a refused connection and
    /// nothing else while that machine is still down. Then the advert goes, and this
    /// station joins it the way `session.joinAt` would; if the join fails after all, it
    /// takes the lead up again at once rather than leaving the rig with nobody driving.
    async fn try_handback(&mut self) {
        let Some(addr) = self.handback else { return };
        if !self.state.is_advertising {
            // No longer leading, by whatever route: nothing to hand back.
            self.handback = None;
            return;
        }
        let reachable = matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                tokio::net::TcpStream::connect(addr)
            )
            .await,
            Ok(Ok(_))
        );
        if !reachable {
            return;
        }
        info!("[session] {addr} is back; handing the lead back to it");
        self.stop_advertising();
        self.state.is_advertising = false;
        let (reply, answer) = oneshot::channel();
        self.handle_command(SessionCommand::JoinAt { addr, reply }).await;
        match answer.await {
            Ok(Ok(())) => self.handback = None,
            _ => {
                warn!("[session] could not hand the lead back to {addr}; leading on");
                self.state.is_follower = true;
                self.handle_command(SessionCommand::Promote).await;
            }
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
                // A station starting its own session is nobody's follower any more.
                self.rejoin = None;
                self.handback = None;
                if self.remembers {
                    remembered::forget(self.node_id);
                }
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
                if self.remembers {
                    remembered::remember(self.node_id, addr);
                }
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
                        if self.remembers {
                            remembered::remember(self.node_id, addr);
                        }
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
                // Deliberate, so not gone back to after a restart either.
                self.rejoin = None;
                self.handback = None;
                if self.remembers {
                    remembered::forget(self.node_id);
                }
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
                // A spare that took over keeps looking for the leader it was following,
                // and gives the lead back when it answers (decided at the gig,
                // 2026-10-25: the Mini leads whenever it is up).
                if self.remembers {
                    self.handback = remembered::load(self.node_id);
                    if let Some(addr) = self.handback {
                        info!("[session] will hand the lead back to {addr} when it answers");
                    }
                }
            }

            SessionCommand::WatchNearby { session, on } => {
                if on {
                    self.watchers.insert(session);
                } else {
                    self.watchers.remove(&session);
                }
                self.set_browsing(!self.watchers.is_empty()).await;
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
        let said = (self.state.session_id, self.state.is_follower);
        if self.announced == Some(said) {
            return;
        }
        let instance_name = format!("pult-station-{}", &self.node_id.0.to_string()[..8]);
        let hostname = format!("{}.local.", gethostname());
        let mut props = HashMap::new();
        props.insert("node_id".to_string(), self.node_id.0.to_string());
        props.insert("hostname".to_string(), gethostname());
        props.insert("show_name".to_string(), self.show_name.clone());
        props.insert("sync_port".to_string(), self.sync_port.to_string());
        props.insert("follows".to_string(), if self.state.is_follower { "1" } else { "0" }.to_string());
        props.insert(
            "session_id".to_string(),
            self.state.session_id.map(|id| id.to_string()).unwrap_or_default(),
        );
        // The addresses a peer can actually reach, named — never `enable_addr_auto`,
        // which announces on every interface the machine has: two dozen on a Mac,
        // tunnels and AirDrop's among them. A station is announced for as long as it
        // runs, unlike a session, and a room of them each announcing on every
        // interface swamped a test machine running a few dozen until none could
        // accept a connection.
        let addresses: Vec<IpAddr> = match self.advertise_at {
            Some(ip) => vec![IpAddr::V4(ip)],
            None => crate::infra::net::reachable_for(&self.net, &pult_schema::types::network::NetService::Session)
                .into_iter()
                .map(IpAddr::V4)
                .collect(),
        };
        if addresses.is_empty() {
            debug!("[session] nowhere to announce this station yet");
            return;
        }
        let info = ServiceInfo::new(STATION_SERVICE, &instance_name, &hostname, &addresses[..], self.http_port, props);
        match info.map(|info| self.mdns.register(info)) {
            Ok(Ok(())) => self.announced = Some(said),
            Ok(Err(e)) => warn!("[session] mDNS register of this station failed: {e}"),
            Err(e) => warn!("[session] ServiceInfo::new for this station failed: {e}"),
        }
    }

    /// Start again with a new daemon, because this machine's addresses moved.
    ///
    /// `mdns-sd` 0.13 does not survive that: a daemon up while a Mac went from wifi to
    /// ethernet went on reporting what it had found as removed and never found it
    /// again, while a fresh one found it at once. And the station advert names its
    /// addresses, so without this it went on announcing the wifi's for as long as the
    /// station ran. Everything this manager has said or asked for is said and asked
    /// for again on the new one; what it has found stays, to be found again.
    fn renew_mdns(&mut self) {
        let mdns = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                warn!("[session] cannot renew the mDNS daemon: {e}");
                return;
            }
        };
        let address = self
            .net
            .bind(
                pult_schema::types::network::NetService::Session,
                "session discovery",
                self.net.prefs().session.as_deref(),
            )
            .unwrap_or(None);
        self.advertise_at = crate::infra::net::restrict_mdns(&mdns, address);
        let old = std::mem::replace(&mut self.mdns, mdns);
        let _ = old.shutdown();
        info!("[session] this machine's addresses moved; announcing and looking again");

        match self.mdns.browse(SERVICE_TYPE) {
            Ok(events) => self.forward(events),
            Err(e) => warn!("[session] mDNS browse failed: {e}"),
        }
        if self.browsing {
            match self.mdns.browse(STATION_SERVICE) {
                Ok(events) => self.forward(events),
                Err(e) => warn!("[session] mDNS browse for stations failed: {e}"),
            }
        }
        if let Some(info) = self.session_advert.clone() {
            if let Err(e) = self.mdns.register(info) {
                warn!("[session] mDNS register failed: {e}");
            }
        }
        self.announced = None;
        self.announce_station();
    }

    /// Feed a browse's events to the manager, until its daemon is shut down.
    fn forward(&self, events: mdns_sd::Receiver<ServiceEvent>) {
        let tx = self.station_events.clone();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                if tx.blocking_send(event).is_err() {
                    break;
                }
            }
        });
    }

    /// Start or stop looking for the other stations on the network.
    async fn set_browsing(&mut self, on: bool) {
        if on == self.browsing {
            return;
        }
        if on {
            match self.mdns.browse(STATION_SERVICE) {
                Ok(events) => {
                    self.forward(events);
                    self.browsing = true;
                }
                Err(e) => warn!("[session] mDNS browse for stations failed: {e}"),
            }
        } else {
            let _ = self.mdns.stop_browse(STATION_SERVICE);
            self.browsing = false;
            // What it found is forgotten with it: a list nobody is keeping up to date
            // is a list of stations that may have gone.
            self.candidates.clear();
            self.advert_names.retain(|_, advert| !matches!(advert, Advert::Station(_)));
            if !self.state.nearby.is_empty() {
                self.state.nearby.clear();
                self.push_state().await;
            }
        }
    }

    /// A station advert, resolved: listed as nearby unless it is this one.
    async fn station_resolved(&mut self, info: &ServiceInfo) {
        // A late answer to a browse that has been stopped.
        if !self.browsing {
            return;
        }
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
        let sync_addr = props
            .get("sync_port")
            .and_then(|v| v.val_str().parse::<u16>().ok())
            .map(|port| SocketAddr::new(http_addr.ip(), port).to_string())
            .unwrap_or_default();
        let station = NearbyStation {
            node_id,
            hostname: props.get("hostname").map(|v| v.val_str().to_string()).unwrap_or_default(),
            http_addr: http_addr.to_string(),
            sync_addr,
            follows: props.get("follows").is_some_and(|v| v.val_str() == "1"),
            show_name: props.get("show_name").map(|v| v.val_str().to_string()).unwrap_or_default(),
            session_id: props.get("session_id").and_then(|v| v.val_str().parse::<Uuid>().ok()),
        };
        self.advert_names.insert(info.get_fullname().to_string(), Advert::Station(node_id));
        // A station is resolved again every time any daemon on the network answers for
        // it, which on a busy network is several times a second. Asked at once only
        // when it is new or has moved; otherwise the next round of probes covers it,
        // and a probe per answer was a connection storm with a few dozen stations.
        let moved = self.candidates.get(&node_id) != Some(&station);
        if moved {
            self.probe(&station);
        }
        self.candidates.insert(node_id, station);
    }

    /// Ask whether a station answers at its HTTP port, off this actor: the answer comes
    /// back on `probes_rx`.
    fn probe(&self, station: &NearbyStation) {
        let tx = self.probes_tx.clone();
        let (node, http_addr) = (station.node_id, station.http_addr.clone());
        tokio::spawn(async move {
            let answered = matches!(
                tokio::time::timeout(PROBE_WAIT, tokio::net::TcpStream::connect(&http_addr)).await,
                Ok(Ok(_))
            );
            let _ = tx.send((node, http_addr, answered)).await;
        });
    }

    /// A probe came back: list the station if it answered, and take it off if not.
    async fn probed(&mut self, node: NodeId, http_addr: String, answered: bool) {
        // An answer about an address the station has since moved from, or a station
        // whose advert has since been withdrawn, says nothing about it now.
        let Some(station) = self.candidates.get(&node).filter(|s| s.http_addr == http_addr).cloned() else {
            return;
        };
        let listed = self.state.nearby.iter().position(|s| s.node_id == node);
        match (answered, listed) {
            (true, Some(i)) if self.state.nearby[i] == station => return,
            (true, Some(i)) => self.state.nearby[i] = station,
            (true, None) => {
                info!("[session] station {} on the network at {http_addr}", station.hostname);
                self.state.nearby.push(station);
            }
            (false, Some(i)) => {
                debug!("[session] station {} stopped answering at {http_addr}", station.hostname);
                self.state.nearby.remove(i);
            }
            (false, None) => return,
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
            self.session_advert = None;
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
            Ok(info) => match self.mdns.register(info.clone()) {
                Ok(()) => {
                    self.session_advert = Some(info);
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
                    self.candidates.remove(&node);
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