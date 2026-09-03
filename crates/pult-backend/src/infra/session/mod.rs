use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
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

use crate::{engine::EngineHandle, infra::local_ipv4, infra::sync::SyncHandle};

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
    /// Where each discovered session might be reached, best first.
    ///
    /// A list rather than an address, because a station advertises every address it
    /// has and only some of them are any use from here — see [`reachable_at`]. Keyed
    /// by session_id for deduplication; the engine only ever sees the first one.
    discovered_addrs: HashMap<Uuid, Vec<SocketAddr>>,
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
                        state: SessionState { node_id: Some(node_id), ..Default::default() },
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
                state: SessionState { node_id: Some(node_id), ..Default::default() },
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
        // Say who this node is before anything happens to it. Nothing else will:
        // state is pushed on change, and this node's own id never changes.
        self.push_state().await;

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
                        Some(cmd) => self.handle_command(cmd).await,
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
        if let Err(e) = self.mdns.shutdown() {
            debug!("[session] mDNS shutdown: {e}");
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
                let Some(addrs) = self.discovered_addrs.get(&session_id).cloned() else {
                    let _ = reply.send(Err(format!("session {session_id} not in discovered list")));
                    return;
                };
                let show_id = self.discovered_show_ids.get(&session_id).copied().unwrap_or_default();
                let leader = self.discovered_leader_ids.get(&session_id).copied().unwrap_or(self.node_id);

                // Every address it advertised that could be dialled from here, in the
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

                    let reachable =
                        reachable_at(info.get_addresses().iter().copied(), info.get_port());
                    let Some(sync_addr) = reachable.first().copied() else {
                        // Nothing it advertised can be dialled from here. Not listed,
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

// ── Where a discovered station can actually be reached ────────────────────────

/// The addresses a discovered station can be dialled at, best first.
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
/// address for itself ([`local_ipv4`]) and preferring the family it named keeps two
/// stations agreeing about which path they are on.
///
/// **An IPv6 link-local address is dropped rather than ranked last**, because it cannot
/// be dialled at all: `fe80::/10` is only meaningful together with the interface it was
/// learned on, and mDNS does not say which that was — so the `SocketAddr` built from it
/// has no scope id and the connect fails with "No route to host". That is the bug this
/// function exists for. Keeping it as a last resort would only spend a timeout before
/// reaching the address that was going to work anyway.
fn reachable_at(addresses: impl IntoIterator<Item = IpAddr>, port: u16) -> Vec<SocketAddr> {
    let mut ranked: Vec<(u8, IpAddr)> =
        addresses.into_iter().filter_map(|ip| reach(&ip).map(|rank| (rank, ip))).collect();
    // By address as well as by rank, so a station with two equally good addresses is
    // dialled at the same one every time and the log is worth reading.
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked.dedup();
    ranked.into_iter().map(|(_, ip)| SocketAddr::new(ip, port)).collect()
}

/// How far an address reaches, lower being further. `None` cannot be dialled at all.
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
    /// `fe80::` one, which cannot be dialled without the interface it was learned on.
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