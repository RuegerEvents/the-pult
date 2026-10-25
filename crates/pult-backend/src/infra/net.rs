//! Which cable each of this station's services goes out on.
//!
//! The rule is `pult_schema::types::network` and lives there because a plugin, a
//! panel and a test all ask it. This is the station's half: what this machine's
//! interfaces actually are, what each service was told, and what came of it.
//!
//! **One place resolves, and everything records.** A service calls [`Network::bind`]
//! with what it was told and gets back an address to bind or a reason it cannot, and
//! the answer is kept so the `Station` row can carry it to a console in another room.
//! Nothing resolves a name for itself, because six services answering the same
//! question six ways is how this went wrong in the first place.
//!
//! **Two absences, two answers.** Told nothing is `Ok(None)` — bind everything, the
//! way this console always did, which is why no existing show or preferences file
//! has to change. Told an interface that is not here is `Err`, and the caller
//! refuses rather than quietly binding everything.
//!
//! **With one deliberate exception, and it is worth reading before changing.** A
//! *listener* — the HTTP server, the sync port — falls back to `0.0.0.0` with the
//! fault recorded rather than refusing outright, because the page is how an operator
//! fixes the setting: a console that will not serve its own UI because somebody
//! named the wrong cable is a console nobody can put right without a text editor and
//! a restart. A listener bound too widely *offers* something, which is a smaller
//! harm than a console that cannot be reached. Everything that decides where bytes
//! *go* — the output sockets, sACN's multicast interface — and everything that
//! decides who finds whom — the mDNS daemons — refuses, because that is where a
//! wrong cable actually does damage. [`Network::bind`] is the refusing form and
//! [`Network::bind_listener`] the falling-back one, so which a service uses is a
//! choice it makes by name.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, RwLock};

use pult_schema::types::network::{
    resolve, InterfaceError, NetInterface, NetService, ServiceBinding,
};
use serde::{Deserialize, Serialize};
use sysinfo::{InterfaceOperationalState, Networks};
use tracing::warn;

/// The `[network]` section of `preferences.toml`.
///
/// A station preference and never show data, which barely needs arguing: which cable
/// is in which socket is a fact about this machine. Six keys rather than one, because
/// one is simpler and wrong — the whole point is that Art-Net is on a different cable
/// from the tablet.
///
/// `artnet` and `sacn` are not services that bind on their own. They are the fallback
/// an output row takes when its own `interfaces` map does not name this station, which
/// is what keeps a rig with one Art-Net network from having to name it per output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkPrefs {
    /// The page, the WebSocket, and a hosted MVR-xchange group.
    pub http: Option<String>,
    /// `_pult._tcp` and the sync listener.
    pub session: Option<String>,
    /// `_mvrxchange._tcp` and its listener.
    pub mvr_xchange: Option<String>,
    /// The OpenHaunt device browser and the broker this station runs for them.
    pub openhaunt: Option<String>,
    /// The fallback for an Art-Net output row that names no interface.
    pub artnet: Option<String>,
    /// The fallback for an sACN output row that names no interface.
    pub sacn: Option<String>,
}

impl NetworkPrefs {
    /// What this station prefers for one of the four services that bind on their own.
    pub fn for_service(&self, service: &NetService) -> Option<&str> {
        match service {
            NetService::Http => self.http.as_deref(),
            NetService::Session => self.session.as_deref(),
            NetService::MvrXchange => self.mvr_xchange.as_deref(),
            NetService::OpenHaunt => self.openhaunt.as_deref(),
            // A row's fallback depends on its kind, which this does not know;
            // `Network::for_output` and `for_input` are where that is asked.
            NetService::Output(_) | NetService::Input(_) => None,
        }
    }
}

/// This machine's interfaces, right now.
///
/// A fresh enumeration, for a service coming up before the probe thread has produced
/// a reading. The steady-state source is [`Network::observe`], fed from the probe —
/// which already refreshes the machine's interfaces every couple of seconds for the
/// throughput figures, so nothing here adds a syscall to the loop.
///
/// **On the runtime thread, deliberately, and it was measured before it was written
/// there.** `infra::stations` records why the disks and the thermal sensors are on a
/// thread of their own: the first volume enumeration in a process took six seconds
/// against a large `target/debug/deps`, and six seconds on the runtime is a station
/// that accepts no connection. Interfaces are not like that — **0.5 ms for 24 of
/// them, and no slower on the first call than the third** — so a service can ask on
/// its way up without a channel and a wait.
pub fn interfaces() -> Vec<NetInterface> {
    read(&Networks::new_with_refreshed_list())
}

