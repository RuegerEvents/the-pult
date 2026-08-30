//! The MQTT side of talking to OpenHaunt nodes.
//!
//! Topics, as the protocol documents them:
//!
//! - `openhaunt/<serial>/status`      — retained `online`/`offline`, `offline` as LWT
//! - `openhaunt/<serial>/input/<n>`   — an edge or a reading, with the node's timestamp
//! - `openhaunt/<serial>/health`      — uptime, temperature, PoE class, errors
//! - `openhaunt/<serial>/output/<n>/set` — what the console publishes
//!
//! Parsing is pure and separate from the connection, because a malformed payload
//! from a node is the kind of thing that should be a failing test rather than a
//! surprise on site.

use std::time::Duration;

use pult_schema::types::{devices::DeviceHealth, fixture::ParameterValue};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Everything the console subscribes to, as one wildcard per shape.
const SUBSCRIPTIONS: &[&str] =
    &["openhaunt/+/status", "openhaunt/+/input/+", "openhaunt/+/health"];

#[derive(Debug, Clone, PartialEq)]
pub enum MqttEvent {
    /// A node said whether it is there. `offline` also arrives as its will.
    Status { serial: String, online: bool },
    /// A port changed, or a sensor reported. `node_ts` is the node's own clock in
    /// milliseconds, kept because it is closer to when the thing happened than the
    /// moment the console got round to reading the socket.
    Input { serial: String, port: u8, value: ParameterValue, node_ts: Option<u64> },
    Health { serial: String, health: DeviceHealth },
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// One published message, as an event. None for a topic or payload that makes no
/// sense — an unknown topic under `openhaunt/` is a node newer than this console,
/// which is not an error worth logging on every message.
pub fn parse(topic: &str, payload: &[u8]) -> Option<MqttEvent> {
    let mut parts = topic.split('/');
    if parts.next()? != "openhaunt" {
        return None;
    }
    let serial = parts.next()?.to_string();

    match (parts.next()?, parts.next()) {
        ("status", None) => {
            let body = std::str::from_utf8(payload).ok()?.trim();
            Some(MqttEvent::Status { serial, online: body == "online" })
        }
        ("input", Some(port)) => {
            let port: u8 = port.parse().ok()?;
            let body: serde_json::Value = serde_json::from_slice(payload).ok()?;
            Some(MqttEvent::Input {
                serial,
                port,
                value: input_value(&body)?,
                node_ts: body.get("ts").and_then(|v| v.as_u64()),
            })
        }
        ("health", None) => {
            let body: serde_json::Value = serde_json::from_slice(payload).ok()?;
            Some(MqttEvent::Health { serial, health: health(&body) })
        }
        _ => None,
    }
}

/// A contact reports `state`; a sensor reports `value`. Both arrive on the input
/// topic, because from the show's side they are the same thing: a parameter the
/// device writes.
fn input_value(body: &serde_json::Value) -> Option<ParameterValue> {
    if let Some(state) = body.get("state").and_then(|v| v.as_bool()) {
        return Some(ParameterValue::Bool(state));
    }
    if let Some(value) = body.get("value").and_then(|v| v.as_f64()) {
        return Some(ParameterValue::Float(value as f32));
    }
    if let Some(text) = body.get("text").and_then(|v| v.as_str()) {
        return Some(ParameterValue::Text(text.to_string()));
    }
    None
}

fn health(body: &serde_json::Value) -> DeviceHealth {
    DeviceHealth {
        uptime_s: body.get("uptime_s").and_then(|v| v.as_u64()).unwrap_or(0),
        temperature_c: body.get("temp_c").and_then(|v| v.as_f64()).map(|v| v as f32),
        poe_class: body.get("poe_class").and_then(|v| v.as_u64()).map(|v| v as u8),
        errors: body
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        reported_at: Some(chrono::Utc::now()),
    }
}

/// The topic a node listens on for one of its output ports.
pub fn output_topic(serial: &str, port: u8) -> String {
    format!("openhaunt/{serial}/output/{port}/set")
}

/// The topic a node listens on for a shape to trace on one of its output ports.
pub fn effect_topic(serial: &str, port: u8) -> String {
    format!("openhaunt/{serial}/output/{port}/effect")
}

/// Where the console publishes what time it thinks it is.
///
/// One topic for every node rather than one per node, and deliberately not under a
/// serial: the whole point is that every node on this broker agrees with every other
/// about when a cycle started. Retained, so a node that joins between ticks has an
/// answer immediately rather than after up to a second of rendering against nothing.
pub const CLOCK_TOPIC: &str = "openhaunt/clock";

/// What the console publishes on [`CLOCK_TOPIC`].
///
/// `seq` counts up so a node can tell a fresh sample from a retained one replayed
/// after a broker restart, and reset its estimate rather than smoothing towards a
/// number from before the gap.
pub fn clock_payload(now_ms: u64, seq: u64) -> Vec<u8> {
    serde_json::json!({ "t": now_ms, "seq": seq }).to_string().into_bytes()
}

// ── Connection ────────────────────────────────────────────────────────────────

/// A live connection to the broker, feeding parsed events into a channel.
///
/// The broker is normally this process's own, over loopback, but nothing here
/// assumes that: the same code reaches an external broker if one is ever named.
pub struct MqttLink {
    client: AsyncClient,
    stop: tokio::sync::oneshot::Sender<()>,
}

impl MqttLink {
    pub fn connect(broker: &str, client_id: &str, events: mpsc::Sender<MqttEvent>) -> Self {
        let (host, port) = split_addr(broker);
        let mut options = MqttOptions::new(client_id, host, port);
        options.set_keep_alive(Duration::from_secs(15));
        options.set_max_packet_size(16 * 1024, 16 * 1024);

        let (client, mut eventloop) = AsyncClient::new(options, 64);
        let subscriber = client.clone();
        let (stop, mut stopped) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            for topic in SUBSCRIPTIONS {
                if let Err(e) = subscriber.subscribe(*topic, QoS::AtLeastOnce).await {
                    warn!("[devices] cannot subscribe to {topic}: {e}");
                }
            }
            loop {
                let event = tokio::select! {
                    _ = &mut stopped => break,
                    event = eventloop.poll() => event,
                };
                match event {
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if let Some(event) = parse(&publish.topic, &publish.payload) {
                            if events.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // rumqttc reconnects on its own; this is the log for why a
                        // node went quiet, not a reason to give up on the loop.
                        debug!("[devices] mqtt: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });

        MqttLink { client, stop }
    }

    /// Publish to a node. Fire and forget: a device that is not answering must not
    /// hold up the tick that produced the value.
    ///
    /// `retain` is for the clock and nothing else so far. A retained value is what the
    /// broker hands a node the moment it subscribes, which is the difference between a
    /// node that can place a cycle as soon as it connects and one that renders against
    /// a guess until the next tick comes round.
    pub fn publish(&self, topic: String, payload: Vec<u8>, retain: bool) {
        let client = self.client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.publish(&topic, QoS::AtLeastOnce, retain, payload).await {
                debug!("[devices] publish to {topic}: {e}");
            }
        });
    }

    pub fn stop(self) {
        let _ = self.stop.send(());
    }
}

fn split_addr(addr: &str) -> (String, u16) {
    match addr.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(1883)),
        None => (addr.to_string(), 1883),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_message_says_whether_a_node_is_there() {
        assert_eq!(
            parse("openhaunt/1a2b3c/status", b"online"),
            Some(MqttEvent::Status { serial: "1a2b3c".into(), online: true }),
        );
        // The will, published for the node by the broker when it drops off.
        assert_eq!(
            parse("openhaunt/1a2b3c/status", b"offline"),
            Some(MqttEvent::Status { serial: "1a2b3c".into(), online: false }),
        );
    }

    #[test]
    fn a_contact_edge_becomes_a_boolean() {
        let event = parse(
            "openhaunt/1a2b3c/input/3",
            br#"{"state": true, "edge": "rising", "ts": 12345}"#,
        );
        assert_eq!(
            event,
            Some(MqttEvent::Input {
                serial: "1a2b3c".into(),
                port: 3,
                value: ParameterValue::Bool(true),
                node_ts: Some(12345),
            }),
        );
    }

    #[test]
    fn a_sensor_reading_becomes_a_level_on_the_same_topic() {
        let event = parse("openhaunt/env1/input/0", br#"{"value": 21.5, "unit": "C"}"#);
        assert_eq!(
            event,
            Some(MqttEvent::Input {
                serial: "env1".into(),
                port: 0,
                value: ParameterValue::Float(21.5),
                node_ts: None,
            }),
        );
    }

    #[test]
    fn health_fills_in_what_the_node_left_out() {
        let Some(MqttEvent::Health { serial, health }) =
            parse("openhaunt/1a2b3c/health", br#"{"uptime_s": 90, "errors": ["i2c"]}"#)
        else {
            panic!("expected a health event");
        };
        assert_eq!(serial, "1a2b3c");
        assert_eq!(health.uptime_s, 90);
        assert_eq!(health.errors, vec!["i2c"]);
        assert_eq!(health.temperature_c, None);
        assert!(health.reported_at.is_some(), "the console stamps when it heard");
    }

    #[test]
    fn health_reads_every_field_the_node_does_send() {
        let Some(MqttEvent::Health { health, .. }) = parse(
            "openhaunt/1a2b3c/health",
            br#"{"uptime_s": 5, "temp_c": 41.5, "poe_class": 3, "errors": []}"#,
        ) else {
            panic!("expected a health event");
        };
        assert_eq!(health.temperature_c, Some(41.5));
        assert_eq!(health.poe_class, Some(3));
        assert!(health.errors.is_empty());
    }

    #[test]
    fn nonsense_is_dropped_rather_than_guessed_at() {
        assert_eq!(parse("something/else/entirely", b"{}"), None);
        assert_eq!(parse("openhaunt/1a2b3c", b"{}"), None);
        assert_eq!(parse("openhaunt/1a2b3c/input/notaport", b"{}"), None);
        assert_eq!(parse("openhaunt/1a2b3c/input/0", b"not json"), None);
        assert_eq!(
            parse("openhaunt/1a2b3c/input/0", br#"{"nothing": "useful"}"#),
            None,
            "an input with no value in it is not an input",
        );
        assert_eq!(parse("openhaunt/1a2b3c/somethingnew", b"{}"), None);
    }

    #[test]
    fn an_output_topic_names_the_port_the_node_numbers_it_by() {
        assert_eq!(output_topic("1a2b3c", 0), "openhaunt/1a2b3c/output/0/set");
    }

    #[test]
    fn a_broker_address_splits_into_host_and_port() {
        assert_eq!(split_addr("10.0.0.5:1883"), ("10.0.0.5".to_string(), 1883));
        assert_eq!(split_addr("10.0.0.5"), ("10.0.0.5".to_string(), 1883));
        assert_eq!(split_addr("broker.local:7000"), ("broker.local".to_string(), 7000));
    }
}
