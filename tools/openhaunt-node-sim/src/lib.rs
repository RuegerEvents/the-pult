//! An OpenHaunt node, in software.
//!
//! There is no firmware yet, so this is what the-pult is developed and tested
//! against: the HTTP control API, the mDNS advertisement, the MQTT topics, and
//! sACN reception, as `OpenHaunt/node`'s docs describe them.
//!
//! It deliberately shares no code with the console. The module type ids, the port
//! descriptions and the topic shapes are written out again here from the same
//! documents, so a test that drives this and reads the console proves the two ends
//! agree — which is the only thing worth proving before there is hardware to
//! disagree with.
//!
//! Knowing what its own ports are is not a duplicate of anything: only the device
//! knows what it is, and the console reads that off `GET /api/v1/info` rather than
//! keeping a table of its own.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

pub mod motion;

use anyhow::Result;
use axum::{extract::State, routing::{get, post}, Json, Router};
use rumqttc::{AsyncClient, ConnectionError, Event, LastWill, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

pub const SERVICE_TYPE: &str = "_openhaunt._tcp.local.";
/// The E1.31 port. Configurable here only so parallel tests can each have one.
pub const SACN_PORT: u16 = 5568;

// ── What a node says about itself ─────────────────────────────────────────────

/// One terminal, in the words `GET /api/v1/info` uses: E1.73 UDR's `access`,
/// `dataType`, `unit` and range, plus a `class` hint and `color` as a data type.
///
/// Owned strings rather than `&'static str`, because a node's ports are not
/// limited to the ones written down here: a config file can declare a terminal
/// this file has never heard of, which is the whole point of a node describing
/// itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortDescription {
    pub port: u8,
    pub name: String,
    /// `readonly` — the node writes it — or `readwrite`, which the console drives.
    pub access: String,
    /// `boolean`, `number`, `string` or `color`.
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// What this port can trace for itself, if anything.
    ///
    /// Absent is the default and means the console renders every value and streams
    /// it, which is what every node did before any of this existed. Saying so per
    /// port rather than per node is what lets one module have a strip that can trace
    /// a sine beside a relay that can only be chopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<motion::PortEffects>,
}

impl PortDescription {
    pub fn new(
        port: u8,
        name: impl Into<String>,
        access: impl Into<String>,
        data_type: impl Into<String>,
    ) -> Self {
        PortDescription {
            port,
            name: name.into(),
            access: access.into(),
            data_type: data_type.into(),
            unit: None,
            minimum: None,
            maximum: None,
            default: None,
            class: None,
            effects: None,
        }
    }

    /// Declare what this port can trace.
    pub fn effects(mut self, effects: motion::PortEffects) -> Self {
        self.effects = Some(effects);
        self
    }

    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }

    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn range(mut self, minimum: f64, maximum: f64) -> Self {
        self.minimum = Some(minimum);
        self.maximum = Some(maximum);
        self
    }

    pub fn default_at(mut self, default: f64) -> Self {
        self.default = Some(default);
        self
    }

    /// A port the console reads, as opposed to one it drives.
    pub fn is_input(&self) -> bool {
        self.access == "readonly"
    }
}

/// The universe a gateway forwards. Present only on a node that gateways one, and
/// that is what tells a console to allocate it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmxDescription {
    pub protocols: Vec<String>,
    pub universes: u16,
}

/// The module descriptor, as `GET /api/v1/info` and the TXT record report it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDescriptor {
    /// The type id, written `0x0003`. Nothing here looks it up: it is an
    /// inventory key and a mains warning, not the basis for control.
    #[serde(rename = "type", with = "hex_u16")]
    pub type_id: u16,
    pub name: String,
    /// Hardware revision, as the EEPROM records it.
    #[serde(default = "default_rev")]
    pub rev: String,
    /// The capability bitfield, verbatim. Bit 6 says the module switches mains.
    #[serde(default)]
    pub flags: u32,
    /// The `caps` shortlist, comma-separated, for a controller filtering a list.
    #[serde(default)]
    pub caps: String,
}

fn default_rev() -> String {
    "a".to_string()
}

impl ModuleDescriptor {
    pub fn switches_mains(&self) -> bool {
        self.flags & MAINS_FLAG != 0
    }
}

/// Descriptor bit 6: this module switches mains voltage.
pub const MAINS_FLAG: u32 = 1 << 6;

/// Everything a simulated node is, in one value that round-trips through JSON.
///
/// This is the config file. A node is its identity, its module descriptor, and
/// the terminals it describes — and since only the device knows what it is, a
/// file that says so is the whole of what makes one node different from another.
/// Nothing here is looked up in a table: a config can declare a module type this
/// crate has never heard of, with ports to match, and it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig {
    /// The user-assigned friendly name, as the `name` TXT key carries it.
    pub name: String,
    /// The node serial. Identifies it everywhere: mDNS instance, MQTT topics.
    pub serial: String,
    pub module: ModuleDescriptor,
    #[serde(default)]
    pub ports: Vec<PortDescription>,
    /// Present only on a node that forwards a universe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dmx: Option<DmxDescription>,
    /// HTTP control port. 0 asks the OS for a free one.
    #[serde(default)]
    pub http_port: u16,
    /// Register with mDNS.
    #[serde(default = "yes")]
    pub advertise: bool,
    /// Report a reading or toggle an input every this many milliseconds,
    /// unprompted. None leaves the node quiet until something drives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_ms: Option<u64>,
}

fn yes() -> bool {
    true
}

