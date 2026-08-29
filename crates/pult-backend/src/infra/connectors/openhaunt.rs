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
use pult_schema::types::fixture::{
    Fixture, FixtureAddress, ParameterDirection, ParameterValue,
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
            OutputPlugin, SendFuture,
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
        })
    }

    /// Unicast every universe a gateway is listening for, to that gateway.
    async fn feed_the_gateways(&mut self, patch: &Patch) -> Result<()> {
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
            return Ok(());
        }

        let now = std::time::Instant::now();
        // Render once for the whole patch, not once per gateway: the universes are
        // the same however many nodes are waiting for them.
        for universe in render(patch) {
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
            }
        }
        Ok(())
    }

    /// Send the ports that changed on every fixture that lives on a node.
    fn drive_the_ports(&mut self, patch: &Patch) {
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
                let Some(port) = parameter.binding.port() else { continue };
                let value = fixture
                    .live_values
                    .get(&parameter_key(&parameter.kind))
                    .unwrap_or(&parameter.default_value);
                let payload = port_payload(value);

                let key = (serial.to_string(), port);
                if self.last_sent.get(&key) == Some(&payload) {
                    continue;
                }
                debug!("[openhaunt] {serial} port {port} <- {payload}");
                self.last_sent.insert(key, payload.clone());
                self.devices.set_output(serial.to_string(), port, payload);
            }
        }
        self.forget_what_is_no_longer_patched(&patch.fixtures);
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
    }
}

impl OutputPlugin for OpenHauntOutput {
    fn name(&self) -> &'static str {
        "openhaunt"
    }

    fn send<'a>(&'a mut self, patch: &'a Patch, _changed: &'a [Uuid]) -> SendFuture<'a> {
        Box::pin(async move {
            // Nothing is patched to a node, so there is nobody to talk to.
            if !patch.fixtures.iter().any(|f| matches!(f.address, FixtureAddress::OpenHaunt { .. }))
            {
                return Ok(());
            }
            self.drive_the_ports(patch);
            self.feed_the_gateways(patch).await
        })
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
        ParameterValue::Color { r, g, b } => serde_json::json!({
            "r": to_byte(*r),
            "g": to_byte(*g),
            "b": to_byte(*b),
        }),
    }
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests;
