//! An OpenHaunt node, in software.
//!
//! There is no firmware yet, so this is what the-pult is developed and tested
//! against: the HTTP control API, the mDNS advertisement, the MQTT topics, and
//! sACN reception, as `OpenHaunt/node`'s docs describe them.
//!
//! It deliberately shares no code with the console. The module type ids and topic
//! shapes are written out again here from the same documents, so a test that drives
//! this and reads the console proves the two ends agree — which is the only thing
//! worth proving before there is hardware to disagree with.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use axum::{extract::State, routing::{get, post}, Json, Router};
use rumqttc::{AsyncClient, ConnectionError, Event, LastWill, MqttOptions, Packet, QoS};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

pub const SERVICE_TYPE: &str = "_openhaunt._tcp.local.";
/// The E1.31 port. Configurable here only so parallel tests can each have one.
pub const SACN_PORT: u16 = 5568;

// ── Modules ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    DmxOut,
    DigitalIn,
    Ws2812,
    MainsRelay,
    Oled,
    DryContact,
    Environment,
}

impl ModuleKind {
    pub fn type_id(self) -> u16 {
        match self {
            ModuleKind::DmxOut => 0x0001,
            ModuleKind::DigitalIn => 0x0002,
            ModuleKind::Ws2812 => 0x0003,
            ModuleKind::MainsRelay => 0x0004,
            ModuleKind::Oled => 0x0005,
            ModuleKind::DryContact => 0x0006,
            ModuleKind::Environment => 0x0007,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ModuleKind::DmxOut => "DMX Gateway",
            ModuleKind::DigitalIn => "Digital Inputs",
            ModuleKind::Ws2812 => "LED Strip",
            ModuleKind::MainsRelay => "Mains Relay",
            ModuleKind::Oled => "Display",
            ModuleKind::DryContact => "Dry Contacts",
            ModuleKind::Environment => "Environment Sensor",
        }
    }

    pub fn caps(self) -> &'static str {
        match self {
            ModuleKind::DmxOut => "dmx,rdm,sacn",
            _ => "",
        }
    }

    /// Descriptor flags. Bit 6 says the module switches mains.
    pub fn flags(self) -> u32 {
        match self {
            ModuleKind::MainsRelay => 1 << 6,
            _ => 0,
        }
    }

    /// The name the command line and the GUI use, and the inverse of [`parse`].
    pub fn key(self) -> &'static str {
        match self {
            ModuleKind::DmxOut => "dmx",
            ModuleKind::DigitalIn => "input",
            ModuleKind::Ws2812 => "led",
            ModuleKind::MainsRelay => "relay",
            ModuleKind::Oled => "oled",
            ModuleKind::DryContact => "contact",
            ModuleKind::Environment => "env",
        }
    }

    /// Every module, in the order a picker should offer them.
    pub const ALL: [ModuleKind; 7] = [
        ModuleKind::DigitalIn,
        ModuleKind::DryContact,
        ModuleKind::Environment,
        ModuleKind::MainsRelay,
        ModuleKind::Ws2812,
        ModuleKind::Oled,
        ModuleKind::DmxOut,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "dmx" => ModuleKind::DmxOut,
            "input" => ModuleKind::DigitalIn,
            "led" => ModuleKind::Ws2812,
            "relay" => ModuleKind::MainsRelay,
            "oled" => ModuleKind::Oled,
            "contact" => ModuleKind::DryContact,
            "env" => ModuleKind::Environment,
            _ => return None,
        })
    }
}

// ── Configuration ─────────────────────────────────────────────────────────────

pub struct SimConfig {
    pub module: ModuleKind,
    pub serial: String,
    pub name: String,
    /// 0 asks the OS for a free port, which is what tests want.
    pub http_port: u16,
    /// 0 likewise. The bin uses [`SACN_PORT`].
    pub sacn_port: u16,
    /// Register with mDNS. Off in tests, so nothing touches multicast.
    pub advertise: bool,
    /// Report a reading or toggle an input on this interval, unprompted.
    pub auto: Option<Duration>,
}