impl NodeConfig {
    /// Read a config file. Any parse trouble names the file, because the usual
    /// mistake is having edited the wrong one.
    pub fn read(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    /// Write a config file, pretty and with a trailing newline, because these are
    /// meant to be read and edited by hand as much as by this window.
    pub fn write(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let path = path.as_ref();
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Ok(())
    }

    /// The ports the console reads, in the order the node numbered them.
    pub fn inputs(&self) -> impl Iterator<Item = &PortDescription> {
        self.ports.iter().filter(|p| p.is_input())
    }

    /// Whatever is wrong with this config, in words. Empty means it is runnable.
    ///
    /// Kept out of `Deserialize` on purpose: a file with a duplicate port number
    /// should load into the editor so it can be fixed, not be refused at the door.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.serial.trim().is_empty() {
            problems.push("a node needs a serial: it is what its topics are keyed by".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for port in &self.ports {
            if !seen.insert(port.port) {
                problems.push(format!("two ports are numbered {}", port.port));
            }
            if !matches!(port.access.as_str(), "readonly" | "readwrite") {
                problems.push(format!(
                    "port {} has access {:?}; UDR has readonly and readwrite",
                    port.port, port.access,
                ));
            }
            if !matches!(port.data_type.as_str(), "boolean" | "number" | "string" | "color") {
                problems.push(format!(
                    "port {} has dataType {:?}; the protocol has boolean, number, string and color",
                    port.port, port.data_type,
                ));
            }
            if let Some(effects) = &port.effects {
                // A console only ever hands a shape to a port it drives. Advertising
                // on a port it reads is not dangerous, it is just a promise nothing
                // will ever ask this node to keep.
                if port.is_input() {
                    problems.push(format!(
                        "port {} is readonly but advertises effects; nothing drives an input",
                        port.port,
                    ));
                }
                // A string has no midpoint and no ordering, so there is no shape to
                // trace between two of them. Steps are a different matter: a list of
                // messages shown in turn is a perfectly good chase.
                if port.data_type == "string" && !effects.shapes.is_empty() {
                    problems.push(format!(
                        "port {} is a string but lists shapes; only steps make sense on one",
                        port.port,
                    ));
                }
            }
        }
        problems
    }
}

/// `0x0003` on the wire, a `u16` in here. The protocol writes module type ids in
/// hex and a config file should look like the protocol.
mod hex_u16 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{value:#06x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        // A number is accepted too: a file written by hand is allowed to say 4.
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Number(n) => n
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| serde::de::Error::custom("module type out of range")),
            serde_json::Value::String(raw) => {
                let raw = raw.trim();
                match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
                    Some(hex) => u16::from_str_radix(hex, 16),
                    None => raw.parse(),
                }
                .map_err(|_| serde::de::Error::custom(format!("not a module type: {raw}")))
            }
            _ => Err(serde::de::Error::custom("module type is a hex string")),
        }
    }
}

