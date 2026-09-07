use super::*;

use pult_schema::events::operation::NodeId;
use pult_schema::types::output::{OutputConfig, OutputKind};
use pult_schema::types::network::SacnPriority;
use uuid::Uuid;

fn iface(name: &str, address: &str) -> NetInterface {
    NetInterface {
        name: name.to_string(),
        addresses: vec![address.to_string()],
        up: true,
        loopback: address.starts_with("127."),
    }
}

fn a_station(prefs: NetworkPrefs) -> NetHandle {
    let net = Network::new(prefs);
    net.observe(vec![iface("lo0", "127.0.0.1"), iface("en5", "10.0.1.7")]);
    net
}

fn an_output(kind: OutputKind) -> OutputConfig {
    OutputConfig {
        id: Uuid::new_v4(),
        name: "House".into(),
        kind,
        target: None,
        universes: vec![],
        enabled: true,
        node_id: None,
        interfaces: Default::default(),
        priority: SacnPriority::default(),
    }
}

#[test]
fn told_nothing_binds_everything() {
    // The case every existing show and preferences file is in: nothing changes.
    let net = a_station(NetworkPrefs::default());
    assert_eq!(net.bind(NetService::Session, "session", None), Ok(None));
    let binding = &net.bindings()[0];
    assert!(binding.wanted.is_none() && binding.fault.is_none() && binding.bound.is_none());
}

#[test]
fn told_a_cable_binds_that_cable() {
    let net = a_station(NetworkPrefs::default());
    assert_eq!(
        net.bind(NetService::Session, "session", Some("en5")),
        Ok(Some("10.0.1.7".parse().unwrap()))
    );
    assert_eq!(net.bindings()[0].bound.as_deref(), Some("10.0.1.7"));
}

#[test]
fn told_a_cable_that_is_not_here_refuses_and_says_which() {
    let net = a_station(NetworkPrefs::default());
    let outcome = net.bind(NetService::OpenHaunt, "OpenHaunt", Some("eth9"));
    assert!(outcome.is_err(), "a sender must not fall back to every interface");
    let binding = &net.bindings()[0];
    assert_eq!(binding.wanted.as_deref(), Some("eth9"));
    assert!(binding.bound.is_none());
    assert_eq!(binding.fault, Some(InterfaceError::Unknown("eth9".into())));
}

#[test]
fn a_listener_falls_back_and_still_reports_the_fault() {
    // The one deliberate exception: the page is how the setting gets fixed, so a
    // console that will not serve it cannot be put right from the desk. The fault
    // is recorded exactly as a refusal would have been.
    let net = a_station(NetworkPrefs::default());
    assert_eq!(net.bind_listener(NetService::Http, "the console's page", Some("eth9")), None);
    let binding = &net.bindings()[0];
    assert_eq!(binding.fault, Some(InterfaceError::Unknown("eth9".into())));
    assert!(binding.bound.is_none(), "it did not get the cable it asked for");
}

#[test]
fn a_cable_appearing_makes_a_refusal_recoverable() {
    // What re-resolving on the probe tick buys: plugging the show network in never
    // needs a restart at twenty-five past seven.
    let net = a_station(NetworkPrefs::default());
    assert!(net.bind(NetService::Session, "session", Some("en9")).is_err());

    net.observe(vec![iface("en5", "10.0.1.7"), iface("en9", "2.0.0.1")]);
    assert_eq!(
        net.bind(NetService::Session, "session", Some("en9")),
        Ok(Some("2.0.0.1".parse().unwrap()))
    );
    assert!(net.bindings()[0].fault.is_none(), "and the fault clears with it");
}

#[test]
fn a_cable_going_away_becomes_a_fault_on_the_row() {
    let net = a_station(NetworkPrefs::default());
    assert!(net.bind(NetService::Session, "session", Some("en5")).is_ok());
    net.observe(vec![iface("lo0", "127.0.0.1")]);
    assert!(net.bind(NetService::Session, "session", Some("en5")).is_err());
    assert!(net.bindings()[0].fault.is_some());
}

