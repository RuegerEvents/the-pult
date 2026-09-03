//! Output to OpenHaunt nodes.
//!
//! Two different things, because a node's module decides which one it is:
//!
//! - A DMX gateway forwards a whole universe, so it gets an E1.31 frame unicast to
//!   it — the same packet [`super::sacn`] multicasts, addressed to one node.
//! - Everything else has ports. A relay, a strip, a display: each parameter is one
//!   value, sent when it changes and not otherwise, because there is no frame rate
//!   to keep up and a display redrawn 40 times a second is a display on fire.
//!
//! Registered unconditionally. It only ever talks to devices that have been
//! adopted, so a console with no OpenHaunt nodes sends nothing and the rule that a
//! console puts no packets on a network it was not asked to still holds.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use anyhow::Result;
use pult_schema::types::{
    effect::{Curve, Direction, Easing, RunningEffect, RunningFade, Shape},
    fixture::{Fixture, FixtureAddress, ParameterDirection, ParameterValue},
    openhaunt::PortEffectCapability,
    output::{MessageTraffic, OutputMessage, OutputSection, SectionBody},
};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tracing::debug;
use uuid::Uuid;

use crate::{
    infra::{
        connectors::{
            dmx::{render, Patch, SequenceCounter, UniverseCache, REFRESH_AFTER},
            sacn::e131_data_packet,
            Frame, Frames, OutputPlugin, SendFuture,
        },
        devices::{DeviceDirectory, DeviceHandle},
    },
    model::playback::parameter_key,
};

pub struct OpenHauntOutput {
    directory: watch::Receiver<DeviceDirectory>,
    devices: DeviceHandle,
    socket: UdpSocket,
    cid: [u8; 16],
    /// The port gateways are told to listen on. Fixed in the field; a parameter
    /// only so a test can have one that is not 5568.
    sacn_port: u16,
    sent: UniverseCache,
    sequence: SequenceCounter,
    /// The last value sent for each port, keyed by (serial, port), so a relay is
    /// commanded when it changes rather than forty times a second.
    last_sent: BTreeMap<(String, u8), serde_json::Value>,
    /// Which nodes were online at the last send. A node that comes back has
    /// rebooted and sits at its defaults, so what it was last sent no longer
    /// describes it and is forgotten, which makes every port get sent again.
    was_online: BTreeMap<String, bool>,
    /// The last shape handed to each port, so a description that has not changed is
    /// not sent again. This is what turns a chase into one message: the value it
    /// renders moves on every tick, but the description of it does not.
    last_sent_effect: BTreeMap<(String, u8), serde_json::Value>,
    /// Ports currently tracing a shape for themselves. A port leaving this set has to
    /// be told to stop before it is sent a value again, or it would keep tracing.
    offloaded: std::collections::BTreeSet<(String, u8)>,
    /// What has been said to the nodes, for somebody watching.
    said: MessageRing,
}

impl OpenHauntOutput {
    pub async fn new(
        directory: watch::Receiver<DeviceDirectory>,
        devices: DeviceHandle,
        sacn_port: u16,
    ) -> Result<Self> {
        Ok(Self {
            directory,
            devices,
            socket: UdpSocket::bind("0.0.0.0:0").await?,
            cid: *Uuid::new_v4().as_bytes(),
            sacn_port,
            sent: UniverseCache::default(),
            sequence: SequenceCounter::default(),
            last_sent: BTreeMap::new(),
            was_online: BTreeMap::new(),
            last_sent_effect: BTreeMap::new(),
            offloaded: Default::default(),
            said: MessageRing::default(),
        })
    }