impl SimConfig {
    pub fn new(module: ModuleKind, serial: impl Into<String>) -> Self {
        let serial = serial.into();
        SimConfig {
            name: format!("{} {serial}", module.name()),
            module,
            serial,
            http_port: 0,
            sacn_port: 0,
            advertise: false,
            auto: None,
        }
    }
}

/// Everything a node knows about itself, in one value.
///
/// The protocol does not have this and does not need it — a console learns each
/// of these separately, over the wire, which is the point. It exists so that a
/// window onto a simulated node can show what the node is doing without having to
/// subscribe to five channels and stitch them together.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub serial: String,
    pub name: String,
    /// The module's short name, as `--module` takes it.
    pub module: String,
    pub module_name: String,
    pub type_id: u16,
    pub caps: String,
    /// Descriptor bit 6: this module switches mains, and a console should say so.
    pub switches_mains: bool,
    pub http_addr: String,
    pub sacn_addr: Option<String>,
    pub advertising: bool,
    /// Whether a console has ever sent `POST /api/v1/config`.
    pub adopted: bool,
    /// The broker it was told to publish to, which is what adoption amounts to.
    pub broker: Option<String>,
    pub mqtt_connected: bool,
    /// Output ports as the node holds them, keyed by port number.
    pub outputs: BTreeMap<String, Value>,
    /// The last thing published on each input port, keyed the same way.
    pub inputs: BTreeMap<String, Value>,
    /// How many times a console has asked this node to blink.
    pub identified: usize,
    /// Unix milliseconds, so the uptime shown is always current rather than as
    /// current as the last time anything happened.
    pub started_ms: u64,
}

/// What a test can see and do to a running node.
pub struct SimHandle {
    pub http_addr: SocketAddr,
    pub sacn_addr: Option<SocketAddr>,
    /// The last `POST /api/v1/config` body, or None if nobody has configured it.
    pub received_config: watch::Receiver<Option<Value>>,
    /// Output ports as the node currently holds them, keyed by port number.
    pub state: watch::Receiver<BTreeMap<String, Value>>,
    /// Universe and 512 channels, for every E1.31 frame received.
    pub sacn_frames: mpsc::Receiver<(u16, Vec<u8>)>,
    /// Drive an input, as a button or a sensor would.
    pub inputs: mpsc::Sender<Input>,
    /// The whole node in one value, for a window onto it. See [`Snapshot`].
    pub snapshot: watch::Receiver<Snapshot>,
}

/// Something happening at the terminals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// A contact opening or closing.
    Contact { port: u8, state: bool },
    /// A sensor reporting.
    Reading { port: u8, value: f32 },
}

// ── The node ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Node {
    module: ModuleKind,
    serial: String,
    name: String,
    config_tx: Arc<watch::Sender<Option<Value>>>,
    state_tx: Arc<watch::Sender<BTreeMap<String, Value>>>,
    identified: Arc<Mutex<usize>>,
    snapshot: Arc<watch::Sender<Snapshot>>,
}

impl Node {
    /// Change the node's own account of itself. Kept beside every write to the
    /// protocol state rather than derived from it, so that nothing a console can
    /// see depends on whether anybody is watching.
    fn describe(&self, change: impl FnOnce(&mut Snapshot)) {
        self.snapshot.send_modify(change);
    }
}