// ── Presets ───────────────────────────────────────────────────────────────────
//
// The seven modules in `OpenHaunt/node`'s catalogue, as somewhere to start from.
// They are a convenience of this simulator and nothing more: the node runs on a
// [`NodeConfig`], and a preset is only one way of arriving at one. Nothing in the
// running node asks which module kind it came from.

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
            ModuleKind::MainsRelay => MAINS_FLAG,
            _ => 0,
        }
    }

    /// This preset as a config: a whole node, ready to run or to edit.
    pub fn config(self, serial: impl Into<String>) -> NodeConfig {
        let serial = serial.into();
        NodeConfig {
            name: format!("{} {serial}", self.name()),
            module: ModuleDescriptor {
                type_id: self.type_id(),
                name: self.name().to_string(),
                rev: "a".to_string(),
                flags: self.flags(),
                caps: self.caps().to_string(),
            },
            ports: self.ports(),
            dmx: self.dmx(),
            serial,
            http_port: 0,
            advertise: false,
            auto_ms: None,
        }
    }

    /// What this module's terminals are, as the node itself reports them.
    ///
    /// Written from the module documents, because this is the firmware's side of
    /// the contract: the driver for a module type knows its ports and says so.
    pub fn ports(self) -> Vec<PortDescription> {
        use PortDescription as P;
        match self {
            // A gateway has no ports of its own. The lights behind it are patched
            // as their own DMX fixtures, in the universe it forwards.
            ModuleKind::DmxOut => Vec::new(),
            ModuleKind::DigitalIn => (0..8)
                .map(|n| {
                    P::new(n, INPUT_NAMES[n as usize], "readonly", "boolean").class("contact")
                })
                .collect(),
            // The strip is the module that can do all of it: both ports take a
            // shape, a step list and a timed move.
            ModuleKind::Ws2812 => vec![
                P::new(0, "Strip colour", "readwrite", "color")
                    .class("color")
                    .effects(motion::PortEffects::all()),
                P::new(1, "Brightness", "readwrite", "number")
                    .unit("percent")
                    .range(0.0, 1.0)
                    .default_at(0.0)
                    .class("intensity")
                    .effects(motion::PortEffects::all()),
            ],
            // A relay has two states, so it can be chopped and it can be stepped,
            // and there is nothing in between for a sine to trace or a fade to cross.
            ModuleKind::MainsRelay => vec![P::new(0, "Relay", "readwrite", "boolean")
                .default_at(0.0)
                .class("switch")
                .effects(motion::PortEffects::switching())],
            ModuleKind::Oled => vec![P::new(0, "Line", "readwrite", "string").class("text")],
            ModuleKind::DryContact => (0..4)
                .map(|n| {
                    P::new(n, CONTACT_NAMES[n as usize], "readwrite", "boolean")
                        .default_at(0.0)
                        .class("switch")
                        .effects(motion::PortEffects::switching())
                })
                .collect(),
            ModuleKind::Environment => vec![
                P::new(0, "Temperature", "readonly", "number")
                    .unit("degree-celsius")
                    .range(-40.0, 85.0)
                    .class("temperature"),
                P::new(1, "Humidity", "readonly", "number")
                    .unit("percent")
                    .range(0.0, 100.0)
                    .class("humidity"),
                P::new(2, "Air quality", "readonly", "number")
                    .unit("parts-per-million")
                    .range(0.0, 5000.0)
                    .class("air-quality"),
            ],
        }
    }

    /// The universe this module forwards, if it forwards one.
    pub fn dmx(self) -> Option<DmxDescription> {
        match self {
            ModuleKind::DmxOut => Some(DmxDescription {
                protocols: vec!["sacn".to_string(), "artnet".to_string()],
                universes: 1,
            }),
            _ => None,
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

/// Port names, spelled out rather than formatted, so a description is made of
/// `&'static str` and costs nothing to hand out.
const INPUT_NAMES: [&str; 8] = [
    "Input 1", "Input 2", "Input 3", "Input 4", "Input 5", "Input 6", "Input 7", "Input 8",
];

const CONTACT_NAMES: [&str; 4] = ["Contact 1", "Contact 2", "Contact 3", "Contact 4"];

// ── Configuration ─────────────────────────────────────────────────────────────

/// How to run a node here, as opposed to what the node *is*.
///
/// [`NodeConfig`] is the part that goes in a file and describes a device;
/// `sacn_port` is a fact about this machine, so it stays out of it.
pub struct SimConfig {
    pub node: NodeConfig,
    /// 0 asks the OS for a free port, which is what tests want. The bin uses
    /// [`SACN_PORT`].
    pub sacn_port: u16,
}

impl SimConfig {
    /// A node from one of the catalogue presets, quiet and on an ephemeral port —
    /// which is what a test wants, and what everything else overrides.
    pub fn new(module: ModuleKind, serial: impl Into<String>) -> Self {
        SimConfig { node: module.config(serial), sacn_port: 0 }
    }
}

impl From<NodeConfig> for SimConfig {
    fn from(node: NodeConfig) -> Self {
        SimConfig { node, sacn_port: SACN_PORT }
    }
}

/// Everything a node knows about itself, in one value.
///
/// The protocol does not have this and does not need it — a console learns each
/// of these separately, over the wire, which is the point. It exists so that a
/// window onto a simulated node can show what the node is doing without having to
/// subscribe to five channels and stitch them together.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// What this node is: identity, module descriptor, ports, and the rest of
    /// what a config file holds. One value rather than a scattering of copies, so
    /// a window that wants to edit the node is editing the same thing that runs.
    pub config: NodeConfig,
    pub http_addr: String,
    pub sacn_addr: Option<String>,
    /// Whether a console has ever sent `POST /api/v1/config`.
    pub adopted: bool,
    /// The broker it was told to publish to, which is what adoption amounts to.
    pub broker: Option<String>,
    pub mqtt_connected: bool,
    /// Output ports as the node holds them, keyed by port number.
    pub outputs: BTreeMap<String, Value>,
    /// What each port is tracing on its own, keyed by port. The values in `outputs`
    /// are still the truth about where the port is; this says why they are moving.
    pub effects: BTreeMap<String, Value>,
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
    /// Put the node away: its HTTP socket, its mDNS registration, its MQTT
    /// connection, its sACN socket. What a window needs in order to start a
    /// differently-configured node in its place without closing.
    pub stop: Stopper,
}

/// Everything a running node is holding, and one way to let go of it.
///
/// A node is sockets and a name on the network, so stopping it is not a matter of
/// dropping a value: the mDNS registration has to be withdrawn or the record
/// lingers in every browser's cache, and the ports have to actually be free
/// before anything tries to bind them again. [`Stopper::stop`] waits for that.
#[derive(Clone, Default)]
pub struct Stopper {
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    mdns: Arc<Mutex<Option<(mdns_sd::ServiceDaemon, String)>>>,
}

impl Stopper {
    fn holds(&self, task: tokio::task::JoinHandle<()>) {
        self.tasks.lock().unwrap().push(task);
    }

    fn advertises(&self, daemon: mdns_sd::ServiceDaemon, fullname: String) {
        *self.mdns.lock().unwrap() = Some((daemon, fullname));
    }

    /// Stop the node and wait for it to have stopped.
    ///
    /// Aborting rather than asking politely: this is a simulator, there is no
    /// in-flight work worth finishing, and a caller about to rebind port 5568
    /// cares only that the old socket is gone.
    pub async fn stop(&self) {
        if let Some((daemon, fullname)) = self.mdns.lock().unwrap().take() {
            // Withdrawing the record is worth doing even though the daemon is
            // about to go: a console that browsed us should hear we left.
            if let Ok(receiver) = daemon.unregister(&fullname) {
                let _ = receiver.recv_timeout(Duration::from_millis(500));
            }
            let _ = daemon.shutdown();
        }
        let tasks: Vec<_> = std::mem::take(&mut *self.tasks.lock().unwrap());
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
    }
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
    /// What this node is. Read for every answer it gives, because a node that
    /// describes itself has nowhere else to get the answer from.
    config: Arc<NodeConfig>,
    config_tx: Arc<watch::Sender<Option<Value>>>,
    state_tx: Arc<watch::Sender<BTreeMap<String, Value>>>,
    identified: Arc<Mutex<usize>>,
    snapshot: Arc<watch::Sender<Snapshot>>,
    /// What each port is doing that one value cannot describe.
    motions: Arc<Mutex<BTreeMap<u8, motion::Motion>>>,
    /// What this node thinks the console's clock reads.
    clock: Arc<Mutex<motion::ClockOffset>>,
}

impl Node {
    /// Change the node's own account of itself. Kept beside every write to the
    /// protocol state rather than derived from it, so that nothing a console can
    /// see depends on whether anybody is watching.
    fn describe(&self, change: impl FnOnce(&mut Snapshot)) {
        self.snapshot.send_modify(change);
    }

    /// Put a value on a port.
    ///
    /// Everything that changes an output goes through here — a `set` off MQTT, a
    /// `POST /state`, and the renderer forty times a second — so there is one answer
    /// to "where is this port" rather than one per caller.
    fn write_port(&self, port: &str, value: Value) {
        self.state_tx.send_modify(|state| {
            state.insert(port.to_string(), value.clone());
        });
        self.describe(|s| {
            s.outputs.insert(port.to_string(), value);
        });
    }

    fn port_value(&self, port: &str) -> Option<Value> {
        self.state_tx.borrow().get(port).cloned()
    }

    /// Start, replace, or stop what a port is tracing, and say so in the snapshot.
    fn set_motion(&self, port: u8, what: Option<motion::Motion>) {
        {
            let mut motions = self.motions.lock().unwrap();
            match what {
                Some(motion) => motions.insert(port, motion),
                None => motions.remove(&port),
            };
        }
        let described = motion::describe_all(&self.motions.lock().unwrap());
        self.describe(|s| s.effects = described);
    }

    /// The console's clock, as best this node can tell.
    fn console_now(&self) -> i64 {
        self.clock.lock().unwrap().console_now(now_ms() as i64)
    }
}

/// Trace whatever the ports have been given, on this node's own initiative.
///
/// This is the whole point of the exercise: nothing on the network says anything
/// while this runs. It writes through the same store a `set` writes to, so
/// `GET /api/v1/state` and the panel both see a port that is genuinely moving rather
/// than a description of one that might be.
async fn run_renderer(node: Node) {
    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        let now = node.console_now();

        let due: Vec<(u8, motion::Motion)> = {
            let motions = node.motions.lock().unwrap();
            motions.iter().map(|(port, m)| (*port, m.clone())).collect()
        };
        if due.is_empty() {
            continue;
        }

        let mut finished = Vec::new();
        for (port, what) in due {
            let key = port.to_string();
            match &what {
                motion::Motion::Effect(effect) => {
                    let value = effect.sample(now);
                    // A port already showing this value is left alone: at 40 Hz a
                    // square wave is the same answer for twenty ticks running.
                    if node.port_value(&key).as_ref() != Some(&value) {
                        node.write_port(&key, value);
                    }
                }
                motion::Motion::Transition(transition) => {
                    let (value, done) = transition.sample(now);
                    if node.port_value(&key).as_ref() != Some(&value) {
                        node.write_port(&key, value);
                    }
                    // A fade that has arrived is over. An effect never is.
                    if done {
                        finished.push(port);
                    }
                }
            }
        }
        for port in finished {
            node.set_motion(port, None);
        }
    }
}

