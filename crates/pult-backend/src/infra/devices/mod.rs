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
        devices::{DevicesState, DiscoveredDevice, NodeAudio},
        openhaunt::EffectCapability,
        fixture::{Fixture, FixtureAddress, FixtureType, ParameterBinding, ParameterDirection},
        group::Group,
        openhaunt,
        output::{OutputConfig, OutputKind},
        pixels::{self, NodePixels, PixelStrip, StripMode},
        scene::SceneObject,
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
                if !self.state.discovered.contains_key(&serial) {
                    return;
                }
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
                    strip_objects: self
                        .state
                        .discovered
                        .get(&serial)
                        .map(|d| d.strip_objects.clone())
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
    /// serialised `NodeDescription`, so a port that started advertising `effects`
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
    /// device, and a console restarted mid-show recognises every device it had
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
            device.strip_objects = on_node()
                .filter(|f| f.address.strip().is_some())
                .filter_map(|f| f.address.head())
                .map(|h| (h, pixels::strip_object_id(&device.serial, h)))
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
            let wanted =
                openhaunt::head_type_from(device.module_type, &device.module_name, description, head, &effects);
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
        // not collide on one colour.
        let effects = device.pixels.as_ref().map(|p| p.effects.clone()).unwrap_or_default();
        // A node carries a universe exactly when it says it forwards one.
        let universe = match description.dmx {
            Some(_) => Some(self.next_free_universe().await),
            None => None,
        };
        let mut fixture = None;
        for head in description.head_numbers() {
            let fixture_type = openhaunt::head_type_from(
                device.module_type,
                &device.module_name,
                description,
                head,
                &effects,
            );
            self.ensure_fixture_type(&fixture_type).await?;
            let strip = device
                .pixels
                .as_ref()
                .and_then(|p| p.strip_for_head(head))
                .map(PixelStrip::from_node);
            let name = match head {
                0 => device.name.clone(),
                _ => format!("{} {}", device.name, fixture_type.name),
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
            // A strip the node is already running in `sacn` mode is a strip with cells.
            if let Some(strip) = strip.filter(|s| s.mode == StripMode::Sacn) {
                let plan = pixels::plan(&this, strip, &[]);
                self.write_strip_plan(plan, None).await?;
            }
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
        device.strip_objects.clear();
        device.faults.clear();
        let online = device.online;

        // Every head, and what each strip in `sacn` mode put in the show for itself: its
        // cells, the object they hang off and the group that is them.
        let fixtures = self.fixtures().await;
        let heads: Vec<&Fixture> =
            fixtures.iter().filter(|f| f.address.serial() == Some(serial)).collect();
        for head in &heads {
            let object = pixels::strip_object_id(serial, head.address.head().unwrap_or(0));
            for cell in fixtures.iter().filter(|f| f.parent == Some(object)) {
                self.delete("fixtures", cell.id).await?;
            }
        }
        for head in &heads {
            self.delete("fixtures", head.id).await?;
        }
        let objects: Vec<SceneObject> = self.read("scene_objects").await;
        let groups: Vec<Group> = self.read("groups").await;
        for head in &heads {
            let number = head.address.head().unwrap_or(0);
            let object = pixels::strip_object_id(serial, number);
            if objects.iter().any(|o| o.id == object) {
                self.delete("scene_objects", object).await?;
            }
            let group = pixels::strip_group_id(serial, number);
            if groups.iter().any(|g| g.id == group) {
                self.delete("groups", group).await?;
            }
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
        let object_of = |f: &Fixture| f.address.serial().map(|s| {
            pixels::strip_object_id(s, f.address.head().unwrap_or(0))
        });
        // The strip's own cells and its own universes are not in its way.
        let mine = fixtures.iter().find(|f| f.id == except).and_then(object_of);
        let taken: std::collections::BTreeSet<u16> = fixtures
            .iter()
            .filter(|f| f.id != except && (mine.is_none() || f.parent != mine))
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

        let object = pixels::strip_object_id(serial, head);
        let cells: Vec<Fixture> =
            fixtures.into_iter().filter(|f| f.parent == Some(object)).collect();
        let plan = pixels::plan(&fixture, strip, &cells);
        self.write_strip_plan(plan, Some((user, Uuid::new_v4()))).await?;
        self.reconcile_strips(serial).await;
        self.publish().await;
        Ok(fixture.id)
    }

    /// Make the writes a strip plan asks for.
    ///
    /// Under one gesture when somebody asked for it, so a strip patched with three
    /// hundred cells is one Ctrl-Z. Unattributed when adoption does it, the way adoption
    /// writes every other row.
    async fn write_strip_plan(
        &self,
        plan: pixels::StripPlan,
        by: Option<(Uuid, Uuid)>,
    ) -> Result<(), String> {
        let objects: Vec<SceneObject> = self.read("scene_objects").await;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        let groups: Vec<Group> = self.read("groups").await;
        let mut writes: Vec<(Path, serde_json::Value)> = Vec::new();
        let value = |v: &dyn erased::Json| v.json();

        if let Some(cell_type) = &plan.cell_type {
            if !types.iter().any(|t| t.id == cell_type.id) {
                writes.push((create_path("fixture_types"), value(cell_type)));
            }
        }
        let wants_object = plan.cell_type.is_some();
        if wants_object && !objects.iter().any(|o| o.id == plan.object.id) {
            writes.push((create_path("scene_objects"), value(&plan.object)));
        }
        writes.push((replace_path("fixtures", plan.head.id), value(&plan.head)));
        for id in &plan.delete {
            writes.push((delete_path("fixtures", *id), serde_json::Value::Null));
        }
        for cell in &plan.update {
            writes.push((replace_path("fixtures", cell.id), value(cell)));
        }
        for cell in &plan.create {
            writes.push((create_path("fixtures"), value(cell)));
        }
        match &plan.group {
            Some(group) => {
                let path = if groups.iter().any(|g| g.id == group.id) {
                    replace_path("groups", group.id)
                } else {
                    create_path("groups")
                };
                writes.push((path, value(group)));
            }
            None => {
                let id = plan.object.id;
                let group = groups.iter().find(|g| {
                    g.query.clauses.iter().any(|c| {
                        c.term == pult_schema::types::group::SelectionTerm::Hanging { object: id }
                    })
                });
                if let Some(group) = group {
                    writes.push((delete_path("groups", group.id), serde_json::Value::Null));
                }
            }
        }

        for (path, value) in writes {
            let written = match by {
                Some((user, gesture)) => {
                    self.engine.set_as(user, Some(gesture), path, Lifecycle::Persisted, value).await
                }
                None => self.engine.set(path, Lifecycle::Persisted, value).await,
            };
            written.map_err(|e| e.to_string())?;
        }
        Ok(())
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

/// Anything that can be written as a row.
mod erased {
    pub trait Json {
        fn json(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or_default()
        }
    }
}

// ── mDNS ──────────────────────────────────────────────────────────────────────

/// Browse for OpenHaunt nodes and feed what turns up into the manager.
///
/// The mdns-sd receiver is blocking, so it gets its own thread and forwards over a
/// channel — the same shape `SessionManager` uses. Every node browses, follower or
/// not: seeing what is on the network costs nothing and is worth showing.
pub fn spawn_mdns_browser(
    devices: DeviceHandle,
    net: crate::infra::net::NetHandle,
) -> Option<ServiceDaemon> {
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
            return None;
        }
    };
    // Handed back so a station being stopped can take its responder off the network.
    // The daemon runs on a thread of its own, so nothing about dropping this station
    // reaches it — and a console that has opened three shows would otherwise be three
    // browsers on the same segment, two of them nobody is reading.
    let handle = daemon.clone();

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

    Some(handle)
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
