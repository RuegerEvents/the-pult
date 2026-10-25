//! OpenHaunt I/O nodes: finding them, adopting them, and keeping track of them.
//!
//! Modeled on [`crate::infra::session::SessionManager`] — an actor owning a piece
//! of LOCAL state, pushing it to the engine whenever it changes. The difference is
//! that discovery arrives as [`DeviceEvent`] rather than being read from mDNS
//! inside the loop, so a test can drive the whole thing without multicast.
//!
//! A node is discovered, not configured. Every node on the network browses and
//! lists what it finds; only the one driving the session adopts or commands
//! anything, which is the same gate playback uses for follow cues.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::Utc;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use futures::StreamExt;
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::{Path, PathPattern, PathSegment},
    types::{
        devices::{DevicesState, DiscoveredDevice, NodeAudio},
        openhaunt::EffectCapability,
        fixture::{Fixture, FixtureAddress, FixtureType, ParameterBinding, ParameterDirection},
        openhaunt,
        output::{OutputConfig, OutputKind},
        pixels::{self, NodePixels, PixelStrip, StripMode},
    },
};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    engine::EngineHandle,
    infra::devices::mqtt::{MqttEvent, MqttLink},
    model::playback::parameter_key,
};

pub mod broker;
pub mod mqtt;

pub const SERVICE_TYPE: &str = "_openhaunt._tcp.local.";

/// How often an adopted node's strips are compared with the show.
const STRIP_CHECK: Duration = Duration::from_secs(5);

// ── Events in ─────────────────────────────────────────────────────────────────

/// Discovery, as the manager sees it. mDNS is one source; a test is another.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    Resolved {
        serial: String,
        ip: String,
        port: u16,
        host: String,
        txt: BTreeMap<String, String>,
    },
    Removed {
        serial: String,
    },
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "SetOutput gains its caller with the output plugin")]
pub enum DeviceCommand {
    /// Turn a discovered device into a patched fixture.
    Adopt { serial: String, reply: oneshot::Sender<Result<Uuid, String>> },
    /// Make the node blink, so the operator can tell which box it is.
    Identify { serial: String, reply: oneshot::Sender<Result<(), String>> },
    /// Unpatch a device's fixture. The device stays discovered.
    Forget { serial: String, reply: oneshot::Sender<Result<(), String>> },
    /// Patch one strip head in a mode, as the operator `user` asked: one gesture.
    PatchStrip {
        serial: String,
        head: u8,
        change: StripChange,
        user: Uuid,
        reply: oneshot::Sender<Result<Uuid, String>>,
    },
    /// Drive one output port on a node.
    SetOutput { serial: String, port: u8, value: serde_json::Value },
    /// Hand one output port a shape to trace, or `None` to take it back.
    SetEffect { serial: String, port: u8, payload: Option<serde_json::Value> },
    /// Tell a node's loudspeaker something: `play`, `stop`, `gain`, `stream`, under
    /// `openhaunt/<sn>/audio/`.
    Audio { serial: String, verb: &'static str, payload: serde_json::Value },
    Event(DeviceEvent),
    Stop,
}

/// What a `device.strip` call changes about a strip. Everything left out stays.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripChange {
    pub mode: Option<StripMode>,
    pub count: Option<u16>,
    pub order: Option<String>,
    pub universe: Option<u16>,
    pub address: Option<u16>,
    pub hold_ms: Option<u32>,
    pub pitch_m: Option<f32>,
}

#[derive(Clone)]
pub struct DeviceHandle(pub mpsc::Sender<DeviceCommand>);

#[allow(dead_code, reason = "set_output gains its caller with the output plugin")]
impl DeviceHandle {
    pub async fn adopt(&self, serial: String) -> Result<Uuid, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(DeviceCommand::Adopt { serial, reply: tx }).await;
        rx.await.unwrap_or_else(|_| Err("device manager is gone".into()))
    }

    pub async fn identify(&self, serial: String) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(DeviceCommand::Identify { serial, reply: tx }).await;
        rx.await.unwrap_or_else(|_| Err("device manager is gone".into()))
    }

    pub async fn forget(&self, serial: String) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(DeviceCommand::Forget { serial, reply: tx }).await;
        rx.await.unwrap_or_else(|_| Err("device manager is gone".into()))
    }

    /// Patch a strip head in a mode. Answers the head's fixture id.
    pub async fn patch_strip(
        &self,
        serial: String,
        head: u8,
        change: StripChange,
        user: Uuid,
    ) -> Result<Uuid, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .0
            .send(DeviceCommand::PatchStrip { serial, head, change, user, reply: tx })
            .await;
        rx.await.unwrap_or_else(|_| Err("device manager is gone".into()))
    }

    /// Drive an output port. Never blocks: a device that is not keeping up must not
    /// hold up whatever produced the value.
    pub fn set_output(&self, serial: String, port: u8, value: serde_json::Value) {
        let _ = self.0.try_send(DeviceCommand::SetOutput { serial, port, value });
    }

    /// Hand an output port a shape, or `None` to stop it tracing one. Same rule.
    pub fn set_effect(&self, serial: String, port: u8, payload: Option<serde_json::Value>) {
        let _ = self.0.try_send(DeviceCommand::SetEffect { serial, port, payload });
    }

    /// Tell a node's loudspeaker something. Never blocks, for the same reason, and
    /// callable from a thread that is not the runtime's: the stream sender is one.
    pub fn audio(&self, serial: String, verb: &'static str, payload: serde_json::Value) {
        if self.0.try_send(DeviceCommand::Audio { serial, verb, payload }).is_err() {
            debug!("[devices] the device manager is behind; an audio command was dropped");
        }
    }
}

// ── Directory ─────────────────────────────────────────────────────────────────