pub async fn start(config: SimConfig) -> Result<SimHandle> {
    let SimConfig { node: node_config, sacn_port } = config;
    let node_config = Arc::new(node_config);

    let (config_tx, received_config) = watch::channel(None);
    let (state_tx, state) = watch::channel(BTreeMap::new());
    let (broker_tx, broker_rx) = watch::channel(None::<String>);
    let (inputs, inputs_rx) = mpsc::channel(64);
    let (frames_tx, sacn_frames) = mpsc::channel(64);
    let (snapshot_tx, snapshot) = watch::channel(Snapshot {
        config: (*node_config).clone(),
        http_addr: String::new(),
        sacn_addr: None,
        adopted: false,
        broker: None,
        mqtt_connected: false,
        outputs: BTreeMap::new(),
        effects: BTreeMap::new(),
        inputs: BTreeMap::new(),
        identified: 0,
        started_ms: now_ms(),
    });

    let node = Node {
        config: node_config.clone(),
        config_tx: Arc::new(config_tx),
        state_tx: Arc::new(state_tx),
        identified: Arc::new(Mutex::new(0)),
        snapshot: Arc::new(snapshot_tx),
        motions: Arc::new(Mutex::new(BTreeMap::new())),
        clock: Arc::new(Mutex::new(motion::ClockOffset::default())),
    };
    let stop = Stopper::default();

    let http_addr = serve_http(node.clone(), &stop, broker_tx).await?;

    // A node listens for sACN exactly when it says it forwards a universe. There
    // is no module id involved: the description is the whole of the answer.
    let sacn_addr = match node_config.dmx {
        Some(_) => Some(listen_for_sacn(sacn_port, &stop, frames_tx).await?),
        None => None,
    };

    node.describe(|s| {
        s.http_addr = http_addr.to_string();
        s.sacn_addr = sacn_addr.map(|a| a.to_string());
    });

    // Before the MQTT task, so a descriptor that arrives in the first millisecond
    // has something already running to be picked up by.
    stop.holds(tokio::spawn(run_renderer(node.clone())));

    let auto = node_config.auto_ms.map(Duration::from_millis);
    stop.holds(tokio::spawn(run_mqtt(node, broker_rx, inputs_rx, auto)));

    if node_config.advertise {
        advertise(&node_config, http_addr, &stop)?;
    }

    info!("[sim] {} on {http_addr}", node_config.serial);
    Ok(SimHandle {
        http_addr,
        sacn_addr,
        received_config,
        state,
        sacn_frames,
        inputs,
        snapshot,
        stop,
    })
}

// ── HTTP ──────────────────────────────────────────────────────────────────────

async fn serve_http(
    node: Node,
    stop: &Stopper,
    broker: watch::Sender<Option<String>>,
) -> Result<SocketAddr> {
    let port = node.config.http_port;
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
                        info!("[sim] {} configured: {body}", node.config.serial);
                        if let Some(addr) = body["mqtt"]["broker"].as_str() {
                            let _ = broker.send(Some(addr.to_string()));
                            node.describe(|s| s.broker = Some(addr.to_string()));
                        }
                        // A node accepts `dmx` only where its own description said
                        // it forwards a universe. Anything else is a console that
                        // guessed, and a node that says so is easier to debug than
                        // one that quietly forwards nothing.
                        if !body["dmx"].is_null() && node.config.dmx.is_none() {
                            warn!(
                                "[sim] {} was sent dmx config but describes no universe: {}",
                                node.config.serial, body["dmx"],
                            );
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
    stop.holds(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("[sim] http stopped: {e}");
        }
    }));
    Ok(addr)
}

