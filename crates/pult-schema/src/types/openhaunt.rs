//! What a node says it is, and the fixture type that follows from it.
//!
//! Only the device knows what it is. A node's `GET /api/v1/info` carries a list of
//! ports — each one an access, a data type, a unit and a range, in the vocabulary
//! E1.73 UDR uses — and this module turns that description into a fixture type.
//! There is no table from module id to fixture here, and there must not be one: a
//! node newer than this console, or a module nobody here has heard of, describes
//! itself and works.
//!
//! The ids are derived from the description rather than random, so adopting the
//! same kind of module on two consoles — or on the same console after the showfile
//! was rebuilt — lands on one fixture type instead of a pile of identical ones.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::types::fixture::{
    FixtureType, ParameterBinding, ParameterDefinition, ParameterDirection, ParameterKind,
    ParameterValue,
};

/// Descriptor flag bit 6 means the module switches mains voltage.
pub const MODULE_FLAG_MAINS: u32 = 1 << 6;

/// Namespace for OpenHaunt fixture type ids. Arbitrary, and fixed forever:
/// changing it orphans every fixture patched against the old ids.
const NAMESPACE: Uuid = Uuid::from_u128(0x0e6f_9f31_5f24_4f3a_9f1d_6f6f7e2c8a01);

// ── What a node says about itself ─────────────────────────────────────────────

/// Which way a port flows, in UDR's words.
///
/// `readonly` is the node's to write and the console's to read — an input. The
/// console drives a `readwrite` one, which is an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum PortAccess {
    ReadOnly,
    ReadWrite,
}

/// UDR's data types, plus `color` — the one extension, and documented as such in
/// `OpenHaunt/node`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum PortDataType {
    Boolean,
    Number,
    String,
    Color,
}

/// One terminal, as the node describes it.
///
/// Everything but `port`, `name`, `access` and `dataType` is optional: a node says
/// as much as it usefully can and the console does without the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PortDescription {
    /// The `<n>` in the node's topics and in its `/state`.
    pub port: u8,
    /// Friendly, and shown to the operator as the parameter's name.
    pub name: String,
    pub access: PortAccess,
    pub data_type: PortDataType,
    /// A UDR unit name — `degree-celsius`, `percent`, `unitless` — on numbers.
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    /// Where the port sits before anything drives it. `0` or `1` on a boolean.
    #[serde(default)]
    pub default: Option<f64>,
    /// A hint at what the port means, from a small vocabulary the console
    /// recognises. Absent, or a word this console does not know, is not an error:
    /// the port becomes a named parameter instead.
    #[serde(default)]
    pub class: Option<String>,
}

/// A node that forwards a universe says so, and that is what makes it a gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DmxDescription {
    #[serde(default)]
    pub protocols: Vec<String>,
    #[serde(default)]
    pub universes: u16,
}

/// The self-description a node serves from `GET /api/v1/info`.
///
/// Both keys default, so firmware that predates self-description parses as a node
/// with nothing to say — which is exactly what it is, and what stops it being
/// adopted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeDescription {
    #[serde(default)]
    pub ports: Vec<PortDescription>,
    #[serde(default)]
    pub dmx: Option<DmxDescription>,
}

impl NodeDescription {
    /// Whether the node said anything at all. A node that describes neither ports
    /// nor a universe has told the console nothing it could patch.
    pub fn is_empty(&self) -> bool {
        self.ports.is_empty() && self.dmx.is_none()
    }

    pub fn inputs(&self) -> usize {
        self.ports.iter().filter(|p| p.access == PortAccess::ReadOnly).count()
    }

    pub fn outputs(&self) -> usize {
        self.ports.iter().filter(|p| p.access == PortAccess::ReadWrite).count()
    }
}

// ── What a node says it can render for itself ─────────────────────────────────

/// What one port can be handed instead of a stream of values.
///
/// A node that can trace a shape on its own wants a description, not forty messages a
/// second; one that cannot must never be sent a description it will ignore in silence.
/// Nothing is assumed either way, so a port says so.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortEffectCapability {
    pub port: u8,
    /// Shape names in the wire's spelling: `sine`, `triangle`, `square`, `saw-up`,
    /// `saw-down`. Kept as strings rather than an enum because a node may name a shape
    /// this console has never heard of, and the answer to that is to not use it.
    pub shapes: Vec<String>,
    pub steps: bool,
    /// Whether a `set` carrying `fade_ms` is understood, so a fade can be handed over
    /// whole instead of interpolated here.
    pub transitions: bool,
}