    /// Unicast every universe a gateway is listening for, to that gateway.
    /// Answers what it spent evaluating, and what it put on the wire doing it.
    async fn feed_the_gateways(
        &mut self,
        patch: &Patch,
        now_ms: u64,
    ) -> Result<(std::time::Duration, u64, u32)> {
        let gateways: Vec<(String, u16, SocketAddr)> = self
            .directory
            .borrow()
            .entries
            .iter()
            .filter(|(_, entry)| entry.online)
            .filter_map(|(serial, entry)| {
                let universe = entry.universe?;
                let ip: IpAddr = entry.ip.parse().ok()?;
                Some((serial.clone(), universe, SocketAddr::new(ip, self.sacn_port)))
            })
            .collect();
        if gateways.is_empty() {
            return Ok((std::time::Duration::ZERO, 0, 0));
        }

        let now = std::time::Instant::now();
        // Render once for the whole patch, not once per gateway: the universes are
        // the same however many nodes are waiting for them.
        let universes = render(patch, now_ms);
        let evaluating = now.elapsed();
        let mut bytes = 0u64;
        let mut packets = 0u32;
        for universe in universes {
            let listening: Vec<&SocketAddr> = gateways
                .iter()
                .filter(|(_, n, _)| *n == universe.number)
                .map(|(_, _, addr)| addr)
                .collect();
            if listening.is_empty() {
                continue;
            }
            if !self.sent.needs_send(&universe, now, REFRESH_AFTER) {
                continue;
            }
            let sequence = self.sequence.next(universe.number);
            let packet = e131_data_packet(
                &self.cid,
                "the-pult",
                universe.number,
                sequence,
                100,
                &universe.channels,
            );
            for addr in listening {
                self.socket.send_to(&packet, addr).await?;
                bytes += packet.len() as u64;
                packets += 1;
            }
        }
        Ok((evaluating, bytes, packets))
    }

    /// Send each port whatever is the least it needs to hear.
    ///
    /// In order of preference: a shape it can trace on its own, a fade it can run on
    /// its own, or the stream of values every node has always got. The first two are
    /// sent once and then nothing more, which is the whole point — a three second
    /// fade was a hundred and twenty messages and is now one.
    ///
    /// Which of the three applies is decided by what is *driving* the port, in the
    /// same priority order every other consumer uses. A parameter the programmer has
    /// hold of is a value however much is running underneath it: the fade under it is
    /// still published, because letting go must not need the station to republish the
    /// rig, and a node handed it would trace something nobody is looking at.
    fn drive_the_ports(&mut self, patch: &Patch, now_ms: u64) {
        self.forget_what_rebooted_nodes_were_sent();
        for fixture in &patch.fixtures {
            let Some(serial) = fixture.address.serial() else { continue };
            if !self.is_online(serial) {
                continue;
            }
            let Some(fixture_type) = patch.fixture_type(fixture) else { continue };

            for parameter in &fixture_type.parameters {
                if parameter.direction != ParameterDirection::Output {
                    continue;
                }
                let Some(port) = parameter.binding.and_then(|binding| binding.port()) else { continue };
                let param_key = parameter_key(&parameter.kind);
                let key = (serial.to_string(), port);
                let capable = self.capability(serial, port);
                let driving = patch.driving(fixture, &param_key);

                // 1. A shape the port has said it can trace.
                if let Some(effect) = driving.effect.filter(|_| driving.programmer.is_none()) {
                    if let Some(payload) = capable
                        .as_ref()
                        .filter(|c| supports(c, effect))
                        .and_then(|_| effect_payload(effect))
                    {
                        if self.last_sent_effect.get(&key) != Some(&payload) {
                            debug!("[openhaunt] {serial} port {port} traces {payload}");
                            self.said.note(serial, port, "traces", &payload);
                            self.last_sent_effect.insert(key.clone(), payload.clone());
                            self.devices.set_effect(serial.to_string(), port, Some(payload));
                        }
                        self.offloaded.insert(key);
                        continue;
                    }
                }

                // 2. Not tracing, but it was: take the shape back before anything else.
                if self.offloaded.remove(&key) {
                    debug!("[openhaunt] {serial} port {port} stops tracing");
                    self.said.note(serial, port, "stops tracing", &serde_json::Value::Null);
                    self.last_sent_effect.remove(&key);
                    self.devices.set_effect(serial.to_string(), port, None);
                    // The node has gone back to whatever value it was left holding,
                    // which is not what this plugin last recorded sending it, so the
                    // value below has to go out even if it looks unchanged.
                    self.last_sent.remove(&key);
                }

                // 3. A fade the port can run itself, described once.
                if let Some(fade) = driving
                    .fade
                    .filter(|_| driving.programmer.is_none() && driving.effect.is_none())
                {
                    if capable.as_ref().is_some_and(|c| c.transitions) {
                        let payload = transition_payload(fade);
                        if self.last_sent_effect.get(&key) != Some(&payload) {
                            debug!("[openhaunt] {serial} port {port} fades {payload}");
                            self.said.note(serial, port, "fades", &payload);
                            self.last_sent_effect.insert(key.clone(), payload.clone());
                            // Where the fade is going is where the port will be when
                            // it lands, so recording it now means the arrival is not
                            // sent again as a value afterwards.
                            self.last_sent.insert(key, port_payload(&fade.to));
                            self.devices.set_output(serial.to_string(), port, payload);
                        }
                        continue;
                    }
                }

                // 4. The stream of values, as it has always been — worked out here for
                // this frame's moment rather than read off a fixture.
                let value = pult_render::value_at(&driving, now_ms)
                    .unwrap_or_else(|| parameter.default_value.clone());
                let payload = port_payload(&value);
                if self.last_sent.get(&key) == Some(&payload) {
                    continue;
                }
                debug!("[openhaunt] {serial} port {port} <- {payload}");
                self.said.note(serial, port, "value", &payload);
                self.last_sent.insert(key, payload.clone());
                self.devices.set_output(serial.to_string(), port, payload);
            }
        }
        self.forget_what_is_no_longer_patched(&patch.fixtures);
    }