#[test]
fn a_row_beats_the_preference_and_the_preference_beats_nothing() {
    let mine = NodeId::new();
    let net = a_station(NetworkPrefs { artnet: Some("en5".into()), ..Default::default() });

    let plain = an_output(OutputKind::Artnet);
    assert_eq!(net.for_output(&plain, mine).as_deref(), Some("en5"), "the station's own");

    let mut named = plain.clone();
    named.interfaces.insert(mine, "en9".into());
    assert_eq!(net.for_output(&named, mine).as_deref(), Some("en9"), "the row is specific");

    // And a row that names a *different* station says nothing about this one.
    let mut theirs = plain.clone();
    theirs.interfaces.insert(NodeId::new(), "eth1".into());
    assert_eq!(net.for_output(&theirs, mine).as_deref(), Some("en5"));

    // A kind the preferences say nothing about falls through to nothing at all.
    let sacn = an_output(OutputKind::Sacn);
    assert_eq!(net.for_output(&sacn, mine), None);
}

#[test]
fn a_stopped_service_leaves_no_row_behind() {
    let net = a_station(NetworkPrefs::default());
    let service = NetService::Output(Uuid::new_v4());
    let _ = net.bind(service.clone(), "House", Some("en5"));
    assert_eq!(net.bindings().len(), 1);
    net.forget(&service);
    assert!(net.bindings().is_empty(), "a removed output claims no cable");
}

#[test]
fn the_machine_reads_as_a_list_a_dropdown_can_use() {
    // Not an assertion about this machine's cabling — only that enumerating works
    // at all, has a stable order, and finds the one interface every machine has.
    let found = interfaces();
    assert!(!found.is_empty(), "every machine has at least a loopback");
    let names: Vec<&str> = found.iter().map(|i| i.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "a list that reshuffles cannot be clicked");
    assert!(found.iter().any(|i| i.loopback), "loopback is included, not excluded");
}

// ── and that the restriction reaches the wire ─────────────────────────────────

/// The one interface restriction a test machine can actually make.
///
/// A real two-NIC console cannot be assumed on anybody's CI, and a loopback alias
/// needs `sudo` on a Mac — so there is no way to *build* a second network here. What
/// there is instead is `mdns-sd`'s own default: loopback is off unless a daemon is
/// told otherwise. So a daemon restricted to `127.0.0.1` and one left alone are two
/// daemons on genuinely different interfaces, and whether one can see the other's
/// advertisement is a real answer about whether `restrict_mdns` does anything.
///
/// Everything above this is arithmetic; this is the test that the setting reaches a
/// socket at all, which is the half that otherwise silently does nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_daemon_is_on_the_interface_it_was_told_and_not_the_others() {
    use mdns_sd::{ServiceDaemon, ServiceInfo};

    const TYPE: &str = "_pult-net-test._tcp.local.";
    let loopback: Ipv4Addr = "127.0.0.1".parse().unwrap();

    let Ok(advertiser) = ServiceDaemon::new() else {
        // No mDNS on this machine at all. Said rather than passed quietly.
        eprintln!("no mDNS daemon here; nothing to prove");
        return;
    };
    assert_eq!(
        restrict_mdns(&advertiser, Some(loopback)),
        loopback,
        "what a service advertises is what it bound"
    );

    let info = ServiceInfo::new(TYPE, "restricted", "restricted.local.", IpAddr::V4(loopback), 9999, None)
        .expect("a service to advertise");
    advertiser.register(info).expect("advertising on loopback");

    // A daemon told the same thing finds it.
    let listener = ServiceDaemon::new().expect("a second daemon");
    restrict_mdns(&listener, Some(loopback));
    let found = saw_it(listener.browse(TYPE).expect("browsing")).await;

    // And one that was told nothing does not — because `mdns-sd` leaves loopback off
    // unless asked, so this daemon is genuinely on the other interfaces only.
    let elsewhere = ServiceDaemon::new().expect("a third daemon");
    let found_elsewhere = saw_it(elsewhere.browse(TYPE).expect("browsing")).await;

    let _ = advertiser.shutdown();
    let _ = listener.shutdown();
    let _ = elsewhere.shutdown();

    if !found {
        // The positive half is what says this machine can do mDNS on loopback at
        // all. Without it the negative half proves nothing — a daemon that finds
        // nothing anywhere is not a daemon that was restricted — so say so rather
        // than assert a pass that means nothing.
        eprintln!("this machine does not carry mDNS over loopback; nothing to prove");
        return;
    }
    assert!(
        !found_elsewhere,
        "a daemon left on the other interfaces saw an advertisement made only on loopback, \
         so restricting one does nothing"
    );
}

/// Wait a little for one advertisement, and answer whether it arrived.
async fn saw_it(rx: mdns_sd::Receiver<mdns_sd::ServiceEvent>) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    tokio::task::spawn_blocking(move || {
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(_)) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}