/// Turn what `sysinfo` holds into what a station publishes.
///
/// `Down` is the only state read as down. `Unknown` is what Linux reports for
/// loopback and what a driver reports when it will not say, and treating either as
/// unusable would hide an interface a dev station deliberately binds.
pub fn read(networks: &Networks) -> Vec<NetInterface> {
    let mut found: Vec<NetInterface> = networks
        .iter()
        .map(|(name, data)| NetInterface {
            name: name.clone(),
            addresses: data
                .ip_networks()
                .iter()
                .filter(|net| net.addr.is_ipv4())
                .map(|net| net.addr.to_string())
                .collect(),
            up: data.operational_state() != InterfaceOperationalState::Down,
            loopback: is_loopback(name, data.ip_networks()),
        })
        .collect();
    // A stable order, because this is a dropdown as well as a diagnostic and a list
    // that reshuffles every two seconds cannot be clicked.
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The addresses another machine could reach this one at, in a stable order: what
/// [`Network::address_changes`] compares. Loopback and interfaces that are down are
/// not places anything is found.
fn addresses_of(interfaces: &[NetInterface]) -> Vec<&str> {
    let mut addresses: Vec<&str> = interfaces
        .iter()
        .filter(|i| i.up && !i.loopback)
        .flat_map(|i| i.addresses.iter().map(String::as_str))
        .collect();
    addresses.sort_unstable();
    addresses
}

fn is_loopback(name: &str, addresses: &[sysinfo::IpNetwork]) -> bool {
    // By address rather than by name where there is one: `lo`, `lo0` and `Loopback
    // Pseudo-Interface 1` are three spellings of one thing, and the address is the
    // same everywhere.
    addresses.iter().any(|net| match net.addr {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }) || name.starts_with("lo")
}

/// What every service on this station was told about cables, and what came of it.
#[derive(Debug, Default)]
pub struct Network {
    prefs: NetworkPrefs,
    inner: RwLock<Inner>,
    /// Counts the times this machine's addresses have changed, for what has to start
    /// again when they do — see [`Network::address_changes`].
    moved: tokio::sync::watch::Sender<u64>,
}

#[derive(Debug, Default)]
struct Inner {
    interfaces: Vec<NetInterface>,
    bindings: BTreeMap<NetService, ServiceBinding>,
    standing: Standing,
    /// Whether the last look found multicast refused, so the moment it is allowed can
    /// be told apart from it always having been.
    multicast_refused: bool,
}

/// Where this station stands in the session, as the reporter last worked it out.
///
/// Kept here rather than passed down because both of the things it decides are read
/// on the output path: which outputs this station runs at all — an unowned Art-Net
/// output is the leader's — and what priority byte an sACN packet carries. The
/// reporter is the one thing that already computes both, on a timer, and a second
/// answer to "am I the leader" is a second answer to which console is driving the rig.
///
/// A lone console is the leader, so that is the default: nothing about a one-station
/// rig waits for a first report to start sending.
#[derive(Debug, Clone, Copy)]
pub struct Standing {
    pub is_leader: bool,
    /// The sACN priority slot this station claimed, or `None` on a leader, which is
    /// at the top of the ladder rather than in it.
    pub sacn_slot: Option<u8>,
}

impl Default for Standing {
    fn default() -> Self {
        Standing { is_leader: true, sacn_slot: None }
    }
}

/// Shared, because every service on the station reports into it and the reporter
/// reads it. Cheap to clone and never held across an await.
pub type NetHandle = Arc<Network>;

impl Network {
    pub fn new(prefs: NetworkPrefs) -> NetHandle {
        Arc::new(Network {
            prefs,
            inner: RwLock::new(Inner {
                interfaces: interfaces(),
                bindings: BTreeMap::new(),
                standing: Standing::default(),
                multicast_refused: multicast_refused().is_some(),
            }),
            moved: Default::default(),
        })
    }

    /// A station that says nothing about cables: every test, and every console
    /// whose operator has never opened the Network panel.
    pub fn unconfigured() -> NetHandle {
        Network::new(NetworkPrefs::default())
    }

    pub fn prefs(&self) -> &NetworkPrefs {
        &self.prefs
    }

    /// Take a fresh reading of the machine's interfaces.
    ///
    /// Called from the reporter on the probe tick, which is what makes a refusal
    /// recoverable: a service told to use a cable that was not there when the console
    /// came up starts by itself when somebody plugs it in, and never needs a restart
    /// at twenty-five past seven.
    pub fn observe(&self, found: Vec<NetInterface>) {
        // macOS refuses a freshly built app the local network for its first few seconds
        // (seen at every install at the gig), and an mDNS daemon started in that window
        // stays deaf for good: the Mini showed every node as not on the network after
        // each restart. Multicast becoming allowed is therefore an address change as far
        // as the daemons are concerned — they start again on it.
        let refused = multicast_refused().is_some();
        let moved = match self.inner.write() {
            Ok(mut inner) => {
                let mut moved = addresses_of(&inner.interfaces) != addresses_of(&found);
                if inner.multicast_refused && !refused {
                    tracing::info!("[network] multicast is allowed now; discovery starts again");
                    moved = true;
                }
                inner.multicast_refused = refused;
                inner.interfaces = found;
                moved
            }
            Err(_) => false,
        };
        if moved {
            self.moved.send_modify(|n| *n += 1);
        }
    }

    /// Ticks whenever an address this machine can be reached at comes or goes.
    ///
    /// For an mDNS daemon, which **does not survive it**: `mdns-sd` 0.13 adds a socket
    /// for an interface that appears, but a daemon that was up when a Mac moved from
    /// wifi to ethernet went on reporting the node it had found as removed, several
    /// times a second, and never resolved it again — while `dns-sd` and a fresh daemon
    /// on the same machine both found it at once. Every node then looked switched off
    /// and was never told where the broker had moved to. So a browser starts again
    /// when this ticks, rather than trusting the daemon to follow the cable.
    pub fn address_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.moved.subscribe()
    }

    /// What the reporter last worked out about this station's place in the session.
    pub fn standing(&self) -> Standing {
        self.inner.read().map(|inner| inner.standing).unwrap_or_default()
    }

    /// The session's half of the standing, the moment it changes. A leader is at the
    /// top of the sACN ladder rather than in a slot; a follower keeps whatever slot
    /// the reporter last claimed for it until the reporter claims again.
    pub fn set_leader(&self, is_leader: bool) {
        if let Ok(mut inner) = self.inner.write() {
            inner.standing.is_leader = is_leader;
            if is_leader {
                inner.standing.sacn_slot = None;
            }
        }
    }

    pub fn set_standing(&self, standing: Standing) {
        if let Ok(mut inner) = self.inner.write() {
            inner.standing = standing;
        }
    }

    pub fn interfaces(&self) -> Vec<NetInterface> {
        self.inner.read().map(|inner| inner.interfaces.clone()).unwrap_or_default()
    }

    /// What each service was told, what it managed, and what went wrong.
    pub fn bindings(&self) -> Vec<ServiceBinding> {
        self.inner.read().map(|inner| inner.bindings.values().cloned().collect()).unwrap_or_default()
    }

    /// Resolve what a service was told, and record the answer.
    ///
    /// `Ok(None)` is "told nothing": bind every interface, which is what this console
    /// did before any of this existed. `Err` is "told something wrong", and the
    /// caller must refuse rather than fall back — that fall-back is the silent wrong
    /// cable this whole mechanism exists to remove.
    pub fn bind(
        &self,
        service: NetService,
        label: &str,
        wanted: Option<&str>,
    ) -> Result<Option<Ipv4Addr>, InterfaceError> {
        let found = self.interfaces();
        let outcome = wanted.map(|w| resolve(w, &found));
        let binding = ServiceBinding {
            service: service.clone(),
            label: label.to_string(),
            wanted: wanted.map(str::to_string),
            bound: match &outcome {
                Some(Ok(addr)) => Some(addr.to_string()),
                _ => None,
            },
            fault: match &outcome {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            },
        };
        if let Some(Err(e)) = &outcome {
            warn!("[network] {label} cannot start: {e}");
        }
        self.record(binding);
        match outcome {
            None => Ok(None),
            Some(Ok(addr)) => Ok(Some(addr)),
            Some(Err(e)) => Err(e),
        }
    }

    /// The same question for something that *listens*, which answers it differently.
    ///
    /// A listener falls back to every interface rather than refusing, with the fault
    /// recorded exactly as it would have been — see this module's own note. The page
    /// is how the setting gets fixed, and a console that will not serve it because a
    /// cable is out is one nobody can put right from the desk.
    pub fn bind_listener(
        &self,
        service: NetService,
        label: &str,
        wanted: Option<&str>,
    ) -> Option<Ipv4Addr> {
        match self.bind(service, label, wanted) {
            Ok(addr) => addr,
            Err(_) => {
                warn!("[network] {label} is listening on every interface instead");
                None
            }
        }
    }

    /// Which cable an output should use: what its own row says for this station,
    /// then what the station prefers for that kind, then nothing.
    ///
    /// The row wins because it is the specific answer — this Art-Net output on this
    /// station — and the preference is what keeps a rig with one lighting network
    /// from naming it on every row.
    pub fn for_output(
        &self,
        config: &pult_schema::types::output::OutputConfig,
        node_id: pult_schema::events::operation::NodeId,
    ) -> Option<String> {
        use pult_schema::types::output::OutputKind;
        config.interface_for(node_id).map(str::to_string).or_else(|| {
            match config.kind {
                OutputKind::Artnet => self.prefs.artnet.clone(),
                OutputKind::Sacn => self.prefs.sacn.clone(),
                OutputKind::OpenHaunt => self.prefs.openhaunt.clone(),
            }
        })
    }

    /// The same question for an input, which asks it the same way and of the same
    /// preferences: which cable an sACN receiver joins its groups on is the same fact
    /// about the machine as which cable an sACN sender leaves by.
    pub fn for_input(
        &self,
        config: &pult_schema::types::input::InputConfig,
        node_id: pult_schema::events::operation::NodeId,
    ) -> Option<String> {
        use pult_schema::types::input::InputKind;
        config.interface_for(node_id).map(str::to_string).or_else(|| match config.kind {
            InputKind::Artnet => self.prefs.artnet.clone(),
            InputKind::Sacn => self.prefs.sacn.clone(),
        })
    }

    fn record(&self, binding: ServiceBinding) {
        if let Ok(mut inner) = self.inner.write() {
            inner.bindings.insert(binding.service.clone(), binding);
        }
    }

    /// Forget a service that has stopped, so a removed output does not leave a row
    /// claiming a cable nothing is on.
    pub fn forget(&self, service: &NetService) {
        if let Ok(mut inner) = self.inner.write() {
            inner.bindings.remove(service);
        }
    }
}