    /// What this port said it could do for itself, if anything.
    fn capability(&self, serial: &str, port: u8) -> Option<PortEffectCapability> {
        self.directory
            .borrow()
            .entries
            .get(serial)
            .and_then(|entry| entry.effects.as_ref())
            .and_then(|caps| caps.port(port))
            .cloned()
    }

    /// A node that went offline and came back is at its defaults, whatever it was
    /// sent before: drop the memory of it so the next pass sends everything.
    fn forget_what_rebooted_nodes_were_sent(&mut self) {
        let now: Vec<(String, bool)> = self
            .directory
            .borrow()
            .entries
            .iter()
            .map(|(serial, entry)| (serial.clone(), entry.online))
            .collect();
        for (serial, online) in now {
            let before = self.was_online.insert(serial.clone(), online).unwrap_or(false);
            if online && !before {
                self.last_sent.retain(|(s, _), _| *s != serial);
                // A rebooted node is not tracing anything either, whatever it was
                // handed before it went. Forgetting is what makes the shape go out
                // again on the next pass.
                self.last_sent_effect.retain(|(s, _), _| *s != serial);
                self.offloaded.retain(|(s, _)| *s != serial);
            }
        }
    }

    fn is_online(&self, serial: &str) -> bool {
        self.directory.borrow().entries.get(serial).is_some_and(|entry| entry.online)
    }

    /// Drop remembered values for devices nothing is patched to any more, so
    /// re-adopting one starts from a clean slate rather than a stale comparison.
    fn forget_what_is_no_longer_patched(&mut self, fixtures: &[Fixture]) {
        let patched: std::collections::BTreeSet<&str> =
            fixtures.iter().filter_map(|f| f.address.serial()).collect();
        self.last_sent.retain(|(serial, _), _| patched.contains(serial.as_str()));
        self.last_sent_effect.retain(|(serial, _), _| patched.contains(serial.as_str()));
        self.offloaded.retain(|(serial, _)| patched.contains(serial.as_str()));
    }
}