async fn info(State(node): State<Node>) -> Json<Value> {
    let module = &node.config.module;
    let mut info = json!({
        "v": "1",
        "fw": "0.0.0-sim",
        "sn": node.config.serial,
        "name": node.config.name,
        "module": {
            "type": format!("{:#06x}", module.type_id),
            "name": module.name,
            "sn": format!("mod-{}", node.config.serial),
            "rev": module.rev,
            "flags": module.flags,
            "caps": module.caps,
        },
        // Only the device knows what it is. A controller reads this and builds its
        // fixture type from it; it carries no catalogue of module types.
        "ports": node.config.ports,
    });
    // `dmx` is present only on a node that forwards a universe: its absence is
    // what tells a console there is none to allocate.
    if let Some(dmx) = &node.config.dmx {
        info["dmx"] = json!(dmx);
    }
    Json(info)
}

async fn read_state(State(node): State<Node>) -> Json<Value> {
    let mut body = json!({ "outputs": node.state_tx.borrow().clone() });
    let motions = motion::describe_all(&node.motions.lock().unwrap());
    if !motions.is_empty() {
        body["effects"] = json!(motions);
    }
    if let Some(offset) = node.clock.lock().unwrap().offset_ms() {
        body["consoleOffsetMs"] = json!(offset);
    }
    Json(body)
}

async fn write_state(State(node): State<Node>, Json(body): Json<Value>) -> Json<Value> {
    if let Some(outputs) = body["outputs"].as_object() {
        for (port, value) in outputs {
            apply_set(&node, port, value);
        }
    }
    // The same two things a node hears over MQTT, for a console that has adopted it
    // but has not yet told it where the broker is.
    if let Some(effects) = body["effects"].as_object() {
        for (port, descriptor) in effects {
            apply_effect(&node, port, descriptor);
        }
    }
    Json(json!({ "ok": true }))
}

async fn identify(State(node): State<Node>) -> Json<Value> {
    *node.identified.lock().unwrap() += 1;
    node.describe(|s| s.identified += 1);
    info!("[sim] {} blinking", node.config.serial);
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

    let serial = node.config.serial.clone();
    let status = format!("openhaunt/{serial}/status");
    let (host, port) = split_addr(&addr);
    let mut options = MqttOptions::new(format!("openhaunt-{serial}"), host, port);
    options.set_keep_alive(Duration::from_secs(10));
    options.set_last_will(LastWill::new(&status, "offline", QoS::AtLeastOnce, true));

    let (client, mut eventloop) = AsyncClient::new(options, 32);
    let _ = client.publish(&status, QoS::AtLeastOnce, true, "offline").await;
    let _ = client.publish(&status, QoS::AtLeastOnce, true, "online").await;
    // Both verbs, not just `set`: a port can be handed a shape as well as a value.
    let _ = client
        .subscribe(format!("openhaunt/{serial}/output/+/+"), QoS::AtLeastOnce)
        .await;
    // Not under this node's serial. The console publishes one clock for every node
    // on the broker, because the whole point is that they agree with each other.
    let _ = client.subscribe(motion::CLOCK_TOPIC, QoS::AtLeastOnce).await;

    let mut health = tokio::time::interval(Duration::from_secs(10));
    let mut ticker = auto.map(tokio::time::interval);
    let started = tokio::time::Instant::now();
    let mut auto_state = false;

    loop {
        tokio::select! {
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    if publish.topic == motion::CLOCK_TOPIC {
                        take_clock_sample(&node, &publish.payload, publish.retain);
                    } else {
                        apply_output(&node, &publish.topic, &publish.payload);
                    }
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
                let topic = format!("openhaunt/{serial}/health");
                let _ = client.publish(topic, QoS::AtLeastOnce, false, payload.to_string()).await;
            }

            _ = async { ticker.as_mut().unwrap().tick().await }, if ticker.is_some() => {
                auto_state = !auto_state;
                // Whatever this node said it reads, driven the way its data type
                // asks — a boolean toggles, a number walks its declared range.
                if let Some(input) = unprompted(&node, auto_state) {
                    publish_input(&client, &node, input).await;
                }
            }
        }
    }
}

/// The reading or edge a node with `--auto` reports on its own, on the first port
/// it described as `readonly`. None where the module reads nothing.
fn unprompted(node: &Node, state: bool) -> Option<Input> {
    let port = node.config.inputs().next()?;
    Some(match port.data_type.as_str() {
        "boolean" => Input::Contact { port: port.port, state },
        _ => {
            let low = port.minimum.unwrap_or(0.0);
            let high = port.maximum.unwrap_or(1.0);
            // A third of the way up and back down: plainly moving, plainly in range.
            let value = low + (high - low) * if state { 0.33 } else { 0.4 };
            Input::Reading { port: port.port, value: value as f32 }
        }
    })
}

/// One `openhaunt/clock` message.
///
/// `retain` matters: the broker replays the last one on subscribe, and it was
/// published at an unknown time in the past. It seeds the estimate and is never
/// allowed to correct it.
fn take_clock_sample(node: &Node, payload: &[u8], retained: bool) {
    let Ok(body) = serde_json::from_slice::<Value>(payload) else { return };
    let (Some(t), Some(seq)) = (body["t"].as_i64(), body["seq"].as_u64()) else { return };
    let mut clock = node.clock.lock().unwrap();
    clock.feed(t, seq, now_ms() as i64, retained);
    debug!("[sim] {} console offset {:?} ms", node.config.serial, clock.offset_ms());
}

fn connection_lost(node: &Node, error: &ConnectionError) {
    debug!("[sim] {} mqtt: {error}", node.config.serial);
    node.describe(|s| s.mqtt_connected = false);
}

async fn publish_input(client: &AsyncClient, node: &Node, input: Input) {
    let serial = &node.config.serial;
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
            // The unit is the one this node published in its own description, not
            // a literal: whatever `/api/v1/info` said about the port is what a
            // reading off it carries.
            let unit = node
                .config
                .ports
                .iter()
                .find(|p| p.port == port)
                .and_then(|p| p.unit.as_deref())
                .unwrap_or("unitless");
            (port, json!({ "value": value, "unit": unit, "ts": now_ms() }))
        }
    };
    let topic = format!("openhaunt/{serial}/input/{port}");
    info!("[sim] {serial} input {port} -> {payload}");
    node.describe(|s| {
        s.inputs.insert(port.to_string(), payload.clone());
    });
    let _ = client.publish(topic, QoS::AtLeastOnce, false, payload.to_string()).await;
}