/// Hold an mDNS daemon to one interface, and say what it should advertise.
///
/// `mdns-sd`'s `IfKind` takes exactly the two forms this console stores — a name or
/// an address — so restricting a daemon is disabling everything and enabling the one.
/// **A daemon both advertises and browses**, so this restricts discovery as well:
/// a console told to keep the session on the lighting network will not *find* a peer
/// on the house LAN either, which is the feature rather than a limitation. Adopting a
/// node the console cannot reach is how a fixture ends up on a cable that cannot
/// carry it.
///
/// Loopback is enabled explicitly where the address is a loopback one, because
/// `mdns-sd` disables it by default — and it is the one interface restriction a test
/// machine can actually make.
///
/// Answers the address to advertise where it was told one, and `None` where it was
/// not — in which case the caller registers with `enable_addr_auto`, so the daemon
/// puts *each interface's own* address in the record it sends out of that interface.
/// The old answer here was [`crate::infra::local_ipv4`], which on a lighting network
/// with no default route is loopback: a peer was told to dial 127.0.0.1.
pub fn restrict_mdns(daemon: &mdns_sd::ServiceDaemon, address: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
    say_if_multicast_is_refused();
    let address = address?;
    if let Err(e) = daemon.disable_interface(mdns_sd::IfKind::All) {
        warn!("[network] could not restrict an mDNS daemon: {e}");
        return Some(address);
    }
    if address.is_loopback() {
        let _ = daemon.enable_interface(mdns_sd::IfKind::LoopbackV4);
    }
    if let Err(e) = daemon.enable_interface(mdns_sd::IfKind::Addr(IpAddr::V4(address))) {
        warn!("[network] could not put an mDNS daemon on {address}: {e}");
    }
    Some(address)
}

