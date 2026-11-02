//! The MQTT broker the leader runs for its OpenHaunt nodes.
//!
//! A node is discovered, not configured — which has to include not being told to
//! go and find a broker somebody installed first. So the console runs one itself
//! and hands out its address on adoption.
//!
//! Two halves, because rumqttd cannot be stopped and the door to it has to be. The
//! **broker** is rumqttd on loopback, on a port of its own, started once per door
//! port and never stopped: `Broker::start` blocks for the life of the process, and a
//! node reconnecting to a broker that was torn down and rebuilt would lose its
//! retained status for no gain. The **door** ([`FrontDoor`]) is what a node dials —
//! the configured port on every interface, piped through to the broker — and it is
//! open exactly while this station drives the nodes.
//!
//! That is the whole of what stops two stations both believing they own a node: a
//! node can only ever be connected to the station commanding it, because nobody
//! else's door is open. A station that stops leading closes its door, every node on
//! it drops, and each walks its broker list to whoever leads now (doc 15). A door
//! that stayed open on a follower would hold nodes whose inputs nobody acts on.
//!
//! The config is built in Rust rather than parsed from TOML, so a shape change
//! between rumqttd releases is a compile error here rather than a runtime one on
//! someone's show night.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{Mutex, OnceLock},
    time::Duration,
};

use rumqttd::{Broker, ConnectionSettings, RouterConfig, ServerSettings};
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{debug, info, warn};

/// Where each running broker listens, by the door port it serves.
///
/// Per door rather than per process, so two stations in one process — every
/// two-station test — have a broker each, the way two machines would: a retained
/// message one of them publishes is not one the other's nodes hear.
static RUNNING: OnceLock<Mutex<HashMap<u16, SocketAddr>>> = OnceLock::new();

/// Start the broker behind `door_port` if it is not already running, and say where
/// it listens on loopback.
///
/// Idempotent by design: adoption calls this, and so does becoming the leader, and
/// neither knows what the other did. A station that stops and another that starts in
/// its place on the same port — a show being opened — find the same broker, with its
/// retained topics.
pub fn ensure(door_port: u16) -> SocketAddr {
    let running = RUNNING.get_or_init(Default::default);
    let mut running = running.lock().unwrap();
    if let Some(addr) = running.get(&door_port) {
        return *addr;
    }
    // A free loopback port, asked of the OS and given straight back: rumqttd binds
    // what it is told and cannot be handed a listener.
    let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(0);
    let listen = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let config = config(listen);

    std::thread::Builder::new()
        .name("openhaunt-broker".into())
        .spawn(move || {
            let mut broker = Broker::new(config);
            // Returns only when the broker stops, which is at shutdown.
            if let Err(e) = broker.start() {
                warn!("[devices] MQTT broker stopped: {e}");
            }
        })
        .map(|_| info!("[devices] MQTT broker on {listen}, behind port {door_port}"))
        .unwrap_or_else(|e| warn!("[devices] could not start the MQTT broker: {e}"));

    running.insert(door_port, listen);
    listen
}

/// Where a node should be told to publish. The door binds `0.0.0.0`, so the
/// address handed out has to be one this console is actually reachable on.
pub fn advertised_addr(local_ip: std::net::IpAddr, port: u16) -> String {
    format!("{local_ip}:{port}")
}

/// Every `ip:port` this station's door can be reached at, best first: on the cable
/// the `openhaunt` preference names, or else every address this machine has that is
/// not loopback ([`crate::infra::net::reachable`]).
pub fn doors(net: &crate::infra::net::Network, port: u16) -> Vec<String> {
    crate::infra::net::reachable_for(net, &pult_schema::types::network::NetService::OpenHaunt)
        .into_iter()
        .map(|ip| advertised_addr(std::net::IpAddr::V4(ip), port))
        .collect()
}