impl OutputPlugin for OpenHauntOutput {
    fn name(&self) -> &'static str {
        "openhaunt"
    }

    /// A frame while something moves, and the DMX family's keep-alive when nothing
    /// does — because a gateway is fed E1.31 and expects to keep hearing it. The ports
    /// themselves are told only what changed, so a settled rig of relays and displays
    /// costs one comparison per port per keep-alive and puts nothing on the wire.
    fn frames(&self) -> Frames {
        Frames::DMX
    }

    fn send<'a>(
        &'a mut self,
        patch: &'a Patch,
        _changed: &'a [Uuid],
        now_ms: u64,
    ) -> SendFuture<'a> {
        Box::pin(async move {
            // Nothing is patched to a node, so there is nobody to talk to.
            if !patch.fixtures.iter().any(|f| matches!(f.address, FixtureAddress::OpenHaunt { .. }))
            {
                return Ok(Frame::default());
            }
            let began = std::time::Instant::now();
            self.drive_the_ports(patch, now_ms);
            // The ports are worked out here and only queued, so this whole call is
            // evaluating. What the gateways cost is measured inside their own render.
            let mut frame = Frame::evaluated(began.elapsed());
            // What leaves here is the sACN this station sends to gateway nodes. The
            // per-port commands a node itself is given travel over MQTT from the
            // device manager, on its own schedule and not inside any frame, so they
            // are not in this figure — which the panel says. That omission is small by
            // construction: a three-second fade is one message to a node that can run
            // it, not a hundred and twenty.
            let (evaluating, bytes, packets) = self.feed_the_gateways(patch, now_ms).await?;
            frame.evaluating += evaluating;
            frame.bytes += bytes;
            frame.packets += packets;
            Ok(frame)
        })
    }

    /// A ring costs something per message, so it is kept only while it is read.
    fn watched(&mut self, watching: bool) {
        self.said.reading(watching);
    }

    /// Two sections, because this connector is two things.
    ///
    /// What a node is told about its ports is discrete and goes out when it changes;
    /// what a gateway is fed is a universe forty times a second. A viewer that showed
    /// one of them would be describing half of what left the station — and which half
    /// depends on what somebody happens to have patched.
    fn observe(&mut self, focus: Option<&str>) -> Option<Vec<OutputSection>> {
        let gateways = self.sent.observe(focus, std::time::Instant::now());
        let mut sections = vec![OutputSection {
            title: "To the nodes".to_string(),
            // Said here because nothing else can say it: these do not travel inside a
            // frame, so they are absent from this connector's row in the System panel
            // and a reader comparing the two would otherwise find them missing.
            note: Some(
                "Port commands, sent when they change. They travel over MQTT from the                  device manager rather than inside an output frame, so they are not in                  this connector's byte count."
                    .to_string(),
            ),
            body: SectionBody::Messages(self.said.drain()),
        }];
        if !gateways.universes.is_empty() {
            sections.push(OutputSection {
                title: "sACN to the gateways".to_string(),
                note: None,
                body: SectionBody::Universes(gateways),
            });
        }
        Some(sections)
    }
}

/// The last few things said to nodes, kept while somebody is reading them.
///
/// Off by default and empty when off, which is the whole design: a rig of two hundred
/// relays says something every time one of them moves, and keeping that for a viewer
/// nobody has open would be a cost paid for ever against a benefit paid never. The
/// ring is bounded, and what it throws away it counts — a silent hole in a diagnostic
/// is worse than a visible one, which is the same rule the log's `seq` gap follows.
#[derive(Default)]
pub struct MessageRing {
    reading: bool,
    messages: std::collections::VecDeque<OutputMessage>,
    dropped: u64,
}

/// How many messages are held between two looks. At a tenth of a second apart, a rig
/// would have to move two thousand ports a second to overflow it.
const RING: usize = 200;

impl MessageRing {
    /// Somebody opened a viewer, or closed one. Closing throws away what was held:
    /// it is a picture of what happened while somebody was looking, and nobody was.
    fn reading(&mut self, on: bool) {
        self.reading = on;
        if !on {
            self.messages.clear();
            self.dropped = 0;
        }
    }

    fn note(&mut self, serial: &str, port: u8, what: &str, detail: &serde_json::Value) {
        if !self.reading {
            return;
        }
        if self.messages.len() >= RING {
            self.messages.pop_front();
            self.dropped += 1;
        }
        self.messages.push_back(OutputMessage {
            at_ms: pult_schema::types::sequence::now_ms(),
            to: format!("{serial} port {port}"),
            what: what.to_string(),
            detail: if detail.is_null() { String::new() } else { detail.to_string() },
        });
    }

