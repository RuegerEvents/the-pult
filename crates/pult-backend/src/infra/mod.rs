pub mod assets;
pub mod identity;
pub mod preferences;
pub mod showfile;
pub mod sync;
pub mod session;
pub mod stations;
pub mod devices;
pub mod connectors;
pub mod plugins;

/// The address this machine reaches the local network on.
///
/// Found by asking the routing table which interface a packet to the outside would
/// leave by, without sending one. Loopback if there is no route at all, which keeps
/// a laptop with no network from failing to start.
pub fn local_ipv4() -> std::net::Ipv4Addr {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|s| s.connect("8.8.8.8:80").ok().map(|_| s))
        .and_then(|s| s.local_addr().ok())
        .and_then(|a| match a.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(_) => None,
        })
        .unwrap_or(std::net::Ipv4Addr::LOCALHOST)
}