impl PortEffectCapability {
    /// Whether this port can be trusted with a particular shape.
    pub fn has_shape(&self, shape: &str) -> bool {
        self.shapes.iter().any(|s| s == shape)
    }
}

/// Every port on one node that said anything about effects.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectCapability {
    pub ports: Vec<PortEffectCapability>,
}

impl EffectCapability {
    pub fn port(&self, port: u8) -> Option<&PortEffectCapability> {
        self.ports.iter().find(|p| p.port == port)
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }
}

/// Read the per-port `effects` blocks out of a raw `/info` body.
///
/// Deliberately not a field on [`PortDescription`]. [`fixture_type_id`] hashes the
/// serialised [`NodeDescription`], so a field there would give every adopted node a
/// new fixture type the moment firmware started advertising effects, orphaning every
/// parameter already patched against the old id. Serde ignores the unknown key, the
/// hash stays put, and the capability is read from the raw JSON alongside — the same
/// arrangement the mains flag already uses.
pub fn effect_capability_from(info: &serde_json::Value) -> Option<EffectCapability> {
    let ports: Vec<PortEffectCapability> = info["ports"]
        .as_array()?
        .iter()
        .filter_map(|port| {
            let effects = port.get("effects")?.as_object()?;
            Some(PortEffectCapability {
                port: port.get("port")?.as_u64()? as u8,
                shapes: effects
                    .get("shapes")
                    .and_then(|s| s.as_array())
                    .map(|list| {
                        list.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()
                    })
                    .unwrap_or_default(),
                steps: effects.get("steps").and_then(|v| v.as_bool()).unwrap_or(false),
                transitions: effects
                    .get("transitions")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect();
    if ports.is_empty() { None } else { Some(EffectCapability { ports }) }
}

// ── The fixture type that follows ─────────────────────────────────────────────

/// The stable id of the fixture type a description becomes.
///
/// Derived from the module type and the description itself, so two identical
/// modules share one type and firmware that changes its ports gets a fresh one
/// rather than silently mismatching the parameters already patched against it.
/// `new_v5` hashes the name it is given, so the description goes in whole.
pub fn fixture_type_id(module_type: u16, description: &NodeDescription) -> Uuid {
    let mut name = module_type.to_be_bytes().to_vec();
    name.extend_from_slice(&serde_json::to_vec(description).unwrap_or_default());
    Uuid::new_v5(&NAMESPACE, &name)
}

/// The fixture type for a node that has described itself.
///
/// Each port becomes one parameter, bound to the port the node numbered it. The
/// `class` hint decides which parameter kind it is where the console has semantics
/// for one — a temperature reading has to be a temperature for the stage view to
/// draw it — and everything else becomes a parameter named after what the node
/// called it.
pub fn fixture_type_from(
    module_type: u16,
    module_name: &str,
    description: &NodeDescription,
) -> FixtureType {
    // A name that appears twice would give two parameters one `live_values` key.
    // Only named parameters are at risk: the classed kinds either carry their port
    // or are singular on a module.
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for port in &description.ports {
        *seen.entry(port.name.as_str()).or_default() += 1;
    }

    let parameters = description
        .ports
        .iter()
        .map(|port| ParameterDefinition {
            kind: kind_for(port, seen.get(port.name.as_str()).copied().unwrap_or(1) > 1),
            direction: match port.access {
                PortAccess::ReadOnly => ParameterDirection::Input,
                PortAccess::ReadWrite => ParameterDirection::Output,
            },
            binding: ParameterBinding::Port { index: port.port },
            default_value: default_value(port),
        })
        .collect();

    FixtureType {
        id: fixture_type_id(module_type, description),
        name: module_name.to_string(),
        manufacturer: "OpenHaunt".to_string(),
        // A gateway occupies a whole universe; a node with ports occupies none.
        channel_count: if description.dmx.is_some() { 512 } else { 0 },
        parameters,
    }
}

/// The parameter kind a port's `class` asks for, or a named one.
fn kind_for(port: &PortDescription, name_is_shared: bool) -> ParameterKind {
    match port.class.as_deref() {
        Some("contact") => ParameterKind::Contact(port.port),
        Some("switch") => ParameterKind::Switch(port.port),
        Some("temperature") => ParameterKind::Temperature,
        Some("humidity") => ParameterKind::Humidity,
        Some("air-quality") => ParameterKind::AirQuality,
        Some("intensity") => ParameterKind::Intensity,
        Some("color") => ParameterKind::ColorRgb,
        Some("text") => ParameterKind::Text,
        _ if name_is_shared => ParameterKind::Named(format!("{} {}", port.name, port.port)),
        _ => ParameterKind::Named(port.name.clone()),
    }
}

/// Where a port sits before anything has driven it, from its type and `default`.
fn default_value(port: &PortDescription) -> ParameterValue {
    match port.data_type {
        PortDataType::Boolean => ParameterValue::Bool(port.default.unwrap_or(0.0) != 0.0),
        PortDataType::Number => ParameterValue::Float(port.default.unwrap_or(0.0) as f32),
        PortDataType::String => ParameterValue::Text(String::new()),
        PortDataType::Color => ParameterValue::Color { r: 0.0, g: 0.0, b: 0.0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descriptions the catalogue modules serve, as `OpenHaunt/node`'s docs
    /// write them. Here as a *device's* words, to have something to parse — the
    /// console has no such table and adoption never consults one.
    fn described(module_type: u16) -> NodeDescription {
        let json = match module_type {
            // 0x0001 DMX gateway: no ports of its own, a universe to forward.
            0x0001 => serde_json::json!({
                "ports": [],
                "dmx": { "protocols": ["sacn"], "universes": 1 },
            }),
            // 0x0002 eight opto-isolated inputs.
            0x0002 => serde_json::json!({
                "ports": (0..8).map(|n| serde_json::json!({
                    "port": n,
                    "name": format!("Input {}", n + 1),
                    "access": "readonly",
                    "dataType": "boolean",
                    "class": "contact",
                })).collect::<Vec<_>>(),
            }),
            // 0x0003 WS2812 strip.
            0x0003 => serde_json::json!({ "ports": [
                { "port": 0, "name": "Strip colour", "access": "readwrite",
                  "dataType": "color", "class": "color" },
                { "port": 1, "name": "Brightness", "access": "readwrite",
                  "dataType": "number", "unit": "percent",
                  "minimum": 0, "maximum": 1, "default": 0, "class": "intensity" },
            ]}),
            // 0x0004 mains relay.
            0x0004 => serde_json::json!({ "ports": [
                { "port": 0, "name": "Relay", "access": "readwrite",
                  "dataType": "boolean", "default": 0, "class": "switch" },
            ]}),
            // 0x0005 OLED.
            0x0005 => serde_json::json!({ "ports": [
                { "port": 0, "name": "Line", "access": "readwrite",
                  "dataType": "string", "class": "text" },
            ]}),
            // 0x0006 four dry contacts the console closes.
            0x0006 => serde_json::json!({ "ports": (0..4).map(|n| serde_json::json!({
                "port": n,
                "name": format!("Contact {}", n + 1),
                "access": "readwrite",
                "dataType": "boolean",
                "class": "switch",
            })).collect::<Vec<_>>() }),
            // 0x0007 environment sensor.
            0x0007 => serde_json::json!({ "ports": [
                { "port": 0, "name": "Temperature", "access": "readonly",
                  "dataType": "number", "unit": "degree-celsius",
                  "minimum": -40, "maximum": 85, "class": "temperature" },
                { "port": 1, "name": "Humidity", "access": "readonly",
                  "dataType": "number", "unit": "percent",
                  "minimum": 0, "maximum": 100, "class": "humidity" },
                { "port": 2, "name": "Air quality", "access": "readonly",
                  "dataType": "number", "unit": "parts-per-million",
                  "minimum": 0, "maximum": 5000, "class": "air-quality" },
            ]}),
            other => panic!("no catalogue description for {other:#06x}"),
        };
        serde_json::from_value(json).expect("a description the docs write parses")
    }

    #[test]
    fn every_catalogue_module_describes_itself_into_a_fixture_type() {
        for module_type in [0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007] {
            let description = described(module_type);
            let ft = fixture_type_from(module_type, "Module", &description);
            assert_eq!(ft.parameters.len(), description.ports.len());
            for (parameter, port) in ft.parameters.iter().zip(&description.ports) {
                assert_eq!(parameter.binding, ParameterBinding::Port { index: port.port });
            }
        }
    }

    #[test]
    fn what_a_node_reports_is_read_and_what_it_drives_is_written() {
        let inputs = fixture_type_from(0x0002, "Digital Inputs", &described(0x0002));
        assert_eq!(inputs.parameters.len(), 8);
        assert!(inputs.parameters.iter().all(|p| p.direction == ParameterDirection::Input));
        assert_eq!(inputs.parameters[3].kind, ParameterKind::Contact(3));

        let sensor = fixture_type_from(0x0007, "Environment Sensor", &described(0x0007));
        assert!(sensor.parameters.iter().all(|p| p.direction == ParameterDirection::Input));
        assert_eq!(sensor.parameters[0].kind, ParameterKind::Temperature);

        let relay = fixture_type_from(0x0004, "Mains Relay", &described(0x0004));
        assert!(relay.parameters.iter().all(|p| p.direction == ParameterDirection::Output));
        assert_eq!(relay.parameters[0].kind, ParameterKind::Switch(0));
    }

    #[test]
    fn only_a_node_that_forwards_a_universe_occupies_channels() {
        assert_eq!(fixture_type_from(0x0001, "DMX Gateway", &described(0x0001)).channel_count, 512);
        assert_eq!(fixture_type_from(0x0004, "Mains Relay", &described(0x0004)).channel_count, 0);
    }

    #[test]
    fn a_port_this_console_has_no_word_for_becomes_a_named_parameter() {
        let description: NodeDescription = serde_json::from_value(serde_json::json!({
            "ports": [
                { "port": 0, "name": "Fog output", "access": "readwrite",
                  "dataType": "number", "unit": "percent", "class": "fog-density" },
                { "port": 1, "name": "Tank level", "access": "readonly",
                  "dataType": "number", "unit": "percent" },
            ],
        }))
        .unwrap();

        let ft = fixture_type_from(0x00ff, "Fogger", &description);
        assert_eq!(ft.parameters[0].kind, ParameterKind::Named("Fog output".into()));
        assert_eq!(ft.parameters[0].direction, ParameterDirection::Output);
        assert_eq!(ft.parameters[1].kind, ParameterKind::Named("Tank level".into()));
        assert_eq!(ft.parameters[1].direction, ParameterDirection::Input);
    }

    #[test]
    fn two_ports_with_one_name_do_not_end_up_sharing_a_key() {
        let description: NodeDescription = serde_json::from_value(serde_json::json!({
            "ports": [
                { "port": 0, "name": "Valve", "access": "readwrite", "dataType": "boolean" },
                { "port": 1, "name": "Valve", "access": "readwrite", "dataType": "boolean" },
            ],
        }))
        .unwrap();

        let ft = fixture_type_from(0x00ff, "Manifold", &description);
        assert_eq!(ft.parameters[0].kind, ParameterKind::Named("Valve 0".into()));
        assert_eq!(ft.parameters[1].kind, ParameterKind::Named("Valve 1".into()));
    }

    #[test]
    fn a_default_follows_the_ports_data_type() {
        let description: NodeDescription = serde_json::from_value(serde_json::json!({
            "ports": [
                { "port": 0, "name": "Latch", "access": "readwrite",
                  "dataType": "boolean", "default": 1 },
                { "port": 1, "name": "Level", "access": "readwrite",
                  "dataType": "number", "default": 0.25 },
                { "port": 2, "name": "Caption", "access": "readwrite", "dataType": "string" },
                { "port": 3, "name": "Tint", "access": "readwrite", "dataType": "color" },
            ],
        }))
        .unwrap();

        let ft = fixture_type_from(0x00ff, "Oddity", &description);
        assert_eq!(ft.parameters[0].default_value, ParameterValue::Bool(true));
        assert_eq!(ft.parameters[1].default_value, ParameterValue::Float(0.25));
        assert_eq!(ft.parameters[2].default_value, ParameterValue::Text(String::new()));
        assert_eq!(
            ft.parameters[3].default_value,
            ParameterValue::Color { r: 0.0, g: 0.0, b: 0.0 },
        );
    }

    #[test]
    fn the_ids_are_stable_and_distinct() {
        // Adopting the same module on two consoles has to land on one fixture type,
        // or the same rig ends up with a different id on every node.
        let relay = described(0x0004);
        assert_eq!(
            fixture_type_from(0x0004, "Mains Relay", &relay).id,
            fixture_type_id(0x0004, &relay),
        );
        assert_eq!(fixture_type_id(0x0004, &relay), fixture_type_id(0x0004, &described(0x0004)));

        let ids: std::collections::BTreeSet<Uuid> =
            [0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007]
                .into_iter()
                .map(|t| fixture_type_id(t, &described(t)))
                .collect();
        assert_eq!(ids.len(), 7, "two modules must not share an id");
    }

    #[test]
    fn firmware_that_changes_its_ports_gets_a_new_type_rather_than_a_mismatched_one() {
        let before = described(0x0004);
        let mut after = before.clone();
        after.ports.push(PortDescription {
            port: 1,
            name: "Second relay".into(),
            access: PortAccess::ReadWrite,
            data_type: PortDataType::Boolean,
            unit: None,
            minimum: None,
            maximum: None,
            default: None,
            class: Some("switch".into()),
        });

        assert_ne!(fixture_type_id(0x0004, &before), fixture_type_id(0x0004, &after));
    }

    #[test]
    fn firmware_that_says_nothing_describes_nothing() {
        let silent: NodeDescription = serde_json::from_value(serde_json::json!({
            "v": "1", "fw": "0.1.0", "module": { "type": "0x0004", "flags": 64 },
        }))
        .unwrap();

        assert!(silent.is_empty(), "an old node has no ports and no universe to offer");
    }

    #[test]
    fn a_description_counts_its_own_ports_for_the_panel() {
        let inputs = described(0x0002);
        assert_eq!(inputs.inputs(), 8);
        assert_eq!(inputs.outputs(), 0);

        let strip = described(0x0003);
        assert_eq!(strip.inputs(), 0);
        assert_eq!(strip.outputs(), 2);
    }

    #[test]
    fn a_port_that_says_nothing_about_effects_offers_none() {
        let info = serde_json::json!({
            "ports": [
                { "port": 0, "name": "Relay", "access": "readwrite", "dataType": "boolean" },
            ],
        });
        assert!(effect_capability_from(&info).is_none());
    }

    #[test]
    fn a_port_that_advertises_effects_is_read_out_of_the_raw_info() {
        let info = serde_json::json!({
            "ports": [
                { "port": 0, "name": "Strip colour", "access": "readwrite", "dataType": "color",
                  "effects": { "shapes": ["sine", "steps-are-not-a-shape"], "steps": true } },
                { "port": 1, "name": "Brightness", "access": "readwrite", "dataType": "number",
                  "effects": { "shapes": ["sine", "square"], "steps": false, "transitions": true } },
                { "port": 2, "name": "Contact", "access": "readonly", "dataType": "boolean" },
            ],
        });

        let caps = effect_capability_from(&info).expect("two ports advertise");
        assert_eq!(caps.ports.len(), 2, "the silent port contributes nothing");

        let colour = caps.port(0).unwrap();
        assert!(colour.has_shape("sine"));
        assert!(colour.steps);
        assert!(!colour.transitions, "absent means no");

        let brightness = caps.port(1).unwrap();
        assert!(brightness.has_shape("square"));
        assert!(!brightness.has_shape("triangle"));
        assert!(brightness.transitions);

        assert!(caps.port(2).is_none());
    }

    /// The trap this whole arrangement exists to avoid.
    ///
    /// `fixture_type_id` hashes the serialised description, so if `effects` were a
    /// field on `PortDescription` then firmware that started advertising it would give
    /// every adopted node a new fixture type and orphan every patched parameter.
    /// Reading it from the raw JSON instead keeps the id where it was.
    #[test]
    fn firmware_that_starts_advertising_effects_does_not_re_type_the_node() {
        let plain = serde_json::json!({
            "ports": [
                { "port": 0, "name": "Brightness", "access": "readwrite",
                  "dataType": "number", "class": "intensity" },
            ],
        });
        let mut advertising = plain.clone();
        advertising["ports"][0]["effects"] =
            serde_json::json!({ "shapes": ["sine"], "steps": true, "transitions": true });

        let before: NodeDescription = serde_json::from_value(plain).unwrap();
        let after: NodeDescription = serde_json::from_value(advertising.clone()).unwrap();

        assert_eq!(
            fixture_type_id(0x0003, &before),
            fixture_type_id(0x0003, &after),
            "the unknown key must not reach the hash",
        );
        assert!(effect_capability_from(&advertising).is_some(), "and must still be readable");
    }
}