    /// What has been said since the last look, and nothing twice.
    ///
    /// Drained rather than kept, because the connector's ring is bounded by what it
    /// can afford and the reader's by what it can read: the panel keeps the history.
    fn drain(&mut self) -> MessageTraffic {
        MessageTraffic {
            messages: self.messages.drain(..).collect(),
            dropped: std::mem::take(&mut self.dropped),
        }
    }
}

/// What a port expects, per data type rather than per module.
///
/// The node described each port as a boolean, a number, a string or a colour, and
/// the shape follows from that alone — which is why there is nothing here that has
/// to know a relay from a strip.
fn port_payload(value: &ParameterValue) -> serde_json::Value {
    match value {
        ParameterValue::Bool(state) => serde_json::json!({ "state": state }),
        ParameterValue::Float(level) => serde_json::json!({ "value": level }),
        ParameterValue::Int(n) => serde_json::json!({ "value": n }),
        ParameterValue::Text(text) => serde_json::json!({ "text": text }),
        ParameterValue::Color { r, g, b, .. } => serde_json::json!({
            "r": to_byte(*r),
            "g": to_byte(*g),
            "b": to_byte(*b),
        }),
    }
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

// ── What a node is told instead of values ─────────────────────────────────────

/// The wire spelling of a shape. Lower case and hyphenated, as the port advertises
/// them, so what a node says it can do and what it is asked to do are the same string.
fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Sine => "sine",
        Shape::Triangle => "triangle",
        Shape::Square => "square",
        Shape::SawUp => "saw-up",
        Shape::SawDown => "saw-down",
    }
}

fn easing_name(easing: Easing) -> &'static str {
    match easing {
        Easing::Step => "step",
        Easing::Linear => "linear",
        Easing::EaseIn => "ease-in",
        Easing::EaseOut => "ease-out",
        Easing::EaseInOut => "ease-in-out",
    }
}

/// Whether this port can be trusted with this particular effect.
///
/// Asked shape by shape rather than as a single flag, because a relay that can chop
/// a square wave has no way to trace a sine and saying so is cheaper than finding out.
/// A node that cannot is not sent a description it would ignore in silence; it gets
/// the stream of values it has always had.
fn supports(capability: &PortEffectCapability, effect: &RunningEffect) -> bool {
    match &effect.curve {
        Curve::Shape(shape) => capability.has_shape(shape_name(*shape)),
        Curve::Steps(steps) => capability.steps && !steps.is_empty(),
    }
}

/// One effect, in the shape a node reads.
///
/// Values go out in the port's own payload shape rather than the schema's, so a node
/// parses the endpoints of a shape with exactly the code it already uses for a `set`.
fn effect_payload(effect: &RunningEffect) -> Option<serde_json::Value> {
    let curve = match &effect.curve {
        Curve::Shape(shape) => serde_json::json!({ "shape": shape_name(*shape) }),
        Curve::Steps(steps) => serde_json::json!({
            "steps": steps
                .iter()
                .map(|step| serde_json::json!({
                    "at": step.at,
                    "value": port_payload(&step.value),
                    "easing": easing_name(step.easing),
                }))
                .collect::<Vec<_>>(),
        }),
    };

    Some(serde_json::json!({
        "id": effect.effect_id,
        "curve": curve,
        "rate": effect.rate_hz,
        "phase": effect.phase,
        "direction": match effect.direction {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        },
        "width": effect.width,
        "low": port_payload(&effect.low),
        "high": port_payload(&effect.high),
        "t0": effect.t0,
    }))
}

/// A fade, as a `set` that says when rather than a `set` repeated forty times a second.
///
/// The destination is at the top level in the port's ordinary payload shape, so a node
/// that ignores the timing keys entirely still lands on the right value — just
/// immediately. That is the fallback the whole design leans on: unknown keys are
/// harmless, so nothing has to be negotiated.
fn transition_payload(fade: &RunningFade) -> serde_json::Value {
    let mut payload = port_payload(&fade.to);
    if let Some(map) = payload.as_object_mut() {
        map.insert("fade_ms".into(), fade.duration_ms.into());
        map.insert("t0".into(), fade.t0.into());
        map.insert("curve".into(), easing_name(fade.easing).into());
    }
    payload
}

#[cfg(test)]
mod tests;