/// `openhaunt/<sn>/output/<n>/{set,effect}` → port `n`.
fn apply_output(node: &Node, topic: &str, payload: &[u8]) {
    let mut parts = topic.split('/').skip(3);
    let Some(port) = parts.next() else { return };
    let verb = parts.next().unwrap_or("set");
    let Ok(body) = serde_json::from_slice::<Value>(payload) else { return };
    match verb {
        "effect" => apply_effect(node, port, &body),
        _ => apply_set(node, port, &body),
    }
}

/// A value, with or without a time to reach it by.
///
/// A `set` carrying no timing at all is the path this node has always had: apply it
/// and be done. Either way it cancels whatever the port was tracing — a console that
/// has decided to send a value has taken the port back, and a shape still running
/// underneath would overwrite it on the next tick.
fn apply_set(node: &Node, port: &str, body: &Value) {
    let Ok(number) = port.parse::<u8>() else { return };
    let from = node.port_value(port).unwrap_or_else(|| json!({ "value": 0.0 }));

    match motion::parse_transition(body, from, node.console_now()) {
        Some(transition) => {
            info!(
                "[sim] {} output {port} fades to {} over {} ms",
                node.config.serial, transition.to, transition.duration_ms,
            );
            node.set_motion(number, Some(motion::Motion::Transition(transition)));
        }
        None => {
            info!("[sim] {} output {port} <- {body}", node.config.serial);
            node.set_motion(number, None);
            node.write_port(port, body.clone());
        }
    }
}

/// A shape to trace, or `{"clear": true}` to stop.
///
/// Clearing leaves the port wherever the shape had got to. The console follows a
/// clear with a value precisely because of that: this node has no opinion about where
/// a stopped effect should leave things.
fn apply_effect(node: &Node, port: &str, body: &Value) {
    let Ok(number) = port.parse::<u8>() else { return };
    match motion::parse_effect(body) {
        Some(effect) => {
            info!("[sim] {} output {port} traces {body}", node.config.serial);
            node.set_motion(number, Some(motion::Motion::Effect(effect)));
        }
        None => {
            info!("[sim] {} output {port} stops tracing", node.config.serial);
            node.set_motion(number, None);
        }
    }
}

// ── sACN ──────────────────────────────────────────────────────────────────────

