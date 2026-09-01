//! OpenHaunt I/O nodes: finding them, adopting them, and keeping track of them.
//!
//! Modelled on [`crate::infra::session::SessionManager`] — an actor owning a piece
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
        devices::{DevicesState, DiscoveredDevice},
        openhaunt::EffectCapability,
        fixture::{Fixture, FixtureAddress, FixtureType, ParameterBinding, ParameterDirection},
        openhaunt,
        output::{OutputConfig, OutputKind},
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
    /// Drive one output port on a node.
    SetOutput { serial: String, port: u8, value: serde_json::Value },
    /// Hand one output port a shape to trace, or `None` to take it back.
    SetEffect { serial: String, port: u8, payload: Option<serde_json::Value> },
    Event(DeviceEvent),
    Stop,
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

    /// Drive an output port. Never blocks: a device that is not keeping up must not
    /// hold up whatever produced the value.
    pub fn set_output(&self, serial: String, port: u8, value: serde_json::Value) {
        let _ = self.0.try_send(DeviceCommand::SetOutput { serial, port, value });
    }

    /// Hand an output port a shape, or `None` to stop it tracing one. Same rule.
    pub fn set_effect(&self, serial: String, port: u8, payload: Option<serde_json::Value>) {
        let _ = self.0.try_send(DeviceCommand::SetEffect { serial, port, payload });
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
    pub universe: Option<u16>,
    pub online: bool,
    /// Which of this node's ports can trace a shape for themselves.
    ///
    /// Here as well as on the discovered device because an output plugin is handed
    /// the directory and never the device list — it has to decide port by port
    /// whether to send a description or a stream of samples, and this is where it
    /// looks.
    pub effects: Option<EffectCapability>,
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
    /// The connection to the broker. Present only while driving.
    mqtt: Option<MqttLink>,
    mqtt_tx: mpsc::Sender<MqttEvent>,
    /// Devices that have announced themselves on the broker. A node reachable this
    /// way is driven over MQTT; one that has been adopted but has not connected yet
    /// still has to be reachable, and for that there is HTTP.
    on_broker: std::collections::BTreeSet<String>,
    /// Counts up with every clock publish, so a node can tell a fresh sample from a
    /// retained one replayed after the broker restarted.
    clock_seq: u64,
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
                mqtt: None,
                mqtt_tx,
                on_broker: Default::default(),
                clock_seq: 0,
            },
            DeviceHandle(tx),
            directory_rx,
        )
    }

    pub async fn run(mut self) {
        let (mqtt_tx, mut mqtt_rx) = mpsc::channel(256);
        self.mqtt_tx = mqtt_tx;

        // Leadership changes arrive as writes to the LOCAL session state, which is
        // the same thing playback gates follow cues on.
        let mut sessions = self.engine.subscribe_pattern(PathPattern::new("session")).await;

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
                        DeviceCommand::SetOutput { serial, port, value } => {
                            self.set_output(&serial, port, value).await;
                        }
                        DeviceCommand::SetEffect { serial, port, payload } => {
                            self.set_effect(&serial, port, payload).await;
                        }
                    }
                }
                Some(event) = mqtt_rx.recv() => self.handle_mqtt(event).await,
                Some(_) = sessions.next() => self.reconsider_leadership().await,
                _ = clock.tick() => self.publish_clock().await,
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
                // The broker thread stays up — it is started once per process — but
                // this node is no longer the one nodes should publish to.
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
        let listen = broker::ensure(self.broker_port);
        let advertised = broker::advertised_addr(
            std::net::IpAddr::V4(crate::infra::local_ipv4()),
            listen.port(),
        );
        self.state.broker_addr = Some(advertised.clone());

        // The console reaches its own broker over loopback whatever it advertises.
        // The client id carries the node id: a broker disconnects the older of two
        // clients claiming the same one, so two consoles sharing a broker would
        // otherwise take turns kicking each other off.
        let local = format!("127.0.0.1:{}", listen.port());
        let client_id = format!("the-pult-{}", &self.node_id.0.to_string()[..8]);
        self.mqtt = Some(MqttLink::connect(&local, &client_id, self.mqtt_tx.clone()));
        info!("[devices] driving devices, broker advertised as {advertised}");

        self.ensure_openhaunt_output().await;

        // A device adopted by a previous leader is pointing at a broker that is no
        // longer listening, so every online one has to be told where to look now.
        for serial in self.adopted_and_online() {
            self.push_config(&serial).await;
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
                let Some(device) = self.state.discovered.get(&serial) else { return };
                let Some(fixture_id) = device.adopted_fixture_id else { return };
                let Some(key) = self.parameter_on_port(fixture_id, port).await else {
                    debug!("[devices] {serial} port {port} is not bound to a parameter");
                    return;
                };
                let value = serde_json::to_value(&value).unwrap_or_default();
                if let Err(e) = self.engine.set_live_value(fixture_id, key, value).await {
                    warn!("[devices] {serial} port {port}: {e}");
                }
            }
            MqttEvent::Status { serial, online } => {
                if online {
                    self.on_broker.insert(serial.clone());
                } else {
                    self.on_broker.remove(&serial);
                }
                let Some(device) = self.state.discovered.get_mut(&serial) else { return };
                if device.online == online {
                    return;
                }
                device.online = online;
                self.publish().await;
            }
            MqttEvent::Health { serial, health } => {
                let Some(device) = self.state.discovered.get_mut(&serial) else { return };
                device.health = Some(health);
                self.publish().await;
            }
        }
    }

    /// The `live_values` key for whatever is bound to a port on a fixture.
    ///
    /// A node numbers its own terminals, so the port is the only thing both ends
    /// agree on; the parameter's kind is this console's business alone.
    async fn parameter_on_port(&self, fixture_id: Uuid, port: u8) -> Option<String> {
        let fixture = self.fixtures().await.into_iter().find(|f| f.id == fixture_id)?;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        let fixture_type = types.into_iter().find(|t| t.id == fixture.fixture_type_id)?;
        fixture_type
            .parameters
            .iter()
            .find(|p| {
                p.direction == ParameterDirection::Input
                    && p.binding == ParameterBinding::Port { index: port }
            })
            .map(|p| parameter_key(&p.kind))
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

                if let Some((mains, description, effects)) =
                    self.ask_the_node_what_it_is(&serial).await
                {
                    if let Some(device) = self.state.discovered.get_mut(&serial) {
                        device.is_mains = mains;
                        device.description = Some(description);
                        device.effects = effects;
                    }
                }
                self.reconcile_adoptions().await;

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
    /// serialised `NodeDescription`, so a port that started advertising `effects`
    /// through that struct would give every node of its kind a fresh fixture type
    /// and orphan every parameter already patched against the old one. Serde drops
    /// the unknown key, the id stays put, and the capability is picked up alongside.
    async fn ask_the_node_what_it_is(
        &self,
        serial: &str,
    ) -> Option<(bool, openhaunt::NodeDescription, Option<EffectCapability>)> {
        let info: serde_json::Value = self.get_json(serial, "info").await?;
        let mains = info["module"]["flags"]
            .as_u64()
            .is_some_and(|flags| flags as u32 & openhaunt::MODULE_FLAG_MAINS != 0);
        let effects = openhaunt::effect_capability_from(&info);
        // The whole body, not just the two keys: `ports` and `dmx` both default,
        // so firmware that predates self-description parses as having neither.
        let description = serde_json::from_value(info).unwrap_or_default();
        Some((mains, description, effects))
    }

    /// Work out which devices are adopted, from the fixtures rather than from memory.
    ///
    /// The fixture is the only persisted thing an adopted device has, so it is the
    /// answer in both directions: a fixture the operator deleted un-adopts its
    /// device, and a console restarted mid-show recognises every device it had
    /// adopted before, which is what tells it to configure them again.
    async fn reconcile_adoptions(&mut self) {
        let fixtures = self.fixtures().await;
        for device in self.state.discovered.values_mut() {
            device.adopted_fixture_id = fixtures
                .iter()
                .find(|f| f.address.serial() == Some(device.serial.as_str()))
                .map(|f| f.id);
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

        let fixture_type =
            openhaunt::fixture_type_from(device.module_type, &device.module_name, description);
        self.ensure_fixture_type(&fixture_type).await?;

        // A node carries a universe exactly when it says it forwards one.
        let universe = match description.dmx {
            Some(_) => Some(self.next_free_universe().await),
            None => None,
        };

        let fixture = Fixture {
            id: Uuid::new_v4(),
            name: device.name.clone(),
            fixture_type_id: fixture_type.id,
            address: FixtureAddress::OpenHaunt { serial: serial.to_string(), universe },
            position: None,
            live_values: Default::default(),
            live_effects: Default::default(),
            live_fades: Default::default(),
            home_values: Default::default(),
        };
        let value = serde_json::to_value(&fixture).map_err(|e| e.to_string())?;
        self.engine
            .set(create_path("fixtures"), Lifecycle::Persisted, value)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(device) = self.state.discovered.get_mut(serial) {
            device.adopted_fixture_id = Some(fixture.id);
        }
        info!("[devices] adopted {serial} as fixture {}", fixture.id);

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
        let fixture_id = device.adopted_fixture_id.take();
        let online = device.online;

        if let Some(id) = fixture_id {
            self.engine
                .set(delete_path("fixtures", id), Lifecycle::Persisted, serde_json::Value::Null)
                .await
                .map_err(|e| e.to_string())?;
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
        let Some(broker) = &self.state.broker_addr else { return };
        let mut config = serde_json::json!({ "mqtt": { "broker": broker } });

        if let Some(universe) = self.gateway_universe(serial).await {
            config["dmx"] = serde_json::json!({ "protocol": "sacn", "universe": universe });
        }
        if let Err(e) = self.post(serial, "config", config).await {
            debug!("[devices] could not configure {serial}: {e}");
        }
    }

    /// The universe an adopted gateway forwards, if this device is one.
    async fn gateway_universe(&self, serial: &str) -> Option<u16> {
        self.fixtures().await.into_iter().find_map(|f| match f.address {
            FixtureAddress::OpenHaunt { serial: ref s, universe } if s == serial => universe,
            _ => None,
        })
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
            .filter_map(|f| match &f.address {
                FixtureAddress::Dmx { universe, .. } => Some(*universe),
                FixtureAddress::OpenHaunt { universe, .. } => *universe,
            })
            .collect();
        (1..=u16::MAX).find(|n| !taken.contains(n)).unwrap_or(1)
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
        let universes: BTreeMap<String, Option<u16>> = self
            .fixtures()
            .await
            .into_iter()
            .filter_map(|f| match f.address {
                FixtureAddress::OpenHaunt { serial, universe } => Some((serial, universe)),
                FixtureAddress::Dmx { .. } => None,
            })
            .collect();

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
                            universe: universes.get(&d.serial).copied().flatten(),
                            online: d.online,
                            effects: d.effects.clone(),
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

// ── mDNS ──────────────────────────────────────────────────────────────────────

/// Browse for OpenHaunt nodes and feed what turns up into the manager.
///
/// The mdns-sd receiver is blocking, so it gets its own thread and forwards over a
/// channel — the same shape `SessionManager` uses. Every node browses, follower or
/// not: seeing what is on the network costs nothing and is worth showing.
pub fn spawn_mdns_browser(devices: DeviceHandle) {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            warn!("[devices] cannot create mDNS daemon: {e}");
            return;
        }
    };
    let receiver = match daemon.browse(SERVICE_TYPE) {
        Ok(r) => r,
        Err(e) => {
            warn!("[devices] mDNS browse failed: {e}");
            return;
        }
    };

    let (tx, mut rx) = mpsc::channel::<ServiceEvent>(64);
    std::thread::spawn(move || {
        // Hold the daemon: dropping it stops the browse.
        let _daemon = daemon;
        while let Ok(event) = receiver.recv() {
            if tx.blocking_send(event).is_err() {
                break;
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