/// Every address another machine could reach this station at, best first, for a
/// service bound on `bound` — or on every interface, where it was told nothing.
///
/// What a service actually bound wins outright: that is task 65's rule, and a door on
/// the lighting network is not reachable from the house LAN however it is advertised.
/// Told nothing, it is every IPv4 address on an interface that is up and not loopback,
/// with `routed` — the address the route table would leave by, when it is not loopback
/// — first, so a console with one network says exactly what it always said. A
/// link-local `169.254/16` address comes last: it reaches the segment and nothing else.
///
/// **Loopback only when there is nothing else**, and the reason is the whole defect
/// this exists for. [`crate::infra::local_ipv4`] asks the route table which way a
/// packet to the internet would leave, and a venue's lighting network has no route to
/// the internet — so the answer was 127.0.0.1, and every node on that network was told
/// to dial itself. A machine with no interface but loopback has no remote client to
/// mislead: whoever is asking is on this machine.
pub fn reachable(bound: Option<Ipv4Addr>, interfaces: &[NetInterface], routed: Option<Ipv4Addr>) -> Vec<Ipv4Addr> {
    if let Some(bound) = bound.filter(|b| !b.is_unspecified()) {
        return vec![bound];
    }
    let usable = |ip: &Ipv4Addr| !ip.is_loopback() && !ip.is_unspecified() && !ip.is_broadcast() && !ip.is_multicast();
    let mut found: Vec<Ipv4Addr> = interfaces
        .iter()
        .filter(|i| i.up && !i.loopback)
        .flat_map(|i| i.ipv4())
        .filter(usable)
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    found.retain(|ip| seen.insert(*ip));
    // Stable, so the list a node is handed does not reshuffle between two reports.
    found.sort_by_key(|ip| (Some(*ip) != routed, ip.is_link_local()));
    if found.is_empty() {
        found.push(Ipv4Addr::LOCALHOST);
    }
    found
}

