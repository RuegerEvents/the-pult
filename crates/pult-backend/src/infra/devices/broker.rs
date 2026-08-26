//! The MQTT broker the leader runs for its OpenHaunt nodes.
//!
//! A node is discovered, not configured — which has to include not being told to
//! go and find a broker somebody installed first. So the console runs one itself
//! and hands out its address on adoption.
//!
//! rumqttd's `Broker::start` blocks for the life of the process, so it gets its own
//! thread. It is started once and never stopped: a node reconnecting to a broker
//! that was torn down and rebuilt on a leadership change would lose its retained
//! status for no gain, and an idle broker with no clients costs nothing.
//!
//! The config is built in Rust rather than parsed from TOML, so a shape change
//! between rumqttd releases is a compile error here rather than a runtime one on
//! someone's show night.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::OnceLock,
};

use rumqttd::{Broker, ConnectionSettings, RouterConfig, ServerSettings};
use tracing::{info, warn};

/// Where the running broker listens, once it has been started.
static RUNNING: OnceLock<SocketAddr> = OnceLock::new();

/// Start the broker if it is not already running, and say where it listens.
///
/// Idempotent by design: adoption calls this, and so does becoming the leader, and
/// neither knows what the other did.
pub fn ensure(port: u16) -> SocketAddr {
    *RUNNING.get_or_init(|| {
        let listen = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
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
            .map(|_| info!("[devices] MQTT broker on {listen}"))
            .unwrap_or_else(|e| warn!("[devices] could not start the MQTT broker: {e}"));

        listen
    })
}

/// Where a node should be told to publish. The broker binds `0.0.0.0`, so the
/// address handed out has to be one this console is actually reachable on.
pub fn advertised_addr(local_ip: std::net::IpAddr, port: u16) -> String {
    format!("{local_ip}:{port}")
}

fn config(listen: SocketAddr) -> rumqttd::Config {
    let connections = ConnectionSettings {
        connection_timeout_ms: 10_000,
        // A whole-strip colour payload is a few dozen bytes; nothing here is large.
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