/// Where each device is right now, for the parts of the system that send to one.
///
/// Output plugins are handed only a patch, so a plugin that has to reach a device
/// over the network watches this instead.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceDirectory {
    pub entries: BTreeMap<String, DeviceEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceEntry {
    pub ip: String,
    pub port: u16,
    pub module_type: u16,
    /// Every universe the node reads: the one a gateway forwards, and every one a strip
    /// in `sacn` mode is patched to. The OpenHaunt connector unicasts each of them to it.
    pub universes: Vec<u16>,
    pub online: bool,
    /// Which of this node's ports can trace a shape for themselves.
    ///
    /// Here as well as on the discovered device because an output plugin is handed
    /// the directory and never the device list — it has to decide port by port
    /// whether to send a description or a stream of samples, and this is where it
    /// looks.
    pub effects: Option<EffectCapability>,
    /// What the node's loudspeaker can do, where it has one. Here for the reason
    /// `effects` is: the audio manager is handed the directory, not the device list.
    pub audio: Option<NodeAudio>,
    /// Connected to this station's broker, which is the only way a node hears an audio
    /// command. Online on mDNS and not yet on the broker is a node that would miss one.
    pub on_broker: bool,
    /// How many times this station has seen the node start over without going away:
    /// a reboot quick enough that mDNS never noticed (its uptime went backwards), or a
    /// strip re-patched (the node rebuilds the strip's ports at their defaults).
    ///
    /// A connector remembering what it last sent a node has to forget it when this
    /// moves, or the node sits at its defaults while the console believes it told it
    /// otherwise — a strip left black after its pixel count changed. Going offline and
    /// back is the other way of starting over, and `online` already says that one.
    pub starts: u64,
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct DeviceManager {
    node_id: NodeId,
    engine: EngineHandle,
    rx: mpsc::Receiver<DeviceCommand>,
    state: DevicesState,
    directory: watch::Sender<DeviceDirectory>,
    http: reqwest::Client,
    /// Where this node's own broker listens, when it is the one driving.
    broker_port: u16,
    /// This station's cables, for working out which of its addresses a node can
    /// reach its broker door at.
    net: crate::infra::net::NetHandle,
    /// The connection to the broker. Present only while driving.
    mqtt: Option<MqttLink>,
    mqtt_tx: mpsc::Sender<MqttEvent>,
    /// Devices that have announced themselves on the broker. A node reachable this
    /// way is driven over MQTT; one that has been adopted but has not connected yet
    /// still has to be reachable, and for that there is HTTP.
    on_broker: std::collections::BTreeSet<String>,
    /// [`DeviceEntry::starts`], per serial.
    starts: BTreeMap<String, u64>,
    /// Counts up with every clock publish, so a node can tell a fresh sample from a
    /// retained one replayed after the broker restarted.
    clock_seq: u64,
    /// The port nodes dial, open while this station drives them and closed the moment
    /// it stops. See [`broker`].
    door: Option<broker::FrontDoor>,
    /// The broker list this station last handed its nodes, leader first (doc 15).
    /// Empty while not driving.
    brokers: Vec<String>,
}

impl DeviceManager {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        broker_port: u16,
    ) -> (Self, DeviceHandle, watch::Receiver<DeviceDirectory>) {
        let (tx, rx) = mpsc::channel(64);
        let (directory, directory_rx) = watch::channel(DeviceDirectory::default());
        let http = reqwest::Client::builder()
            // A node that has fallen off the network must not stall adoption of the
            // one next to it.
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        // Replaced in `run` with the receiving half's real partner; a manager that is
        // never run simply drops what it would have sent.
        let (mqtt_tx, _) = mpsc::channel(1);
        (
            DeviceManager {
                node_id,
                engine,
                rx,
                state: DevicesState::default(),
                directory,
                http,
                broker_port,
                net: crate::infra::net::Network::unconfigured(),
                mqtt: None,
                mqtt_tx,
                on_broker: Default::default(),
                starts: BTreeMap::new(),
                clock_seq: 0,
                door: None,
                brokers: Vec::new(),
            },
            DeviceHandle(tx),
            directory_rx,
        )
    }

    /// The station's own network, so the broker it advertises is on a cable the
    /// `openhaunt` preference names. Without one, every interface this machine has.
    pub fn with_net(mut self, net: crate::infra::net::NetHandle) -> Self {
        self.net = net;
        self
    }

    pub async fn run(mut self) {
        let (mqtt_tx, mut mqtt_rx) = mpsc::channel(256);
        self.mqtt_tx = mqtt_tx;

        // Leadership changes arrive as writes to the LOCAL session state, which is
        // the same thing playback gates follow cues on.
        let mut sessions = self.engine.subscribe_pattern(PathPattern::new("session")).await;
        // Who else is in the session, and where their brokers are: what the broker list
        // a leader hands its nodes is made of.
        let mut stations = self.engine.subscribe_pattern(PathPattern::new("stations/**")).await;

        self.push_state().await;
        // A console restarted mid-show comes up already leading, with devices already
        // adopted. Nothing will announce that, so it is checked once here.
        self.reconsider_leadership().await;

        // The first periodic thing in this loop. A node tracing a shape has to know
        // what time the console thinks it is, or it cannot place the start of a cycle,
        // and a second is often enough: the estimate is smoothed and the error it is
        // correcting is one-way LAN latency, a few milliseconds.
        let mut clock = tokio::time::interval(Duration::from_secs(1));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // How often the strips are checked against the show. The show changes them
        // without telling anybody here — an undo, a peer, a showfile opened — so this is
        // the catch-all; a patch from this station reconciles at once.
        let mut strips = tokio::time::interval(STRIP_CHECK);
        strips.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        DeviceCommand::Stop => break,
                        DeviceCommand::Event(event) => self.handle_event(event).await,
                        DeviceCommand::Adopt { serial, reply } => {
                            let _ = reply.send(self.adopt(&serial).await);
                        }
                        DeviceCommand::Identify { serial, reply } => {
                            let _ = reply.send(self.identify(&serial).await);
                        }
                        DeviceCommand::Forget { serial, reply } => {
                            let _ = reply.send(self.forget(&serial).await);
                        }
                        DeviceCommand::PatchStrip { serial, head, change, user, reply } => {
                            let _ = reply.send(self.patch_strip(&serial, head, change, user).await);
                        }
                        DeviceCommand::SetOutput { serial, port, value } => {
                            self.set_output(&serial, port, value).await;
                        }
                        DeviceCommand::SetEffect { serial, port, payload } => {
                            self.set_effect(&serial, port, payload).await;
                        }
                        DeviceCommand::Audio { serial, verb, payload } => {
                            self.send_audio(&serial, verb, payload);
                        }
                    }
                }
                Some(event) = mqtt_rx.recv() => self.handle_mqtt(event).await,
                Some(_) = sessions.next() => self.reconsider_leadership().await,
                Some(_) = stations.next() => self.refresh_brokers().await,
                _ = clock.tick() => self.publish_clock().await,
                _ = strips.tick() => self.reconcile_all_strips().await,
            }
        }
        if let Some(mqtt) = self.mqtt.take() {
            mqtt.stop();
        }
        info!("[devices] stopped");
    }

    // ── Leadership ────────────────────────────────────────────────────────────

    /// Start or stop driving devices, following the session.
    ///
    /// Browsing never stops: seeing what is on the network costs nothing and is
    /// worth showing on every node. What stops is the broker connection and
    /// everything that commands a device through it.
    ///
    /// A leader with nothing adopted starts no broker either. A console that has
    /// never been near an OpenHaunt node should not be listening on 1883 because it
    /// was started — the same reason Art-Net is off unless asked for.
    async fn reconsider_leadership(&mut self) {
        let driving = !self.is_follower().await && self.has_adopted_a_device().await;
        if driving != self.mqtt.is_some() {
            if driving {
                self.start_driving().await;
            } else {
                info!("[devices] not driving devices");
                if let Some(mqtt) = self.mqtt.take() {
                    mqtt.stop();
                }
                // The broker thread stays up — it cannot be stopped — but its door
                // closes, so every node on it drops and walks its list to whoever
                // leads now. Holding a node this station no longer commands would
                // leave its inputs with nobody to act on them.
                self.door = None;
                self.brokers.clear();
                self.state.broker_addr = None;
            }
        }
        // Always: joining a session changes what this node says about itself even
        // when it had nothing to drive either side of the change.
        self.publish().await;
    }

    /// Is anything in this show patched to a node?
    ///
    /// Read from the fixtures rather than the discovered list, because a console
    /// restarted mid-show has the fixtures long before mDNS finds the devices again.
    async fn has_adopted_a_device(&self) -> bool {
        self.fixtures().await.iter().any(|f| f.address.serial().is_some())
    }

    async fn start_driving(&mut self) {
        let inside = broker::ensure(self.broker_port);
        match broker::FrontDoor::open(self.broker_port, inside).await {
            Ok(door) => self.door = Some(door),
            Err(e) => warn!(
                "[devices] could not open port {} to the nodes: {e}; none of them can reach this station",
                self.broker_port
            ),
        }
        let advertised = self.own_door().unwrap_or_else(|| {
            broker::advertised_addr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), self.broker_port)
        });
        self.state.broker_addr = Some(advertised.clone());

        // The console reaches its own broker over loopback, past the door.
        // The client id carries the node id: a broker disconnects the older of two
        // clients claiming the same one, so two consoles sharing a broker would
        // otherwise take turns kicking each other off.
        let client_id = format!("the-pult-{}", &self.node_id.0.to_string()[..8]);
        self.mqtt = Some(MqttLink::connect(&inside.to_string(), &client_id, self.mqtt_tx.clone()));
        info!("[devices] driving devices, broker advertised as {advertised}");

        self.ensure_openhaunt_output().await;

        // A device adopted by a previous leader is pointing at a broker that is no
        // longer listening. One it walked here by its list hears the new list on the
        // retained topic the moment it subscribes; one mDNS shows is told over HTTP as
        // well, which is also what reaches a node that has never had a list.
        self.brokers.clear();
        self.refresh_brokers().await;
    }

    /// Where the nodes this station has seen are, for choosing which door to name.
    fn node_addresses(&self) -> Vec<std::net::Ipv4Addr> {
        self.state.discovered.values().filter_map(|d| d.ip.parse().ok()).collect()
    }

    /// The door this station names itself by to its nodes: whichever of its own is
    /// nearest the nodes it has seen ([`broker::nearest_door`]).
    fn own_door(&self) -> Option<String> {
        broker::nearest_door(&broker::doors(&self.net, self.broker_port), &self.node_addresses())
    }

    /// Hand the nodes the broker list, if it has changed: this station first, then
    /// every other station of the session that says where its broker is (doc 15).
    ///
    /// Retained on `openhaunt/brokers`, so every node on this broker has it, including
    /// one that walked here by its list and that no browse on this station has ever
    /// seen; and over HTTP to every adopted node mDNS shows, so a node not yet on any
    /// broker learns it too.
    async fn refresh_brokers(&mut self) {
        if self.mqtt.is_none() {
            return;
        }
        // Re-chosen every time: a node discovered on the lighting network since the
        // last list is what says which of this station's cables they are on.
        if let Some(own) = self.own_door() {
            if self.state.broker_addr.as_ref() != Some(&own) {
                self.state.broker_addr = Some(own);
                self.push_state().await;
            }
        }
        let Some(own) = self.state.broker_addr.clone() else { return };
        let stations: Vec<pult_schema::types::station::Station> = self.read("stations").await;
        let list = broker_list(&own, self.node_id, &stations, &self.node_addresses());
        if list == self.brokers {
            return;
        }
        info!("[devices] broker list: {}", list.join(", "));
        self.brokers = list;
        if let Some(mqtt) = &self.mqtt {
            mqtt.publish(mqtt::BROKERS_TOPIC.to_string(), mqtt::brokers_payload(&self.brokers), true);
        }
        for serial in self.adopted_and_online() {
            self.push_config(&serial).await;
        }
        // And to every adopted node mDNS does not show right now, at the address it
        // was last seen at, which doc 15 asks of a controller ("every node it can
        // reach"). The case is this station's own address moving: the node goes on
        // dialing the old one, its broker never answers, and a browse that has not
        // found it again is no reason to leave it there. Off the actor, because a node
        // that really has gone costs the whole HTTP timeout to find out about.
        for serial in self.adopted_and_offline() {
            let (Some(config), Some(base)) = (self.config_for(&serial).await, self.base_url(&serial)) else {
                continue;
            };
            let http = self.http.clone();
            tokio::spawn(async move {
                match http.post(format!("{base}/config")).json(&config).send().await {
                    Ok(r) if r.status().is_success() => info!("[devices] {serial} told the broker list while out of sight"),
                    Ok(r) => debug!("[devices] {serial} refused the broker list: {}", r.status()),
                    Err(e) => debug!("[devices] {serial} is not at its last address: {e}"),
                }
            });
        }
    }

    /// Make sure the show has an output that reaches the nodes.
    ///
    /// Outputs are show data, and the plugin that drives adopted nodes' ports only
    /// runs where an `outputs` row of kind OpenHaunt says so. Adopting a node is
    /// the operator saying they want it driven, so that row is created here rather
    /// than left for them to discover in the Outputs panel after nothing moved.
    /// Idempotent: an existing OpenHaunt output that runs on this station — its
    /// own or an every-station one — is left alone, enabled or not, because
    /// switching an output off is a decision this must not undo.
    async fn ensure_openhaunt_output(&self) {
        let outputs: Vec<OutputConfig> = self.read("outputs").await;
        let covered = outputs.iter().any(|o| {
            o.kind == OutputKind::OpenHaunt
                && o.node_id.map(|owner| owner == self.node_id).unwrap_or(true)
        });
        if covered {
            return;
        }
        let output = OutputConfig {
            id: Uuid::new_v4(),
            name: "OpenHaunt nodes".to_string(),
            kind: OutputKind::OpenHaunt,
            target: None,
            universes: Vec::new(),
            enabled: true,
            node_id: Some(self.node_id),
            interfaces: Default::default(),
            priority: Default::default(),
        };
        match serde_json::to_value(&output) {
            Ok(value) => {
                if let Err(e) =
                    self.engine.set(create_path("outputs"), Lifecycle::Persisted, value).await
                {
                    warn!("[devices] could not create the OpenHaunt output: {e}");
                } else {
                    info!("[devices] added an OpenHaunt output for this station");
                }
            }
            Err(e) => warn!("[devices] could not create the OpenHaunt output: {e}"),
        }
    }

    fn adopted_and_offline(&self) -> Vec<String> {
        self.state
            .discovered
            .values()
            .filter(|d| !d.online && d.adopted_fixture_id.is_some())
            .map(|d| d.serial.clone())
            .collect()
    }

    fn adopted_and_online(&self) -> Vec<String> {
        self.state
            .discovered
            .values()
            .filter(|d| d.online && d.adopted_fixture_id.is_some())
            .map(|d| d.serial.clone())
            .collect()
    }

    // ── Input ─────────────────────────────────────────────────────────────────

    async fn handle_mqtt(&mut self, event: MqttEvent) {
        match event {
            MqttEvent::Input { serial, port, value, .. } => {
                // Not gated on mDNS: a node that walked here by its broker list after
                // a failover may be one no browse on this station has seen, and what it
                // reports is still the show's. Being adopted is the gate, and that is
                // what the port lookup below answers.
                let Some((fixture_id, key)) = self.parameter_on_port(&serial, port).await else {
                    debug!("[devices] {serial} port {port} is not bound to a parameter");
                    return;
                };
                let value = serde_json::to_value(&value).unwrap_or_default();
                if let Err(e) = self.engine.set_sensed_value(fixture_id, key, value).await {
                    warn!("[devices] {serial} port {port}: {e}");
                }
            }
            MqttEvent::Status { serial, online } => {
                let moved = if online {
                    self.on_broker.insert(serial.clone())
                } else {
                    self.on_broker.remove(&serial)
                };
                let Some(device) = self.state.discovered.get_mut(&serial) else { return };
                if device.online == online && !moved {
                    return;
                }
                device.online = online;
                self.publish().await;
            }
            MqttEvent::Health { serial, health } => {
                let Some(device) = self.state.discovered.get_mut(&serial) else { return };
                // Uptime going backwards is a reboot, however quickly it came back.
                if device.health.as_ref().is_some_and(|before| health.uptime_s < before.uptime_s) {
                    info!("[devices] {serial} restarted (uptime {} s)", health.uptime_s);
                    *self.starts.entry(serial.clone()).or_default() += 1;
                }
                device.health = Some(health);
                self.publish().await;
            }
            MqttEvent::AudioStatus { serial, status } => {
                let Some(device) = self.state.discovered.get_mut(&serial) else { return };
                device.audio_status = Some(status);
                // The LOCAL state only: the directory carries what a node *can* do, and
                // a status once a second would wake everything watching it for nothing.
                self.push_state().await;
            }
        }
    }

    /// The fixture and parameter key for whatever is bound to a port on a node.
    ///
    /// A node numbers its own terminals, so the port is the only thing both ends
    /// agree on; the parameter's kind is this console's business alone. Ports are
    /// numbered per node and not per head, so whichever of its fixtures binds the port
    /// is the one it reports into.
    async fn parameter_on_port(&self, serial: &str, port: u8) -> Option<(Uuid, String)> {
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        self.fixtures().await.into_iter().filter(|f| f.address.serial() == Some(serial)).find_map(
            |fixture| {
                let fixture_type = types.iter().find(|t| t.id == fixture.fixture_type_id)?;
                fixture_type
                    .parameters
                    .iter()
                    .find(|p| {
                        p.direction == ParameterDirection::Input
                            && p.binding == Some(ParameterBinding::Port { index: port })
                    })
                    .map(|p| (fixture.id, parameter_key(&p.kind)))
            },
        )
    }

    // ── Discovery ─────────────────────────────────────────────────────────────

    async fn handle_event(&mut self, event: DeviceEvent) {
        match event {
            DeviceEvent::Resolved { serial, ip, port, host, txt } => {
                let module_type = txt
                    .get("mod")
                    .and_then(|v| parse_module_type(v))
                    .unwrap_or_default();

                let get = |key: &str| txt.get(key).cloned().unwrap_or_default();
                let device = DiscoveredDevice {
                    name: txt.get("name").cloned().unwrap_or_else(|| serial.clone()),
                    serial: serial.clone(),
                    host,
                    ip: ip.clone(),
                    port,
                    fw: get("fw"),
                    protocol_version: get("v"),
                    module_type,
                    module_name: txt
                        .get("modname")
                        .cloned()
                        .unwrap_or_else(|| format!("Module {module_type:#06x}")),
                    module_serial: get("modsn"),
                    module_rev: get("modrev"),
                    caps: txt
                        .get("caps")
                        .map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                        .unwrap_or_default(),
                    // What the last `/info` said, until the one below answers.
                    description: self
                        .state
                        .discovered
                        .get(&serial)
                        .and_then(|d| d.description.clone()),
                    effects: self
                        .state
                        .discovered
                        .get(&serial)
                        .and_then(|d| d.effects.clone()),
                    audio: self.state.discovered.get(&serial).and_then(|d| d.audio.clone()),
                    audio_status: self
                        .state
                        .discovered
                        .get(&serial)
                        .and_then(|d| d.audio_status.clone()),
                    pixels: self.state.discovered.get(&serial).and_then(|d| d.pixels.clone()),
                    head_fixtures: self
                        .state
                        .discovered
                        .get(&serial)
                        .map(|d| d.head_fixtures.clone())
                        .unwrap_or_default(),
                    faults: self
                        .state
                        .discovered
                        .get(&serial)
                        .map(|d| d.faults.clone())
                        .unwrap_or_default(),
                    // The TXT record is enough to warn; `/api/v1/info` confirms below.
                    is_mains: txt.get("mains").map(|v| v == "1").unwrap_or(false),
                    online: true,
                    // A device that goes away and comes back keeps its adoption.
                    adopted_fixture_id: self
                        .state
                        .discovered
                        .get(&serial)
                        .and_then(|d| d.adopted_fixture_id),
                    health: self.state.discovered.get(&serial).and_then(|d| d.health.clone()),
                    last_seen: Utc::now(),
                };

                info!("[devices] {} ({}) at {ip}:{port}", device.name, device.module_name);
                self.state.discovered.insert(serial.clone(), device);

                // Every resolve asks again, which is what makes a changed `desc` in the
                // TXT record a refetch: the node re-announces when its description moves,
                // and mDNS hands that over as another resolve.
                self.refresh_description(&serial).await;
                self.reconcile_adoptions().await;
                self.retype(&serial).await;

                // A node that has rebooted has forgotten where to publish, and it
                // announcing itself again is the only notice we get. So does a node
                // this console adopted before its own restart. Re-configuring one
                // that never left is harmless: the same values.
                let adopted = self
                    .state
                    .discovered
                    .get(&serial)
                    .is_some_and(|d| d.adopted_fixture_id.is_some());
                if adopted {
                    self.reconsider_leadership().await;
                    self.push_config(&serial).await;
                }

                self.publish().await;
            }

            DeviceEvent::Removed { serial } => {
                let Some(device) = self.state.discovered.get_mut(&serial) else { return };
                device.online = false;
                debug!("[devices] {serial} went quiet");
                // An adopted device stays in the list while offline: its fixture is
                // still patched, and that is exactly when the operator needs to see it.
                if device.adopted_fixture_id.is_none() {
                    self.state.discovered.remove(&serial);
                }
                self.publish().await;
            }
        }
    }

    /// Ask the node what it is: whether it switches mains, and what its ports are.
    ///
    /// One round trip for all three, because they arrive in the same body. The mains
    /// flag in the descriptor is the authority; the TXT guess above is only there
    /// so the panel can warn without waiting. None means the node did not answer,
    /// in which case the guess stands and it stays undescribed.
    ///
    /// Effect capability is read from the raw body rather than out of the parsed
    /// description, and that is not an accident: `fixture_type_id` hashes the
    /// serialized `NodeDescription`, so a port that started advertising `effects`
    /// through that struct would give every node of its kind a fresh fixture type
    /// and orphan every parameter already patched against the old one. Serde drops
    /// the unknown key, the id stays put, and the capability is picked up alongside.
    async fn ask_the_node_what_it_is(&self, serial: &str) -> Option<WhatItIs> {
        let info: serde_json::Value = self.get_json(serial, "info").await?;
        let mains = info["module"]["flags"]
            .as_u64()
            .is_some_and(|flags| flags as u32 & openhaunt::MODULE_FLAG_MAINS != 0);
        let effects = openhaunt::effect_capability_from(&info);
        // Out of the raw body for the reason `effects` is, below.
        let audio = NodeAudio::from_info(&info);
        let pixels = NodePixels::from_info(&info);
        // The whole body, not just the two keys: `ports` and `dmx` both default,
        // so firmware that predates self-description parses as having neither.
        let description = serde_json::from_value(info).unwrap_or_default();
        Some(WhatItIs { mains, description, effects, audio, pixels })
    }

    /// Ask the node what it is, and keep the answer.
    async fn refresh_description(&mut self, serial: &str) {
        let Some(what) = self.ask_the_node_what_it_is(serial).await else { return };
        if let Some(device) = self.state.discovered.get_mut(serial) {
            device.is_mains = what.mains;
            device.description = Some(what.description);
            device.effects = what.effects;
            device.audio = what.audio;
            device.pixels = what.pixels;
        }
    }

    /// Work out which devices are adopted, from the fixtures rather than from memory.
    ///
    /// The fixture is the only persisted thing an adopted device has, so it is the
    /// answer in both directions: a fixture the operator deleted un-adopts its
    /// device, and a console restarted mid-show recognizes every device it had
    /// adopted before, which is what tells it to configure them again.
    async fn reconcile_adoptions(&mut self) {
        let fixtures = self.fixtures().await;
        for device in self.state.discovered.values_mut() {
            let on_node = || {
                fixtures.iter().filter(|f| f.address.serial() == Some(device.serial.as_str()))
            };
            device.adopted_fixture_id =
                on_node().find(|f| f.address.head() == Some(0)).map(|f| f.id);
            device.head_fixtures = on_node()
                .filter_map(|f| f.address.head().filter(|h| *h != 0).map(|h| (h, f.id)))
                .collect();
        }
    }

    /// Bring every fixture on a node onto the type its description says now.
    ///
    /// What [`FixtureTypeSource::Node`] has always promised: a node's own type is
    /// rebuilt whenever the node describes itself again. Changing a strip's mode
    /// changes its ports, so this is what makes a mode change reach the patch. A type
    /// that came from anywhere else is left alone, and so is the type a fixture leaves
    /// behind: a cue may still name it, and an unreferenced type costs a row.
    ///
    /// [`FixtureTypeSource::Node`]: pult_schema::types::fixture::FixtureTypeSource::Node
    async fn retype(&mut self, serial: &str) {
        if self.is_follower().await {
            return;
        }
        let Some(device) = self.state.discovered.get(serial).cloned() else { return };
        let Some(description) = device.description.as_ref().filter(|d| !d.is_empty()) else {
            return;
        };
        let effects = device.pixels.as_ref().map(|p| p.effects.clone()).unwrap_or_default();
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        for fixture in self.fixtures().await {
            if fixture.address.serial() != Some(serial) {
                continue;
            }
            let Some(head) = fixture.address.head() else { continue };
            let current = types.iter().find(|t| t.id == fixture.fixture_type_id);
            if current.is_some_and(|t| !t.source.is_derived_from_a_node()) {
                continue;
            }
            // A strip in `sacn` mode is drawn by the console, so its type is the strip's
            // own — a color per pixel — and not the nine ports the node offers for it.
            let wanted = match fixture.address.strip().filter(|s| s.mode == StripMode::Sacn) {
                Some(strip) => pixels::strip_type(strip),
                None => openhaunt::head_type_from(
                    device.module_type,
                    &device.module_name,
                    description,
                    head,
                    &effects,
                ),
            };
            let stored = types.iter().find(|t| t.id == wanted.id);
            let same = stored.and_then(|t| serde_json::to_value(t).ok())
                == serde_json::to_value(&wanted).ok();
            if !same {
                let path = match stored {
                    Some(_) => replace_path("fixture_types", wanted.id),
                    None => create_path("fixture_types"),
                };
                if let Err(e) = self.write(path, &wanted).await {
                    warn!("[devices] {serial}: could not rebuild {}: {e}", wanted.name);
                    continue;
                }
            }
            if fixture.fixture_type_id != wanted.id {
                info!("[devices] {serial} head {head} is now {}", wanted.name);
                let mut moved = fixture.clone();
                moved.fixture_type_id = wanted.id;
                if let Err(e) = self.write(replace_path("fixtures", moved.id), &moved).await {
                    warn!("[devices] {serial}: could not re-type head {head}: {e}");
                }
            }
        }
    }

    // ── Adoption ──────────────────────────────────────────────────────────────

    async fn adopt(&mut self, serial: &str) -> Result<Uuid, String> {
        if self.is_follower().await {
            return Err("only the node leading the session adopts devices".into());
        }
        let device = self
            .state
            .discovered
            .get(serial)
            .cloned()
            .ok_or_else(|| format!("no device with serial {serial}"))?;

        if let Some(id) = device.adopted_fixture_id {
            return Ok(id); // already adopted; adopting twice is not an error
        }

        // Only the device knows what it is. Without its description there is
        // nothing to patch, and no table here to guess from.
        let description = device
            .description
            .as_ref()
            .filter(|d| !d.is_empty())
            .ok_or_else(|| format!("{serial} does not describe its ports; cannot adopt"))?;

        // One fixture per head: the node's own ports on head 0, and each strip (or
        // whatever else the node calls a head) a fixture of its own, so two strips do
        // not collide on one color.
        let effects = device.pixels.as_ref().map(|p| p.effects.clone()).unwrap_or_default();
        // A node carries a universe exactly when it says it forwards one.
        let universe = match description.dmx {
            Some(_) => Some(self.next_free_universe().await),
            None => None,
        };
        let mut fixture = None;
        for head in description.head_numbers() {
            let node_type = openhaunt::head_type_from(
                device.module_type,
                &device.module_name,
                description,
                head,
                &effects,
            );
            let strip = device
                .pixels
                .as_ref()
                .and_then(|p| p.strip_for_head(head))
                .map(PixelStrip::from_node);
            // A strip the node is already running in `sacn` mode is a strip of cells.
            let fixture_type = match strip.as_ref().filter(|s| s.mode == StripMode::Sacn) {
                Some(strip) => pixels::strip_type(strip),
                None => node_type.clone(),
            };
            self.ensure_fixture_type(&fixture_type).await?;
            let name = match head {
                0 => device.name.clone(),
                _ => format!("{} {}", device.name, node_type.name),
            };
            let this = Fixture {
                // Head 0 is the node's own fixture and was always a fresh id; every other
                // head is derived, so a second console adopting the node makes the same row.
                id: if head == 0 { Uuid::new_v4() } else { openhaunt::head_fixture_id(serial, head) },
                name,
                fixture_type_id: fixture_type.id,
                address: FixtureAddress::OpenHaunt {
                    serial: serial.to_string(),
                    universe: if head == 0 { universe } else { None },
                    head: (head != 0).then_some(head),
                    strip: strip.clone(),
                },
                ..Fixture::default()
            };
            self.write(create_path("fixtures"), &this).await?;
            if head == 0 {
                fixture = Some(this);
            }
        }
        let fixture = fixture.ok_or_else(|| format!("{serial} has no head 0"))?;

        self.reconcile_adoptions().await;
        info!("[devices] adopted {serial} as fixture {}", fixture.id);
        self.reconcile_strips(serial).await;

        // The first adoption is what brings the broker up. Until now nothing has
        // asked this node for anything, which is the point of the protocol.
        //
        // Starting to drive configures every adopted device, this one included, so
        // it is only the later adoptions that need telling separately.
        let was_driving = self.mqtt.is_some();
        self.reconsider_leadership().await;
        if was_driving {
            self.push_config(serial).await;
        }

        self.publish().await;
        Ok(fixture.id)
    }

    async fn forget(&mut self, serial: &str) -> Result<(), String> {
        if self.is_follower().await {
            return Err("only the node leading the session adopts devices".into());
        }
        let Some(device) = self.state.discovered.get_mut(serial) else {
            return Err(format!("no device with serial {serial}"));
        };
        device.adopted_fixture_id = None;
        device.head_fixtures.clear();
        device.faults.clear();
        let online = device.online;

        // Every head. A strip is one of them, cells and all.
        let fixtures = self.fixtures().await;
        for head in fixtures.iter().filter(|f| f.address.serial() == Some(serial)) {
            self.delete("fixtures", head.id).await?;
        }
        // Nothing left to remember about a device that is not here and not patched.
        if !online {
            self.state.discovered.remove(serial);
        }
        // Unpatching the last node puts the broker connection away again.
        self.reconsider_leadership().await;
        self.publish().await;
        Ok(())
    }

    async fn identify(&self, serial: &str) -> Result<(), String> {
        self.post(serial, "identify", serde_json::json!({})).await
    }

    /// Drive one output port.
    ///
    /// MQTT for a node that has announced itself on the broker: it is already
    /// holding that socket open, and a relay following a button should not wait for
    /// a TCP handshake. HTTP for one that has been adopted but has not connected
    /// yet, which is otherwise unreachable until it does.
    async fn set_output(&self, serial: &str, port: u8, value: serde_json::Value) {
        match &self.mqtt {
            Some(mqtt) if self.on_broker.contains(serial) => {
                let payload = serde_json::to_vec(&value).unwrap_or_default();
                mqtt.publish(mqtt::output_topic(serial, port), payload, false);
            }
            _ => {
                let body = serde_json::json!({ "outputs": { port.to_string(): value } });
                if let Err(e) = self.post(serial, "state", body).await {
                    debug!("[devices] {serial} output {port}: {e}");
                }
            }
        }
    }

    /// Hand one output port a shape to trace, or take it back with `None`.
    ///
    /// The same two routes as a value, for the same reasons. What is different is how
    /// rarely this is called: a shape is described once and then the node is left to
    /// it, so where an output port would have seen forty messages a second it now sees
    /// one and then silence.
    async fn set_effect(&self, serial: &str, port: u8, payload: Option<serde_json::Value>) {
        let descriptor = payload.unwrap_or_else(|| serde_json::json!({ "clear": true }));
        match &self.mqtt {
            Some(mqtt) if self.on_broker.contains(serial) => {
                let body = serde_json::to_vec(&descriptor).unwrap_or_default();
                mqtt.publish(mqtt::effect_topic(serial, port), body, false);
            }
            _ => {
                let body = serde_json::json!({ "effects": { port.to_string(): descriptor } });
                if let Err(e) = self.post(serial, "state", body).await {
                    debug!("[devices] {serial} effect {port}: {e}");
                }
            }
        }
    }

    /// Tell a node's loudspeaker something.
    ///
    /// MQTT only: the audio extension has no HTTP form of `play`, and a node that has
    /// not reached the broker yet has not got the console's clock either, so a time
    /// sent to it would be a time it cannot place.
    fn send_audio(&self, serial: &str, verb: &str, payload: serde_json::Value) {
        match &self.mqtt {
            Some(mqtt) if self.on_broker.contains(serial) => {
                let body = serde_json::to_vec(&payload).unwrap_or_default();
                mqtt.publish(mqtt::audio_topic(serial, verb), body, false);
            }
            _ => debug!("[devices] {serial} is not on the broker; audio/{verb} not sent"),
        }
    }

    /// Say what time it is, for the nodes tracing a shape against it.
    ///
    /// Only while this station is actually driving devices. A follower publishing its
    /// own idea of the time onto the leader's broker would give every node two
    /// answers, and the whole point of the topic is that there is one.
    async fn publish_clock(&mut self) {
        let Some(mqtt) = &self.mqtt else { return };
        if !self.state.active {
            return;
        }
        self.clock_seq += 1;
        let now = pult_schema::types::sequence::now_ms();
        mqtt.publish(
            mqtt::CLOCK_TOPIC.to_string(),
            mqtt::clock_payload(now, self.clock_seq),
            // Retained, so a node that connects between ticks can place a cycle at
            // once rather than rendering against a guess for up to a second.
            true,
        );
    }

    /// Tell a node where to publish, and what to do with the universe it gateways.
    async fn push_config(&self, serial: &str) {
        let Some(config) = self.config_for(serial).await else { return };
        if let Err(e) = self.post(serial, "config", config).await {
            debug!("[devices] could not configure {serial}: {e}");
        }
    }

    /// What [`Self::push_config`] sends, or nothing before this station has a door.
    async fn config_for(&self, serial: &str) -> Option<serde_json::Value> {
        let broker = self.state.broker_addr.as_ref()?;
        // `broker` for a node from before lists, which reads nothing else; `brokers`
        // for one that can fail over on its own (doc 15). The first is always ours.
        let brokers = if self.brokers.is_empty() { vec![broker.clone()] } else { self.brokers.clone() };
        let mut config = serde_json::json!({ "mqtt": { "broker": broker, "brokers": brokers } });

        if let Some(universe) = self.gateway_universe(serial).await {
            config["dmx"] = serde_json::json!({ "protocol": "sacn", "universe": universe });
        }
        Some(config)
    }

    /// The universe an adopted gateway forwards, if this device is one.
    async fn gateway_universe(&self, serial: &str) -> Option<u16> {
        self.fixtures()
            .await
            .into_iter()
            .filter(|f| f.address.serial() == Some(serial))
            .find_map(|f| f.address.gateway_universe())
    }

    /// Create the fixture type if the show does not already have it.
    ///
    /// The id is derived from what the node said about itself, so adopting a second
    /// relay of the same firmware finds the first one's type rather than making
    /// another.
    async fn ensure_fixture_type(&self, fixture_type: &FixtureType) -> Result<(), String> {
        let existing: Vec<FixtureType> = self.read("fixture_types").await;
        if existing.iter().any(|t| t.id == fixture_type.id) {
            return Ok(());
        }
        let value = serde_json::to_value(fixture_type).map_err(|e| e.to_string())?;
        self.engine
            .set(create_path("fixture_types"), Lifecycle::Persisted, value)
            .await
            .map_err(|e| e.to_string())
    }

    /// The lowest universe number no DMX fixture and no other gateway is using.
    async fn next_free_universe(&self) -> u16 {
        let taken: std::collections::BTreeSet<u16> = self
            .fixtures()
            .await
            .iter()
            .flat_map(|f| match &f.address {
                // Every break, not just the first: a gateway handed a universe a
                // fixture's second break is already in would collide silently.
                FixtureAddress::Dmx { breaks, .. } => {
                    breaks.iter().map(|entry| entry.universe).collect::<Vec<_>>()
                }
                FixtureAddress::OpenHaunt { universe, strip, .. } => universe
                    .iter()
                    .copied()
                    .chain(strip.iter().flat_map(|s| s.universes()))
                    .collect(),
            })
            .collect();
        (1..=u16::MAX).find(|n| !taken.contains(n)).unwrap_or(1)
    }

    /// The lowest universe from which `count` universes in a row are free: where a strip
    /// switched to `sacn` mode goes unless somebody said where.
    async fn next_free_block(&self, count: u16, except: Uuid) -> u16 {
        let fixtures: Vec<Fixture> = self.fixtures().await;
        // The strip's own universes are not in its way.
        let taken: std::collections::BTreeSet<u16> = fixtures
            .iter()
            .filter(|f| f.id != except)
            .flat_map(|f| match &f.address {
                FixtureAddress::Dmx { breaks, .. } => {
                    breaks.iter().map(|entry| entry.universe).collect::<Vec<_>>()
                }
                FixtureAddress::OpenHaunt { universe, strip, .. } => universe
                    .iter()
                    .copied()
                    .chain(strip.iter().flat_map(|s| s.universes()))
                    .collect(),
            })
            .collect();
        let count = count.max(1);
        (1..=u16::MAX - count)
            .find(|start| (*start..*start + count).all(|n| !taken.contains(&n)))
            .unwrap_or(1)
    }

    // ── Talking to the engine ─────────────────────────────────────────────────

    async fn is_follower(&self) -> bool {
        self.engine
            .get(vec![PathSegment::Key("session".into())])
            .await
            .ok()
            .and_then(|v| v["is_follower"].as_bool())
            .unwrap_or(false)
    }

    async fn fixtures(&self) -> Vec<Fixture> {
        self.read("fixtures").await
    }

    async fn read<T: serde::de::DeserializeOwned>(&self, table: &str) -> Vec<T> {
        self.engine
            .get(vec![PathSegment::Key(table.into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Publish everything derived from the device list: the LOCAL state the
    /// frontends read, and the directory the output side sends through.
    async fn publish(&mut self) {
        // "Would drive", not "is driving": a leader with nothing adopted yet is
        // still the node an operator's Adopt button has to reach.
        self.state.active = !self.is_follower().await;

        // The universe lives on the fixture, because the fixture is the only
        // persisted thing an adopted device has.
        let mut universes: BTreeMap<String, Vec<u16>> = BTreeMap::new();
        for fixture in self.fixtures().await {
            if let FixtureAddress::OpenHaunt { serial, universe, strip, .. } = fixture.address {
                let list = universes.entry(serial).or_default();
                list.extend(universe);
                list.extend(strip.iter().flat_map(|s| s.universes()));
            }
        }

        let _ = self.directory.send(DeviceDirectory {
            entries: self
                .state
                .discovered
                .values()
                .map(|d| {
                    (
                        d.serial.clone(),
                        DeviceEntry {
                            ip: d.ip.clone(),
                            port: d.port,
                            module_type: d.module_type,
                            universes: universes.get(&d.serial).cloned().unwrap_or_default(),
                            online: d.online,
                            effects: d.effects.clone(),
                            audio: d.audio.clone(),
                            on_broker: self.on_broker.contains(&d.serial),
                            starts: self.starts.get(&d.serial).copied().unwrap_or(0),
                        },
                    )
                })
                .collect(),
        });
        self.push_state().await;
    }

    async fn push_state(&self) {
        if let Ok(json) = serde_json::to_value(&self.state) {
            let path = vec![PathSegment::Key("devices".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }

    // ── Talking to a node ─────────────────────────────────────────────────────

    fn base_url(&self, serial: &str) -> Option<String> {
        let device = self.state.discovered.get(serial)?;
        Some(format!("http://{}:{}/api/v1", device.ip, device.port))
    }

    async fn get_json(&self, serial: &str, endpoint: &str) -> Option<serde_json::Value> {
        let url = format!("{}/{endpoint}", self.base_url(serial)?);
        match self.http.get(&url).send().await {
            Ok(response) => response.json().await.ok(),
            Err(e) => {
                debug!("[devices] GET {url}: {e}");
                None
            }
        }
    }

    async fn post(&self, serial: &str, endpoint: &str, body: serde_json::Value) -> Result<(), String> {
        let base = self.base_url(serial).ok_or_else(|| format!("no device with serial {serial}"))?;
        let url = format!("{base}/{endpoint}");
        let response = self.http.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("{} answered {}", serial, response.status()))
        }
    }
}

// ── Strips ────────────────────────────────────────────────────────────────────

/// What one `/info` body says, read in one round trip.
struct WhatItIs {
    mains: bool,
    description: openhaunt::NodeDescription,
    effects: Option<EffectCapability>,
    audio: Option<NodeAudio>,
    pixels: Option<NodePixels>,
}

impl DeviceManager {
    /// Patch a strip head in a mode, as one gesture of `user`'s.
    ///
    /// The station works the cells out, never the browser: the spill rule is the node's
    /// and one implementation of it is enough. Then the node is told at once rather than
    /// at the next check, so the operator sees the strip change while still looking.
    async fn patch_strip(
        &mut self,
        serial: &str,
        head: u8,
        change: StripChange,
        user: Uuid,
    ) -> Result<Uuid, String> {
        if self.is_follower().await {
            return Err("only the node leading the session patches devices".into());
        }
        let fixtures = self.fixtures().await;
        let fixture = fixtures
            .iter()
            .find(|f| f.address.serial() == Some(serial) && f.address.head() == Some(head))
            .cloned()
            .ok_or_else(|| format!("{serial} has no adopted head {head}"))?;
        let current = fixture
            .address
            .strip()
            .cloned()
            .ok_or_else(|| format!("head {head} of {serial} is not a pixel strip"))?;

        let mut strip = PixelStrip {
            mode: change.mode.unwrap_or(current.mode),
            count: change.count.unwrap_or(current.count),
            order: change.order.clone().unwrap_or_else(|| current.order.clone()),
            universe: change.universe.unwrap_or(current.universe),
            address: change.address.unwrap_or(current.address),
            hold_ms: change.hold_ms.unwrap_or(current.hold_ms),
            pitch_m: change.pitch_m.unwrap_or(current.pitch_m),
            output: current.output,
        };
        // A strip going to `sacn` that was never given a universe gets the first run of
        // free ones it fits in, rather than universe 0, which sACN does not have.
        if strip.mode == StripMode::Sacn && change.universe.is_none() && strip.universe == 0 {
            strip.universe = 1;
            strip.address = 1;
            let needed = strip.universes().len() as u16;
            strip.universe = self.next_free_block(needed, fixture.id).await;
        }
        let pixels = self.state.discovered.get(serial).and_then(|d| d.pixels.clone());
        if let Some(problem) = strip.problem(pixels.as_ref()) {
            return Err(problem);
        }

        // The head takes the strip onto its address and, in `sacn` mode, the strip's own
        // type: one gesture, so a strip patched is one Ctrl-Z however many pixels it has.
        let mut patched = fixture.clone();
        if let FixtureAddress::OpenHaunt { strip: s, .. } = &mut patched.address {
            *s = Some(strip.clone());
        }
        let wanted = match strip.mode {
            StripMode::Sacn => Some(pixels::strip_type(&strip)),
            StripMode::Effect => {
                let device = self.state.discovered.get(serial);
                let description = device.and_then(|d| d.description.as_ref()).filter(|d| !d.is_empty());
                description.map(|description| {
                    let device = device.expect("a description is a device's");
                    let effects = device.pixels.as_ref().map(|p| p.effects.clone()).unwrap_or_default();
                    openhaunt::head_type_from(device.module_type, &device.module_name, description, head, &effects)
                })
            }
        };
        let gesture = Some(Uuid::new_v4());
        let mut writes: Vec<(Path, serde_json::Value)> = Vec::new();
        if let Some(wanted) = wanted {
            let types: Vec<FixtureType> = self.read("fixture_types").await;
            if !types.iter().any(|t| t.id == wanted.id) {
                writes.push((create_path("fixture_types"), serde_json::to_value(&wanted).unwrap_or_default()));
            }
            patched.fixture_type_id = wanted.id;
        }
        writes.push((replace_path("fixtures", patched.id), serde_json::to_value(&patched).unwrap_or_default()));
        for (path, value) in writes {
            self.engine
                .set_as(user, gesture, path, Lifecycle::Persisted, value)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.reconcile_strips(serial).await;
        self.publish().await;
        Ok(fixture.id)
    }

    /// Put every adopted node's strips into the shape the show says.
    async fn reconcile_all_strips(&mut self) {
        if self.is_follower().await {
            return;
        }
        let serials: Vec<String> = self
            .state
            .discovered
            .values()
            .filter(|d| d.online && !d.head_fixtures.is_empty() && d.pixels.is_some())
            .map(|d| d.serial.clone())
            .collect();
        for serial in serials {
            self.reconcile_strips(&serial).await;
        }
    }

    /// Tell a node any strip the show patches differently from how it is running.
    ///
    /// **The show is the truth.** A node that rebooted into another mode, was factory
    /// reset, or was set from a bench script is put back, the way a rebooted node is
    /// sent its outputs again. What the node will not take is a fault on its row, in the
    /// node's own words, and the next check tries again: a 409 says a stored virtual
    /// module wins over the board's strips, which only somebody at the node can undo.
    async fn reconcile_strips(&mut self, serial: &str) {
        let Some(device) = self.state.discovered.get(serial).cloned() else { return };
        if !device.online {
            return;
        }
        let fixtures = self.fixtures().await;
        let mut faults = Vec::new();
        let mut changed = false;
        for fixture in fixtures.iter().filter(|f| f.address.serial() == Some(serial)) {
            let Some(strip) = fixture.address.strip() else { continue };
            let head = fixture.address.head().unwrap_or(0);
            let running = device.pixels.as_ref().and_then(|p| p.strip_for_head(head));
            if running.is_some_and(|r| strip.agrees_with(r)) {
                continue;
            }
            match self.put(serial, &format!("pixels/{}", strip.output), strip.node_body()).await {
                Ok(()) => {
                    info!("[devices] {serial} strip {} is now {:?}", strip.output, strip.mode);
                    changed = true;
                }
                Err(e) => faults.push(format!("{}: {e}", fixture.name)),
            }
        }
        if changed {
            // The node rebuilt the strip's ports at their defaults: what anything sent
            // them before is gone, and the connectors have to be told so.
            *self.starts.entry(serial.to_string()).or_default() += 1;
            // A strip that changed mode owns other ports now. The node says so in its
            // `desc` as well, but asking straight away is what puts the parameters in
            // front of the operator who just pressed the button.
            self.refresh_description(serial).await;
            self.retype(serial).await;
        }
        let device = self.state.discovered.get_mut(serial);
        if let Some(device) = device {
            if device.faults != faults || changed {
                device.faults = faults;
                self.publish().await;
            }
        }
    }

    async fn write<T: serde::Serialize>(&self, path: Path, value: &T) -> Result<(), String> {
        let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
        self.engine.set(path, Lifecycle::Persisted, value).await.map_err(|e| e.to_string())
    }

    async fn delete(&self, table: &str, id: Uuid) -> Result<(), String> {
        self.engine
            .set(delete_path(table, id), Lifecycle::Persisted, serde_json::Value::Null)
            .await
            .map_err(|e| e.to_string())
    }

    /// `PUT` a body to the node, and say what it answered when that was not success.
    async fn put(&self, serial: &str, endpoint: &str, body: serde_json::Value) -> Result<(), String> {
        let base = self.base_url(serial).ok_or_else(|| format!("no device with serial {serial}"))?;
        let url = format!("{base}/{endpoint}");
        let response = self.http.put(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let said = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string));
        Err(match (status.as_u16(), said) {
            (409, _) => "a virtual module stored on the node wins over its strips".into(),
            (_, Some(said)) => format!("the node refused it: {said}"),
            (code, None) => format!("the node answered {code}"),
        })
    }
}

// ── mDNS ──────────────────────────────────────────────────────────────────────

/// Browse for OpenHaunt nodes and feed what turns up into the manager.
///
/// The mdns-sd receiver is blocking, so it gets its own thread and forwards over a
/// channel — the same shape `SessionManager` uses. Every node browses, follower or
/// not: seeing what is on the network costs nothing and is worth showing.
/// The broker list a leader hands its nodes: its own address first, then every other
/// station's that says where its broker is, in a stable order and at most
/// [`MAX_BROKERS`] long, with nobody listed twice.
///
/// Leader first is the whole point: a node prefers the first entry, so a list that
/// put a follower there would send every node to a door that is closed.
///
/// **One list, not one per node**, although a node on another subnet would be better
/// served by its own ordering. The list is retained on `openhaunt/brokers`, which every
/// node on this broker reads, and also posted to each node over HTTP; a node keeps
/// whichever arrived last, so a per-node HTTP list would be overwritten by the shared
/// one on the next subscribe, and the two disagreeing is a node that flips between
/// them. Instead each station is named by the one of its doors nearest the nodes as a
/// whole ([`broker::nearest_door`]) — on a rig, they are on one lighting network.
pub fn broker_list(
    own: &str,
    me: NodeId,
    stations: &[pult_schema::types::station::Station],
    nodes: &[std::net::Ipv4Addr],
) -> Vec<String> {
    let mut others: Vec<&pult_schema::types::station::Station> = stations.iter().filter(|s| s.id != me.0).collect();
    others.sort_by_key(|s| s.id);
    let mut list = vec![own.to_string()];
    for station in others {
        if list.len() == MAX_BROKERS {
            break;
        }
        // An older peer publishes only the one address.
        let doors = if station.broker_doors.is_empty() {
            vec![station.broker_addr.clone()]
        } else {
            station.broker_doors.clone()
        };
        let doors: Vec<String> = doors.into_iter().filter(|d| !d.is_empty()).collect();
        let Some(door) = broker::nearest_door(&doors, nodes) else { continue };
        if !list.contains(&door) {
            list.push(door);
        }
    }
    list
}

/// How many brokers a node is told about. The firmware keeps this many (doc 15);
/// a session of more stations than that has a spare for every node anyway.
pub const MAX_BROKERS: usize = 4;

/// The OpenHaunt browser: an mDNS daemon looking for nodes, started again whenever
/// this machine's addresses move ([`crate::infra::net::Network::address_changes`]).
///
/// Started again rather than left to follow the cable, because it does not: after a
/// Mac moved from wifi to ethernet the daemon that had been up all along reported the
/// node removed several times a second and never found it again, while a fresh one
/// found it at once. Starting again also gives a browser that was refused its cable
/// a second chance when the cable appears.
pub struct DeviceBrowser {
    current: std::sync::Arc<std::sync::Mutex<Option<ServiceDaemon>>>,
    stop: watch::Sender<bool>,
}

impl DeviceBrowser {
    /// Take the responder off the network. The daemon runs on a thread of its own, so
    /// nothing about dropping a station reaches it — and a console that has opened
    /// three shows would otherwise be three browsers on the same segment, two of them
    /// nobody is reading.
    pub fn shutdown(&self) {
        let _ = self.stop.send(true);
        if let Some(daemon) = self.current.lock().ok().and_then(|mut d| d.take()) {
            let _ = daemon.shutdown();
        }
    }
}

pub fn spawn_mdns_browser(devices: DeviceHandle, net: crate::infra::net::NetHandle) -> DeviceBrowser {
    let current = std::sync::Arc::new(std::sync::Mutex::new(None::<ServiceDaemon>));
    let (stop, mut stopped) = watch::channel(false);
    let (tx, mut rx) = mpsc::channel::<ServiceEvent>(64);

    let slot = current.clone();
    let mut moved = net.address_changes();
    tokio::spawn(async move {
        loop {
            let daemon = start_browsing(&net, tx.clone());
            let previous = match slot.lock() {
                Ok(mut held) if !*stopped.borrow() => std::mem::replace(&mut *held, daemon),
                _ => {
                    if let Some(daemon) = daemon {
                        let _ = daemon.shutdown();
                    }
                    break;
                }
            };
            if let Some(previous) = previous {
                info!("[devices] this machine's addresses moved; looking for nodes again");
                let _ = previous.shutdown();
            }
            tokio::select! {
                changed = moved.changed() => if changed.is_err() { break },
                _ = stopped.changed() => break,
            }
        }
    });

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Some(event) = device_event(event) {
                if devices.0.send(DeviceCommand::Event(event)).await.is_err() {
                    break;
                }
            }
        }
    });

    DeviceBrowser { current, stop }
}

/// One daemon, browsing, its events fed to `tx` until it is shut down.
fn start_browsing(net: &crate::infra::net::NetHandle, tx: mpsc::Sender<ServiceEvent>) -> Option<ServiceDaemon> {
    // Told a cable this machine has not got: this browser does not start. A node
    // adopted off the wrong network is a fixture patched onto a wire that cannot
    // carry it, which is worse than a node nobody found.
    let address = net
        .bind(
            pult_schema::types::network::NetService::OpenHaunt,
            "the OpenHaunt device browser",
            net.prefs().openhaunt.as_deref(),
        )
        .ok()?;
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            warn!("[devices] cannot create mDNS daemon: {e}");
            return None;
        }
    };
    crate::infra::net::restrict_mdns(&daemon, address);
    let receiver = match daemon.browse(SERVICE_TYPE) {
        Ok(r) => r,
        Err(e) => {
            warn!("[devices] mDNS browse failed: {e}");
            let _ = daemon.shutdown();
            return None;
        }
    };
    // Ends when the daemon is shut down, which closes the receiver.
    std::thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            if tx.blocking_send(event).is_err() {
                break;
            }
        }
    });
    Some(daemon)
}