/// [`reachable`] for one of this station's own services, as it stands now.
pub fn reachable_for(net: &Network, service: &NetService) -> Vec<Ipv4Addr> {
    let bound = net
        .bindings()
        .into_iter()
        .find(|b| &b.service == service)
        .and_then(|b| b.bound)
        .and_then(|b| b.parse::<Ipv4Addr>().ok());
    let routed = Some(crate::infra::local_ipv4()).filter(|ip| !ip.is_loopback());
    reachable(bound, &net.interfaces(), routed)
}

/// Bind a UDP socket on the interface a service was given, or on every one.
///
/// The one place an output's socket is opened, so that "which cable" and "which port"
/// are not decided in three connectors that can drift.
pub async fn udp(address: Option<Ipv4Addr>) -> std::io::Result<tokio::net::UdpSocket> {
    let bind = address.unwrap_or(Ipv4Addr::UNSPECIFIED);
    tokio::net::UdpSocket::bind((bind, 0)).await
}

#[cfg(test)]
mod tests;

/// Say so, once per process, if this process may not send multicast at all.
///
/// Every mDNS daemon here — the session, the device browser, MVR-xchange — and every
/// sACN output to a multicast group depends on it, and when it is refused the daemon
/// says nothing above `debug`: it skips each interface with "No route to host" and
/// browses an empty network. On macOS that is what the **Local Network** privacy
/// setting does to a process whose app was never allowed it — found when a station
/// started from an agent's shell discovered none of the simulators beside it, while
/// unicast to the same LAN went through. A console in that state looks like a venue
/// network with nothing on it, so it is a `warn` with the remedy in it.
pub fn say_if_multicast_is_refused() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(e) = multicast_refused() {
            warn!(
                "[network] this process may not send multicast ({e}): no mDNS, so no peers or \
                 OpenHaunt nodes will be found, and multicast sACN will not leave. On macOS, \
                 allow Local Network access for the app that started the console \
                 (System Settings → Privacy & Security → Local Network): the-pult \
                 itself for the desktop app, and the terminal (Terminal, iTerm, an \
                 editor) for a pult-backend started from a shell"
            );
        }
    });
}

/// Try one empty mDNS query to the group, on whatever interface the route table picks.
///
/// Twelve zero bytes is a DNS header with no questions — what `mdns-sd` itself sends
/// to test a socket — so a responder that hears it has nothing to answer.
pub fn multicast_refused() -> Option<std::io::Error> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    match socket.send_to(&[0u8; 12], (Ipv4Addr::new(224, 0, 0, 251), 5353)) {
        Ok(_) => None,
        // A machine with no network at all is not a refusal: there is nowhere to send.
        Err(e) if e.kind() == std::io::ErrorKind::NetworkUnreachable => None,
        Err(e) => Some(e),
    }
}