pub async fn start(config: SimConfig) -> Result<SimHandle> {
    let (config_tx, received_config) = watch::channel(None);
    let (state_tx, state) = watch::channel(BTreeMap::new());
    let (broker_tx, broker_rx) = watch::channel(None::<String>);
    let (inputs, inputs_rx) = mpsc::channel(64);
    let (frames_tx, sacn_frames) = mpsc::channel(64);
    let (snapshot_tx, snapshot) = watch::channel(Snapshot {
        serial: config.serial.clone(),
        name: config.name.clone(),
        module: config.module.key().to_string(),
        module_name: config.module.name().to_string(),
        type_id: config.module.type_id(),
        caps: config.module.caps().to_string(),
        switches_mains: config.module.flags() & (1 << 6) != 0,
        advertising: config.advertise,
        started_ms: now_ms(),
        ..Snapshot::default()
    });

    let node = Node {
        module: config.module,
        serial: config.serial.clone(),
        name: config.name.clone(),
        config_tx: Arc::new(config_tx),
        state_tx: Arc::new(state_tx),
        identified: Arc::new(Mutex::new(0)),
        snapshot: Arc::new(snapshot_tx),
    };

    let http_addr = serve_http(node.clone(), config.http_port, broker_tx).await?;

    let sacn_addr = match config.module {
        ModuleKind::DmxOut => Some(listen_for_sacn(config.sacn_port, frames_tx).await?),
        _ => None,
    };

    node.describe(|s| {
        s.http_addr = http_addr.to_string();
        s.sacn_addr = sacn_addr.map(|a| a.to_string());
    });

    tokio::spawn(run_mqtt(node, broker_rx, inputs_rx, config.auto));

    if config.advertise {
        advertise(&config, http_addr)?;
    }

    info!("[sim] {} on {http_addr}", config.serial);
    Ok(SimHandle { http_addr, sacn_addr, received_config, state, sacn_frames, inputs, snapshot })
}

// ── HTTP ──────────────────────────────────────────────────────────────────────

async fn serve_http(
    node: Node,
    port: u16,
    broker: watch::Sender<Option<String>>,
) -> Result<SocketAddr> {
    let broker = Arc::new(broker);
    let app = Router::new()
        .route("/api/v1/info", get(info))
        .route("/api/v1/state", get(read_state).post(write_state))
        .route(
            "/api/v1/config",
            post({
                let broker = broker.clone();
                move |State(node): State<Node>, Json(body): Json<Value>| {
                    let broker = broker.clone();
                    async move {
                        info!("[sim] {} configured: {body}", node.serial);
                        if let Some(addr) = body["mqtt"]["broker"].as_str() {
                            let _ = broker.send(Some(addr.to_string()));
                            node.describe(|s| s.broker = Some(addr.to_string()));
                        }
                        node.describe(|s| s.adopted = true);
                        let _ = node.config_tx.send(Some(body));
                        Json(json!({ "ok": true }))
                    }
                }
            }),
        )
        .route("/api/v1/identify", post(identify))
        .route("/api/v1/ota", post(|| async { Json(json!({ "ok": false, "error": "no ota here" })) }))
        .with_state(node);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("[sim] http stopped: {e}");
        }
    });
    Ok(addr)
}

async fn info(State(node): State<Node>) -> Json<Value> {
    Json(json!({
        "v": "1",
        "fw": "0.0.0-sim",
        "sn": node.serial,
        "name": node.name,
        "module": {
            "type": format!("{:#06x}", node.module.type_id()),
            "name": node.module.name(),
            "sn": format!("mod-{}", node.serial),
            "rev": "a",
            "flags": node.module.flags(),
            "caps": node.module.caps(),
        },
    }))
}

async fn read_state(State(node): State<Node>) -> Json<Value> {
    Json(json!({ "outputs": node.state_tx.borrow().clone() }))
}

async fn write_state(State(node): State<Node>, Json(body): Json<Value>) -> Json<Value> {
    if let Some(outputs) = body["outputs"].as_object() {
        node.state_tx.send_modify(|state| {
            for (port, value) in outputs {
                state.insert(port.clone(), value.clone());
            }
        });
        node.describe(|s| {
            for (port, value) in outputs {
                s.outputs.insert(port.clone(), value.clone());
            }
        });
    }
    Json(json!({ "ok": true }))
}

async fn identify(State(node): State<Node>) -> Json<Value> {
    *node.identified.lock().unwrap() += 1;
    node.describe(|s| s.identified += 1);
    info!("[sim] {} blinking", node.serial);
    Json(json!({ "ok": true }))
}

// ── MQTT ──────────────────────────────────────────────────────────────────────