/// Receive E1.31 and hand over the universe and its 512 channels.
///
/// Enough of the packet is checked to tell a real frame from a stray datagram, and
/// no more: this is a node under test, not a conformance suite.
async fn listen_for_sacn(
    port: u16,
    stop: &Stopper,
    frames: mpsc::Sender<(u16, Vec<u8>)>,
) -> Result<SocketAddr> {
    let socket = tokio::net::UdpSocket::bind(("0.0.0.0", port)).await?;
    let addr = socket.local_addr()?;
    stop.holds(tokio::spawn(async move {
        let mut buffer = vec![0u8; 1500];
        loop {
            let Ok(n) = socket.recv(&mut buffer).await else { break };
            if let Some(frame) = parse_e131(&buffer[..n]) {
                if frames.send(frame).await.is_err() {
                    break;
                }
            }
        }
    }));
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

fn advertise(config: &NodeConfig, http_addr: SocketAddr, stop: &Stopper) -> Result<()> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let mut txt = std::collections::HashMap::new();
    txt.insert("v".to_string(), "1".to_string());
    txt.insert("fw".to_string(), "0.0.0-sim".to_string());
    txt.insert("sn".to_string(), config.serial.clone());
    txt.insert("mod".to_string(), format!("{:#06x}", config.module.type_id));
    txt.insert("modname".to_string(), config.module.name.clone());
    txt.insert("modsn".to_string(), format!("mod-{}", config.serial));
    txt.insert("modrev".to_string(), config.module.rev.clone());
    txt.insert("caps".to_string(), config.module.caps.clone());
    txt.insert("name".to_string(), config.name.clone());
    // The one fact worth stating before anybody asks: a mains warning is more use
    // early than correct-and-late.
    if config.module.switches_mains() {
        txt.insert("mains".to_string(), "1".to_string());
    }

    let info = mdns_sd::ServiceInfo::new(
        SERVICE_TYPE,
        &format!("openhaunt-{}", config.serial),
        &format!("openhaunt-{}.local.", config.serial),
        IpAddr::V4(local_ipv4()),
        http_addr.port(),
        txt,
    )?;
    let fullname = info.get_fullname().to_string();
    daemon.register(info)?;
    // Held rather than forgotten: a node that is being reconfigured has to be able
    // to withdraw its record, or the old shape lingers in every browser's cache.
    stop.advertises(daemon, fullname);
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
    fn every_preset_numbers_its_ports_from_zero_and_is_runnable() {
        for module in ModuleKind::ALL {
            let config = module.config("1a2b3c");
            for (index, port) in config.ports.iter().enumerate() {
                assert_eq!(port.port, index as u8, "{} port {index}", module.key());
            }
            assert!(config.problems().is_empty(), "{}: {:?}", module.key(), config.problems());
        }
    }

    #[test]
    fn a_config_round_trips_through_the_file_it_is_written_to() {
        let before = ModuleKind::Environment.config("9a9a9a");
        let json = serde_json::to_string_pretty(&before).unwrap();
        let after: NodeConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(before, after);
        // The module type is hex on the wire and hex in the file, because a file
        // that does not look like the protocol is a file people mistranslate.
        assert!(json.contains("\"type\": \"0x0007\""), "{json}");
    }

    #[test]
    fn a_config_written_by_hand_may_leave_out_everything_optional() {
        let sparse: NodeConfig = serde_json::from_value(json!({
            "name": "Fog machine",
            "serial": "9f1c22",
            "module": { "type": "0x0100", "name": "Fogger" },
            "ports": [
                { "port": 0, "name": "Fog output", "access": "readwrite", "dataType": "number" },
            ],
        }))
        .unwrap();

        assert_eq!(sparse.module.type_id, 0x0100);
        assert_eq!(sparse.module.rev, "a");
        assert!(sparse.advertise, "a node written down to be run should be findable");
        assert_eq!(sparse.dmx, None);
        assert!(sparse.problems().is_empty());
    }

    #[test]
    fn a_config_says_what_is_wrong_with_it_rather_than_refusing_to_load() {
        let muddled: NodeConfig = serde_json::from_value(json!({
            "name": "Muddle", "serial": "",
            "module": { "type": 4, "name": "Whatever" },
            "ports": [
                { "port": 0, "name": "One", "access": "readwrite", "dataType": "boolean" },
                { "port": 0, "name": "Two", "access": "sideways", "dataType": "vibes" },
            ],
        }))
        .unwrap();

        // It parsed, because an editor has to be able to show a bad config in
        // order for anyone to fix it.
        assert_eq!(muddled.module.type_id, 4, "a plain number is a module type too");
        let problems = muddled.problems();
        assert!(problems.iter().any(|p| p.contains("serial")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("numbered 0")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("sideways")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("vibes")), "{problems:?}");
    }

    #[test]
    fn only_the_gateway_says_it_forwards_a_universe() {
        assert!(ModuleKind::DmxOut.dmx().is_some());
        assert!(ModuleKind::DmxOut.ports().is_empty());
        for module in ModuleKind::ALL.into_iter().filter(|m| *m != ModuleKind::DmxOut) {
            assert!(module.dmx().is_none(), "{} is not a gateway", module.key());
            assert!(!module.ports().is_empty(), "{} has terminals", module.key());
        }
    }

    #[test]
    fn a_reading_carries_the_unit_the_description_gave_it() {
        let temperature = ModuleKind::Environment.ports().remove(0);
        assert_eq!(temperature.unit.as_deref(), Some("degree-celsius"));
        assert_eq!(temperature.class.as_deref(), Some("temperature"));
    }

    #[test]
    fn a_node_that_has_been_stopped_gives_its_port_back() {
        // What a window reconfiguring a node depends on: the old socket has to be
        // gone by the time `stop` returns, or the next node cannot bind it.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut config = ModuleKind::MainsRelay.config("4d5e6f");
            config.http_port = 0;
            let first = start(SimConfig { node: config.clone(), sacn_port: 0 }).await.unwrap();
            let port = first.http_addr.port();
            config.http_port = port;

            first.stop.stop().await;

            let again = start(SimConfig { node: config, sacn_port: 0 }).await;
            assert!(again.is_ok(), "the port a stopped node held has to be free");
        });
    }

    #[test]
    fn a_config_with_no_ports_and_no_universe_is_a_node_that_describes_nothing() {
        // Which a console is entitled to refuse to adopt — and being able to make
        // one here is how that gets tested against a real node rather than a stub.
        let silent: NodeConfig = serde_json::from_value(json!({
            "name": "Old firmware", "serial": "0b501e",
            "module": { "type": "0x0004", "name": "Mains Relay" },
        }))
        .unwrap();

        assert!(silent.ports.is_empty());
        assert_eq!(silent.dmx, None);
        assert!(silent.problems().is_empty(), "saying nothing is not a malformed config");
    }

    #[test]
    fn a_datagram_that_is_not_e131_is_ignored() {
        assert_eq!(parse_e131(b"hello"), None);
        assert_eq!(parse_e131(&[0u8; 200]), None);
    }


    // ── Tracing a shape ───────────────────────────────────────────────────────
    //
    // The point of these is that nothing on the network says anything while they
    // run. A value that moves between two reads moved because this node moved it.

    /// A node handed a shape animates on its own, and `GET /api/v1/state` sees it,
    /// because the renderer writes through the same store a `set` writes to.
    #[tokio::test]
    async fn a_port_handed_a_shape_animates_without_being_told_again() {
        let sim = start(SimConfig::new(ModuleKind::Ws2812, "strip1")).await.unwrap();
        let node_url = format!("http://{}/api/v1/state", sim.http_addr);
        let client = reqwest::Client::new();

        // A one-second sine on the brightness port, anchored now.
        client
            .post(&node_url)
            .json(&json!({ "effects": { "1": {
                "id": "fx", "curve": { "shape": "sine" }, "rate": 1.0, "phase": 0.0,
                "direction": "forward", "width": 0.5,
                "low": { "value": 0.0 }, "high": { "value": 1.0 },
                "t0": now_ms(),
            }}}))
            .send()
            .await
            .unwrap();

        let read = || async {
            let body: Value = client.get(&node_url).send().await.unwrap().json().await.unwrap();
            body
        };

        // The renderer runs at 40 Hz; a sixth of a second is several ticks and a
        // sixth of a cycle, so the value cannot honestly be the same.
        let first = read().await;
        tokio::time::sleep(Duration::from_millis(160)).await;
        let second = read().await;

        assert!(
            first["outputs"]["1"] != second["outputs"]["1"],
            "the port should have moved on its own: {} then {}",
            first["outputs"]["1"],
            second["outputs"]["1"],
        );
        assert!(
            second["effects"]["1"]["summary"].as_str().is_some_and(|s| s.contains("sine")),
            "and it says what it is tracing: {}",
            second["effects"]["1"],
        );

        sim.stop.stop();
    }

    /// Clearing stops the shape and leaves the port where it was. The console
    /// follows a clear with a value for exactly that reason: this node has no
    /// opinion about where a stopped effect should leave things.
    #[tokio::test]
    async fn clearing_a_shape_stops_the_port_where_it_stands() {
        let sim = start(SimConfig::new(ModuleKind::Ws2812, "strip2")).await.unwrap();
        let node_url = format!("http://{}/api/v1/state", sim.http_addr);
        let client = reqwest::Client::new();

        client
            .post(&node_url)
            .json(&json!({ "effects": { "1": {
                "curve": { "shape": "saw-up" }, "rate": 1.0, "phase": 0.0,
                "low": { "value": 0.0 }, "high": { "value": 1.0 }, "t0": now_ms(),
            }}}))
            .send()
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;

        client
            .post(&node_url)
            .json(&json!({ "effects": { "1": { "clear": true } } }))
            .send()
            .await
            .unwrap();

        let stopped: Value = client.get(&node_url).send().await.unwrap().json().await.unwrap();
        tokio::time::sleep(Duration::from_millis(160)).await;
        let later: Value = client.get(&node_url).send().await.unwrap().json().await.unwrap();

        assert_eq!(stopped["outputs"]["1"], later["outputs"]["1"], "held where it stopped");
        assert!(later["effects"].get("1").is_none(), "and nothing is tracing");

        sim.stop.stop();
    }

    /// A three second fade arrives as one message and the node walks it. This is
    /// the other half of the bargain: a hundred and twenty messages become one.
    #[tokio::test]
    async fn a_timed_set_is_walked_rather_than_jumped_to() {
        let sim = start(SimConfig::new(ModuleKind::Ws2812, "strip3")).await.unwrap();
        let node_url = format!("http://{}/api/v1/state", sim.http_addr);
        let client = reqwest::Client::new();

        client.post(&node_url).json(&json!({ "outputs": { "1": { "value": 0.0 } } }))
            .send().await.unwrap();
        client
            .post(&node_url)
            .json(&json!({ "outputs": { "1": {
                "value": 1.0, "fade_ms": 400, "curve": "linear", "t0": now_ms(),
            }}}))
            .send()
            .await
            .unwrap();

        // Part way through: somewhere between the ends, and at neither of them.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let midway: Value = client.get(&node_url).send().await.unwrap().json().await.unwrap();
        let level = midway["outputs"]["1"]["value"].as_f64().unwrap();
        assert!(level > 0.05 && level < 0.95, "part way there, not jumped: {level}");

        // And it arrives, and stops being a transition once it has.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let arrived: Value = client.get(&node_url).send().await.unwrap().json().await.unwrap();
        assert_eq!(arrived["outputs"]["1"]["value"].as_f64().unwrap(), 1.0, "there");
        assert!(arrived["effects"].get("1").is_none(), "and done");

        sim.stop.stop();
    }

    /// A console that has decided to send a value has taken the port back. A shape
    /// still running underneath would overwrite it on the very next tick.
    #[tokio::test]
    async fn a_plain_set_cancels_whatever_the_port_was_tracing() {
        let sim = start(SimConfig::new(ModuleKind::Ws2812, "strip4")).await.unwrap();
        let node_url = format!("http://{}/api/v1/state", sim.http_addr);
        let client = reqwest::Client::new();

        client
            .post(&node_url)
            .json(&json!({ "effects": { "1": {
                "curve": { "shape": "sine" }, "rate": 2.0, "phase": 0.0,
                "low": { "value": 0.0 }, "high": { "value": 1.0 }, "t0": now_ms(),
            }}}))
            .send()
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;

        client.post(&node_url).json(&json!({ "outputs": { "1": { "value": 0.42 } } }))
            .send().await.unwrap();

        tokio::time::sleep(Duration::from_millis(160)).await;
        let held: Value = client.get(&node_url).send().await.unwrap().json().await.unwrap();
        assert_eq!(held["outputs"]["1"], json!({ "value": 0.42 }), "and it stays put");
        assert!(held["effects"].get("1").is_none(), "nothing tracing");

        sim.stop.stop();
    }

    /// The strip advertises what it can do, and a relay honestly advertises less.
    /// The console reads this and nothing else to decide what to send.
    #[tokio::test]
    async fn info_says_which_ports_can_trace_a_shape() {
        let sim = start(SimConfig::new(ModuleKind::Ws2812, "strip5")).await.unwrap();
        let info: Value = reqwest::get(format!("http://{}/api/v1/info", sim.http_addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let brightness = &info["ports"][1]["effects"];
        assert!(brightness["shapes"].as_array().unwrap().iter().any(|s| s == "sine"));
        assert_eq!(brightness["steps"], true);
        assert_eq!(brightness["transitions"], true);
        sim.stop.stop();

        let relay = start(SimConfig::new(ModuleKind::MainsRelay, "relay5")).await.unwrap();
        let info: Value = reqwest::get(format!("http://{}/api/v1/info", relay.http_addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let shapes = info["ports"][0]["effects"]["shapes"].as_array().unwrap().clone();
        assert_eq!(shapes, vec![json!("square")], "two states, so only chopping");
        assert_eq!(info["ports"][0]["effects"]["transitions"], false, "and nothing to cross");
        relay.stop.stop();
    }

    /// A port the node reads is never driven, so advertising on one is a promise
    /// nothing will ever ask it to keep. The editor should say so.
    #[test]
    fn advertising_effects_on_an_input_is_a_problem() {
        let mut config = ModuleKind::Environment.config("aaaaaa");
        config.ports[0].effects = Some(motion::PortEffects::all());
        let problems = config.problems();
        assert!(
            problems.iter().any(|p| p.contains("readonly") && p.contains("effects")),
            "{problems:?}",
        );
    }

    #[test]
    fn a_string_port_may_step_but_has_no_shape_to_trace() {
        let mut config = ModuleKind::Oled.config("bbbbbb");
        config.ports[0].effects = Some(motion::PortEffects { steps: true, ..Default::default() });
        assert!(config.problems().is_empty(), "a list of messages in turn is fine");

        config.ports[0].effects = Some(motion::PortEffects::all());
        assert!(
            config.problems().iter().any(|p| p.contains("string") && p.contains("shapes")),
            "but there is nothing between two strings for a sine to trace",
        );
    }
}