/// Which of a station's doors to hand the nodes: the one nearest the nodes.
///
/// "Nearest" is the longest run of leading bits shared with a node's address, summed
/// over the nodes, and it is a stand-in for "on the same subnet" that needs no prefix
/// length — which the inventory does not carry, and which a leader could not know
/// for a *follower's* interfaces anyway. A station with a house LAN on 192.168.1.5 and
/// the lighting network on 10.0.0.5 shares no leading bits between its house address
/// and a node at 10.0.0.42, and 26 between its show address and it.
///
/// A node on loopback says nothing about which network a *remote* station should be
/// reached on, so it is not counted. With nothing to go on, the station's own first
/// door — the one it would name itself by.
pub fn nearest_door(doors: &[String], nodes: &[Ipv4Addr]) -> Option<String> {
    let nodes: Vec<Ipv4Addr> = nodes.iter().copied().filter(|n| !n.is_loopback()).collect();
    let score = |door: &String| -> u32 {
        let Some(ip) = door.parse::<SocketAddr>().ok().and_then(|a| match a.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(_) => None,
        }) else {
            return 0;
        };
        nodes.iter().map(|n| (u32::from(ip) ^ u32::from(*n)).leading_zeros()).sum()
    };
    // `max_by_key` keeps the last of equals; reversing keeps the first, which is the
    // station's own preference when the nodes cannot tell the doors apart.
    doors.iter().rev().max_by_key(|d| score(d)).cloned()
}

/// The configured port, open to the nodes, while this station drives them.
///
/// Dropping it closes the listener and every connection through it — which is what
/// a node has to see when the station it was talking to stops leading, or dies. An
/// aborted task drops its future, so a killed station closes its door too.
pub struct FrontDoor {
    accept: JoinHandle<()>,
}

impl FrontDoor {
    /// Open `door_port` on every interface, piped through to `broker`.
    ///
    /// Retried for a moment, because the station this one replaced on the same port
    /// may still be closing its own door: an abort lands at the next suspension.
    pub async fn open(door_port: u16, broker: SocketAddr) -> std::io::Result<FrontDoor> {
        let mut attempt = 0;
        let listener = loop {
            match TcpListener::bind((Ipv4Addr::UNSPECIFIED, door_port)).await {
                Ok(listener) => break listener,
                Err(e) if attempt < 40 => {
                    debug!("[devices] port {door_port} is not free yet: {e}");
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e),
            }
        };
        let accept = tokio::spawn(async move {
            // Held by the accept loop, so dropping the loop aborts every pipe in it.
            let mut pipes = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok((mut node, from)) => {
                            pipes.spawn(async move {
                                match tokio::net::TcpStream::connect(broker).await {
                                    Ok(mut inside) => {
                                        let _ = node.set_nodelay(true);
                                        let _ = inside.set_nodelay(true);
                                        let _ = tokio::io::copy_bidirectional(&mut node, &mut inside).await;
                                    }
                                    Err(e) => warn!("[devices] {from} reached the door but not the broker: {e}"),
                                }
                            });
                        }
                        Err(e) => {
                            warn!("[devices] the broker door stopped accepting: {e}");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    },
                    // Reap what has finished, so a night of reconnecting nodes is not a
                    // night of growing bookkeeping.
                    Some(_) = pipes.join_next(), if !pipes.is_empty() => {}
                }
            }
        });
        Ok(FrontDoor { accept })
    }
}

impl Drop for FrontDoor {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

fn config(listen: SocketAddr) -> rumqttd::Config {
    let connections = ConnectionSettings {
        connection_timeout_ms: 10_000,
        // A whole-strip color payload is a few dozen bytes; nothing here is large.
        max_payload_size: 16 * 1024,
        max_inflight_count: 100,
        auth: None,
        external_auth: None,
        dynamic_filters: true,
    };

    let mut v4 = HashMap::new();
    v4.insert(
        "openhaunt".to_string(),
        ServerSettings {
            name: "openhaunt".to_string(),
            listen,
            tls: None,
            next_connection_delay_ms: 1,
            connections: connections.clone(),
        },
    );

    rumqttd::Config {
        id: 0,
        router: RouterConfig {
            max_connections: 256,
            max_outgoing_packet_count: 1024,
            max_segment_size: 256 * 1024,
            max_segment_count: 8,
            ..Default::default()
        },
        v4: Some(v4),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_closed_door_drops_every_node_on_it_and_refuses_the_next() {
        let door_port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        // Anything that answers will do for the far side of a pipe.
        let inside = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker = inside.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = inside.accept().await.unwrap();
                tokio::spawn(async move {
                    let (mut r, mut w) = s.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });

        let door = FrontDoor::open(door_port, broker).await.unwrap();
        let mut node = tokio::net::TcpStream::connect(("127.0.0.1", door_port)).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        node.write_all(b"ping").await.unwrap();
        let mut back = [0u8; 4];
        node.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"ping", "an open door pipes through to the broker");

        drop(door);
        let mut rest = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(2), node.read_to_end(&mut rest)).await;
        assert!(matches!(read, Ok(Ok(0)) | Ok(Err(_))), "the node on it sees the door close");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", door_port)).await.is_err(),
            "and nobody gets in afterwards",
        );
    }
}