/// Wait to be told where the broker is, then behave like a node on it.
async fn run_mqtt(
    node: Node,
    mut broker: watch::Receiver<Option<String>>,
    mut inputs: mpsc::Receiver<Input>,
    auto: Option<Duration>,
) {
    // Nothing is published before the console asks for it: a node is discovered,
    // not configured, and until it is configured it has nowhere to publish to.
    let addr = loop {
        if let Some(addr) = broker.borrow_and_update().clone() {
            break addr;
        }
        if broker.changed().await.is_err() {
            return;
        }
    };

    let status = format!("openhaunt/{}/status", node.serial);
    let (host, port) = split_addr(&addr);
    let mut options = MqttOptions::new(format!("openhaunt-{}", node.serial), host, port);
    options.set_keep_alive(Duration::from_secs(10));
    options.set_last_will(LastWill::new(&status, "offline", QoS::AtLeastOnce, true));

    let (client, mut eventloop) = AsyncClient::new(options, 32);
    let _ = client.publish(&status, QoS::AtLeastOnce, true, "offline").await;
    let _ = client.publish(&status, QoS::AtLeastOnce, true, "online").await;
    let _ = client
        .subscribe(format!("openhaunt/{}/output/+/set", node.serial), QoS::AtLeastOnce)
        .await;

    let mut health = tokio::time::interval(Duration::from_secs(10));
    let mut ticker = auto.map(tokio::time::interval);
    let started = tokio::time::Instant::now();
    let mut auto_state = false;

    loop {
        tokio::select! {
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    apply_output(&node, &publish.topic, &publish.payload);
                }
                // The one packet worth noticing on its own: until it arrives the
                // node is configured but not yet talking to anything.
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    node.describe(|s| s.mqtt_connected = true);
                }
                Ok(_) => {}
                Err(e) => {
                    connection_lost(&node, &e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            },

            Some(input) = inputs.recv() => {
                publish_input(&client, &node, input).await;
            }

            _ = health.tick() => {
                let payload = json!({
                    "uptime_s": started.elapsed().as_secs(),
                    "temp_c": 38.5,
                    "poe_class": 3,
                    "errors": [],
                });
                let topic = format!("openhaunt/{}/health", node.serial);
                let _ = client.publish(topic, QoS::AtLeastOnce, false, payload.to_string()).await;
            }

            _ = async { ticker.as_mut().unwrap().tick().await }, if ticker.is_some() => {
                auto_state = !auto_state;
                let input = match node.module {
                    ModuleKind::Environment => Input::Reading {
                        port: 0,
                        value: if auto_state { 21.5 } else { 22.0 },
                    },
                    _ => Input::Contact { port: 0, state: auto_state },
                };
                publish_input(&client, &node, input).await;
            }
        }
    }
}

fn connection_lost(node: &Node, error: &ConnectionError) {
    debug!("[sim] {} mqtt: {error}", node.serial);
    node.describe(|s| s.mqtt_connected = false);
}

async fn publish_input(client: &AsyncClient, node: &Node, input: Input) {
    let serial = &node.serial;
    let (port, payload) = match input {
        Input::Contact { port, state } => (
            port,
            json!({
                "state": state,
                "edge": if state { "rising" } else { "falling" },
                "ts": now_ms(),
            }),
        ),
        Input::Reading { port, value } => {
            (port, json!({ "value": value, "unit": "C", "ts": now_ms() }))
        }
    };
    let topic = format!("openhaunt/{serial}/input/{port}");
    info!("[sim] {serial} input {port} -> {payload}");
    node.describe(|s| {
        s.inputs.insert(port.to_string(), payload.clone());
    });
    let _ = client.publish(topic, QoS::AtLeastOnce, false, payload.to_string()).await;
}

/// `openhaunt/<sn>/output/<n>/set` → port `n`.
fn apply_output(node: &Node, topic: &str, payload: &[u8]) {
    let Some(port) = topic.split('/').nth(3) else { return };
    let Ok(value) = serde_json::from_slice::<Value>(payload) else { return };
    info!("[sim] {} output {port} <- {value}", node.serial);
    node.state_tx.send_modify(|state| {
        state.insert(port.to_string(), value.clone());
    });
    node.describe(|s| {
        s.outputs.insert(port.to_string(), value);
    });
}

// ── sACN ──────────────────────────────────────────────────────────────────────