fn device_event(event: ServiceEvent) -> Option<DeviceEvent> {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let txt: BTreeMap<String, String> = info
                .get_properties()
                .iter()
                .map(|p| (p.key().to_string(), p.val_str().to_string()))
                .collect();
            // The serial identifies the node; without one there is nothing to key on.
            let serial = txt.get("sn")?.clone();
            Some(DeviceEvent::Resolved {
                serial,
                ip: info.get_addresses().iter().next()?.to_string(),
                port: info.get_port(),
                host: info.get_hostname().to_string(),
                txt,
            })
        }
        ServiceEvent::ServiceRemoved(_ty, fullname) => {
            // "openhaunt-1a2b3c._openhaunt._tcp.local." → "1a2b3c"
            let instance = fullname.split('.').next()?;
            let serial = instance.strip_prefix("openhaunt-")?.to_string();
            Some(DeviceEvent::Removed { serial })
        }
        _ => None,
    }
}

// ── Path helpers ──────────────────────────────────────────────────────────────

fn create_path(table: &str) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())]
}

fn replace_path(table: &str, id: Uuid) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Id(id)]
}

fn delete_path(table: &str, id: Uuid) -> Path {
    vec![
        PathSegment::Key(table.into()),
        PathSegment::Id(id),
        PathSegment::Key("__delete".into()),
    ]
}

/// A module type as the TXT record writes it: `0x0002`, or plain decimal.
fn parse_module_type(raw: &str) -> Option<u16> {
    let raw = raw.trim();
    match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => raw.parse().ok(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod e2e_tests;
