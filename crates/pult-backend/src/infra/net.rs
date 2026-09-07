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
}

#[derive(Debug, Default)]
struct Inner {
    interfaces: Vec<NetInterface>,
    bindings: BTreeMap<NetService, ServiceBinding>,
    standing: Standing,
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
            }),
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
        if let Ok(mut inner) = self.inner.write() {
            inner.interfaces = found;
        }
    }

    /// What the reporter last worked out about this station's place in the session.
    pub fn standing(&self) -> Standing {
        self.inner.read().map(|inner| inner.standing).unwrap_or_default()
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
pub fn restrict_mdns(daemon: &mdns_sd::ServiceDaemon, address: Option<Ipv4Addr>) -> Ipv4Addr {
    let Some(address) = address else {
        return crate::infra::local_ipv4();
    };
    if let Err(e) = daemon.disable_interface(mdns_sd::IfKind::All) {
        warn!("[network] could not restrict an mDNS daemon: {e}");
        return address;
    }
    if address.is_loopback() {
        let _ = daemon.enable_interface(mdns_sd::IfKind::LoopbackV4);
    }
    if let Err(e) = daemon.enable_interface(mdns_sd::IfKind::Addr(IpAddr::V4(address))) {
        warn!("[network] could not put an mDNS daemon on {address}: {e}");
    }
    address
}

/// The address to tell other machines to reach this station at.
///
/// What a service actually bound where it was told which cable, and otherwise
/// [`crate::infra::local_ipv4`] — which asks the route table and is therefore the
/// *house* LAN on a console with two networks. That is the defect this replaces: the
/// address a peer was told to dial for the show was decided by which cable reaches
/// the internet. It stays as the fallback because a console that has said nothing
/// about cables has to keep working exactly as it did.
pub fn advertised(bound: Option<Ipv4Addr>) -> Ipv4Addr {
    bound.unwrap_or_else(crate::infra::local_ipv4)
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