/// Receive E1.31 and hand over the universe and its 512 channels.
///
/// Enough of the packet is checked to tell a real frame from a stray datagram, and
/// no more: this is a node under test, not a conformance suite.
async fn listen_for_sacn(port: u16, frames: mpsc::Sender<(u16, Vec<u8>)>) -> Result<SocketAddr> {
    let socket = tokio::net::UdpSocket::bind(("0.0.0.0", port)).await?;
    let addr = socket.local_addr()?;
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 1500];
        loop {
            let Ok(n) = socket.recv(&mut buffer).await else { break };
            if let Some(frame) = parse_e131(&buffer[..n]) {
                if frames.send(frame).await.is_err() {
                    break;
                }
            }
        }
    });
    Ok(addr)
}

/// Universe and channel data out of an E1.31 data packet.
pub fn parse_e131(packet: &[u8]) -> Option<(u16, Vec<u8>)> {
    // Root layer preamble, then the ACN packet identifier.
    if packet.len() < 126 || &packet[4..16] != b"ASC-E1.17\0\0\0" {
        return None;
    }
    let universe = u16::from_be_bytes([*packet.get(113)?, *packet.get(114)?]);
    // Property value count includes the start code byte at 125.
    let count = u16::from_be_bytes([*packet.get(123)?, *packet.get(124)?]) as usize;
    let channels = packet.get(126..126 + count.saturating_sub(1))?;
    Some((universe, channels.to_vec()))
}

// ── mDNS ──────────────────────────────────────────────────────────────────────

fn advertise(config: &SimConfig, http_addr: SocketAddr) -> Result<()> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let mut txt = std::collections::HashMap::new();
    txt.insert("v".to_string(), "1".to_string());
    txt.insert("fw".to_string(), "0.0.0-sim".to_string());
    txt.insert("sn".to_string(), config.serial.clone());
    txt.insert("mod".to_string(), format!("{:#06x}", config.module.type_id()));
    txt.insert("modname".to_string(), config.module.name().to_string());
    txt.insert("modsn".to_string(), format!("mod-{}", config.serial));
    txt.insert("modrev".to_string(), "a".to_string());
    txt.insert("caps".to_string(), config.module.caps().to_string());
    txt.insert("name".to_string(), config.name.clone());

    let info = mdns_sd::ServiceInfo::new(
        SERVICE_TYPE,
        &format!("openhaunt-{}", config.serial),
        &format!("openhaunt-{}.local.", config.serial),
        IpAddr::V4(local_ipv4()),
        http_addr.port(),
        txt,
    )?;
    daemon.register(info)?;
    // The daemon stops browsing and advertising when dropped, and a simulated node
    // lives for the life of the process.
    std::mem::forget(daemon);
    info!("[sim] advertising openhaunt-{}", config.serial);
    Ok(())
}

fn local_ipv4() -> Ipv4Addr {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|s| s.connect("8.8.8.8:80").ok().map(|_| s))
        .and_then(|s| s.local_addr().ok())
        .and_then(|a| match a.ip() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn split_addr(addr: &str) -> (String, u16) {
    match addr.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(1883)),
        None => (addr.to_string(), 1883),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_module_has_its_own_type_id() {
        let kinds = [
            ModuleKind::DmxOut,
            ModuleKind::DigitalIn,
            ModuleKind::Ws2812,
            ModuleKind::MainsRelay,
            ModuleKind::Oled,
            ModuleKind::DryContact,
            ModuleKind::Environment,
        ];
        let ids: std::collections::BTreeSet<u16> = kinds.iter().map(|k| k.type_id()).collect();
        assert_eq!(ids.len(), kinds.len());
    }

    #[test]
    fn only_the_relay_sets_the_mains_flag() {
        assert_eq!(ModuleKind::MainsRelay.flags(), 1 << 6);
        assert_eq!(ModuleKind::DryContact.flags(), 0);
    }

    #[test]
    fn a_datagram_that_is_not_e131_is_ignored() {
        assert_eq!(parse_e131(b"hello"), None);
        assert_eq!(parse_e131(&[0u8; 200]), None);
    }
}
