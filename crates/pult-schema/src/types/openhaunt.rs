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
    FixturePhysical, FixtureType, FixtureTypeSource, ParameterDefinition, ParameterDirection,
    ParameterKind, ParameterValue, PhysicalRange, PhysicalUnit, Slot,
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
    /// recognizes. Absent, or a word this console does not know, is not an error:
    /// the port becomes a named parameter instead.
    #[serde(default)]
    pub class: Option<String>,
    /// Which of the node's heads this port belongs to. Absent is head 0, the node
    /// itself, which is every port a node described before heads existed.
    ///
    /// Left out of the hash when absent, and that is load-bearing: [`fixture_type_id`]
    /// hashes the description, and a `"head": null` on every port of a node that never
    /// heard of heads would give it a new fixture type and orphan its patch.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub head: Option<u8>,
}

impl PortDescription {
    /// The head this port belongs to, absent meaning the node's own.
    pub fn head(&self) -> u8 {
        self.head.unwrap_or(0)
    }
}

/// One head other than the node itself: a fixture of its own on the same node. See
/// `OpenHaunt/node/docs/14-pixels.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HeadDescription {
    pub head: u8,
    pub name: String,
    /// A hint, like a port's `class`. `pixel-strip` is the one this console knows.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub kind: Option<String>,
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
    /// The heads other than head 0, where the node has any. Left out of the hash when
    /// empty, for the reason a port's `head` is.
    #[serde(default)]
    pub heads: Vec<HeadDescription>,
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

    /// Every head this node has: 0, then each one it lists or names on a port.
    pub fn head_numbers(&self) -> Vec<u8> {
        let mut heads: std::collections::BTreeSet<u8> = [0].into();
        heads.extend(self.heads.iter().map(|h| h.head));
        heads.extend(self.ports.iter().map(PortDescription::head));
        heads.into_iter().collect()
    }

    /// What the node said about one head, and nothing about the others.
    ///
    /// For head 0 of a node that has no heads, this is the description itself, field for
    /// field, so its fixture type keeps the id it has always had. A universe is the
    /// node's, so only head 0 carries `dmx`.
    pub fn for_head(&self, head: u8) -> NodeDescription {
        NodeDescription {
            ports: self.ports.iter().filter(|p| p.head() == head).cloned().collect(),
            dmx: if head == 0 { self.dmx.clone() } else { None },
            heads: self.heads.iter().filter(|h| h.head == head && head != 0).cloned().collect(),
        }
    }

    /// What the node calls a head. Head 0 is the node, and named by the caller.
    pub fn head_name(&self, head: u8) -> Option<&str> {
        self.heads.iter().find(|h| h.head == head).map(|h| h.name.as_str())
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
/// serialized [`NodeDescription`], so a field there would give every adopted node a
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
    name.extend_from_slice(&hashed(description));
    Uuid::new_v5(&NAMESPACE, &name)
}

/// The bytes a description is hashed as: its JSON as it was before heads existed, with a
/// head written only where there is one, so a node that predates heads hashes exactly as
/// it always did.
///
/// A mirror rather than the type itself, because the schema's own serialization writes
/// every absent head as `null` (which the browser wants), and a `Value` would sort the
/// keys and change every byte.
fn hashed(description: &NodeDescription) -> Vec<u8> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Port<'a> {
        port: u8,
        name: &'a str,
        access: PortAccess,
        data_type: PortDataType,
        unit: &'a Option<String>,
        minimum: Option<f64>,
        maximum: Option<f64>,
        default: Option<f64>,
        class: &'a Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        head: Option<u8>,
    }
    #[derive(Serialize)]
    struct Description<'a> {
        ports: Vec<Port<'a>>,
        dmx: &'a Option<DmxDescription>,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        heads: &'a [HeadDescription],
    }
    let mirror = Description {
        ports: description
            .ports
            .iter()
            .map(|p| Port {
                port: p.port,
                name: &p.name,
                access: p.access,
                data_type: p.data_type,
                unit: &p.unit,
                minimum: p.minimum,
                maximum: p.maximum,
                default: p.default,
                class: &p.class,
                head: p.head,
            })
            .collect(),
        dmx: &description.dmx,
        heads: &description.heads,
    };
    serde_json::to_vec(&mirror).unwrap_or_default()
}

/// The stable id of one head's fixture type.
///
/// Head 0 hashes exactly what [`fixture_type_id`] always hashed for a node without
/// heads, so nothing adopted before heads existed changes type. Every other head
/// carries its number into the hash, because strip 1 and strip 2 in the same mode
/// describe identical ports and are still two fixtures on two outputs.
pub fn head_type_id(module_type: u16, description: &NodeDescription, head: u8) -> Uuid {
    let own = description.for_head(head);
    if head == 0 {
        return fixture_type_id(module_type, &own);
    }
    let mut name = module_type.to_be_bytes().to_vec();
    name.extend_from_slice(b"head");
    name.push(head);
    name.extend_from_slice(&hashed(&own));
    Uuid::new_v5(&NAMESPACE, &name)
}

/// The id of the fixture one head of one node becomes.
///
/// Derived, so two consoles adopting the same node make the same rows and a head
/// that disappears and comes back finds its fixture again. Head 0 is not derived:
/// a node's own fixture was always a fresh id, and what it is found by is its
/// address.
pub fn head_fixture_id(serial: &str, head: u8) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("fixture/{serial}/head/{head}").as_bytes())
}

/// The fixture type for one head of a node.
///
/// Built from that head's ports alone, which is what stops two strips colliding on one
/// color. `effects` is the node's `pixels.effects`, the vocabulary of a
/// `pixel-effect` port, kept as that parameter's slots.
pub fn head_type_from(
    module_type: u16,
    module_name: &str,
    description: &NodeDescription,
    head: u8,
    effects: &[String],
) -> FixtureType {
    let own = description.for_head(head);
    let name = match head {
        0 => module_name.to_string(),
        n => description.head_name(n).map(str::to_string).unwrap_or_else(|| format!("Head {n}")),
    };
    let mut fixture_type = fixture_type_from(module_type, &name, &own);
    fixture_type.id = head_type_id(module_type, description, head);
    for (parameter, port) in fixture_type.parameters.iter_mut().zip(&own.ports) {
        if port.class.as_deref() == Some("pixel-effect") {
            parameter.slots =
                effects.iter().map(|e| Slot { name: e.clone(), ..Slot::default() }).collect();
            if parameter.default_value == ParameterValue::Text(String::new()) {
                parameter.default_value =
                    ParameterValue::Text(effects.first().cloned().unwrap_or_else(|| "solid".into()));
            }
        }
    }
    fixture_type
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
    // A name that appears twice would give two parameters one parameter key.
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
            direction: match port.access {
                PortAccess::ReadOnly => ParameterDirection::Input,
                PortAccess::ReadWrite => ParameterDirection::Output,
            },
            physical: physical_range(port),
            ..ParameterDefinition::on_port(
                kind_for(port, seen.get(port.name.as_str()).copied().unwrap_or(1) > 1),
                port.port,
                default_value(port),
            )
        })
        .collect();

    FixtureType {
        id: fixture_type_id(module_type, description),
        name: module_name.to_string(),
        manufacturer: "OpenHaunt".to_string(),
        long_name: module_name.to_string(),
        // A gateway occupies a whole universe; a node with ports occupies none.
        channel_count: if description.dmx.is_some() { 512 } else { 0 },
        parameters,
        // A node's ports are not a DMX layout: what reaches them is a port write, so
        // there is no mode to be in.
        dmx_modes: Vec::new(),
        physical: FixturePhysical::default(),
        geometry: Vec::new(),
        // The one source the console rebuilds without being asked: this whole function
        // runs again whenever the node describes itself again.
        source: FixtureTypeSource::Node,
        ..FixtureType::default()
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

/// What a driven number port's 0..1 spans, where the node gave it a range that is not
/// already 0..1.
///
/// A fader is 0..1 everywhere in this console, so a strip's Speed of 0–10 Hz, or its
/// Segments of 1–16, was a control that could not get past the bottom of its own range:
/// full was 1 Hz. The connector sends `from + (to − from) · v` instead, and the panel
/// reads it in the port's own unit. Only on ports the console *drives*: a sensor's
/// reading arrives in its own units and is kept as it came.
pub fn physical_range(port: &PortDescription) -> Option<PhysicalRange> {
    if port.access != PortAccess::ReadWrite || port.data_type != PortDataType::Number {
        return None;
    }
    let (from, to) = (port.minimum?, port.maximum?);
    if (from == 0.0 && to == 1.0) || to <= from {
        return None;
    }
    let unit = match port.unit.as_deref() {
        Some("hertz") => PhysicalUnit::Hertz,
        Some("percent") => PhysicalUnit::Percent,
        Some("degree") | Some("degrees") => PhysicalUnit::Degrees,
        Some("second") | Some("seconds") => PhysicalUnit::Seconds,
        _ => PhysicalUnit::None,
    };
    Some(PhysicalRange { from: from as f32, to: to as f32, unit })
}

/// Where a port sits before anything has driven it, from its type and `default`.
fn default_value(port: &PortDescription) -> ParameterValue {
    match port.data_type {
        PortDataType::Boolean => ParameterValue::Bool(port.default.unwrap_or(0.0) != 0.0),
        PortDataType::Number => match physical_range(port) {
            // Kept in the console's 0..1, so a range of 1–16 rests at 1 and not at 100%.
            Some(range) => ParameterValue::Float(range.to_normalized(
                port.default.unwrap_or(range.from as f64) as f32,
            )),
            None => ParameterValue::Float(port.default.unwrap_or(0.0) as f32),
        },
        PortDataType::String => ParameterValue::Text(String::new()),
        PortDataType::Color => ParameterValue::rgb(0.0, 0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descriptions the catalog modules serve, as `OpenHaunt/node`'s docs
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
                { "port": 0, "name": "Strip color", "access": "readwrite",
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
            other => panic!("no catalog description for {other:#06x}"),
        };
        serde_json::from_value(json).expect("a description the docs write parses")
    }

    #[test]
    fn every_catalog_module_describes_itself_into_a_fixture_type() {
        for module_type in [0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007] {
            let description = described(module_type);
            let ft = fixture_type_from(module_type, "Module", &description);
            assert_eq!(ft.parameters.len(), description.ports.len());
            for (parameter, port) in ft.parameters.iter().zip(&description.ports) {
                assert_eq!(parameter.binding.and_then(|b| b.port()), Some(port.port));
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
            ParameterValue::rgb(0.0, 0.0, 0.0),
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
            head: None,
        });

        assert_ne!(fixture_type_id(0x0004, &before), fixture_type_id(0x0004, &after));
    }

    /// The bytes every node adopted before heads existed was typed by. If this changes,
    /// every one of them is re-typed the next time it describes itself.
    #[test]
    fn a_node_without_heads_hashes_as_it_always_did() {
        let relay = described(0x0004);
        let mut legacy = 0x0004u16.to_be_bytes().to_vec();
        legacy.extend_from_slice(
            br#"{"ports":[{"port":0,"name":"Relay","access":"readwrite","dataType":"boolean","unit":null,"minimum":null,"maximum":null,"default":0.0,"class":"switch"}],"dmx":null}"#,
        );
        assert_eq!(fixture_type_id(0x0004, &relay), Uuid::new_v5(&NAMESPACE, &legacy));
        assert_eq!(head_type_id(0x0004, &relay, 0), fixture_type_id(0x0004, &relay));
    }

    #[test]
    fn two_strips_on_one_node_are_two_fixtures_each_with_its_own_color() {
        let info = serde_json::json!({
            "ports": [
                { "port": 0, "name": "Relay 1", "access": "readwrite", "dataType": "boolean", "class": "switch" },
                { "port": 6, "name": "Color", "access": "readwrite", "dataType": "color", "class": "color", "head": 1 },
                { "port": 9, "name": "Effect", "access": "readwrite", "dataType": "string", "class": "pixel-effect", "head": 1 },
                { "port": 15, "name": "Color", "access": "readwrite", "dataType": "color", "class": "color", "head": 2 },
            ],
            "heads": [
                { "head": 1, "name": "Strip 1", "kind": "pixel-strip" },
                { "head": 2, "name": "Strip 2", "kind": "pixel-strip" },
            ],
        });
        let description: NodeDescription = serde_json::from_value(info).unwrap();
        assert_eq!(description.head_numbers(), vec![0, 1, 2]);
        let effects = vec!["solid".to_string(), "chase".to_string()];
        let node = head_type_from(0x0f01, "Node V0", &description, 0, &effects);
        let one = head_type_from(0x0f01, "Node V0", &description, 1, &effects);
        let two = head_type_from(0x0f01, "Node V0", &description, 2, &effects);
        assert_eq!(node.parameters.len(), 1, "the node keeps only its own ports");
        assert_eq!(one.name, "Strip 1");
        assert_eq!(one.parameters[0].kind, ParameterKind::ColorRgb);
        assert_eq!(two.parameters[0].kind, ParameterKind::ColorRgb);
        assert_ne!(one.id, two.id, "two strips in one mode are still two types");
        assert_eq!(one.parameters[1].slots.len(), 2, "the effect port knows its words");
        assert_eq!(one.parameters[1].default_value, ParameterValue::Text("solid".into()));
        assert_ne!(head_fixture_id("a", 1), head_fixture_id("a", 2));
    }

    #[test]
    fn a_driven_port_with_its_own_range_is_a_fader_across_that_range() {
        let description: NodeDescription = serde_json::from_value(serde_json::json!({
            "ports": [
                { "port": 10, "name": "Speed", "access": "readwrite", "dataType": "number",
                  "unit": "hertz", "minimum": 0, "maximum": 10, "default": 1 },
                { "port": 14, "name": "Segments", "access": "readwrite", "dataType": "number",
                  "minimum": 1, "maximum": 16, "default": 1 },
                { "port": 7, "name": "Brightness", "access": "readwrite", "dataType": "number",
                  "minimum": 0, "maximum": 1, "default": 0 },
                { "port": 0, "name": "Temperature", "access": "readonly", "dataType": "number",
                  "minimum": -40, "maximum": 85 },
            ],
        }))
        .unwrap();
        let ft = fixture_type_from(0x0f01, "Strip", &description);
        let speed = ft.parameters[0].physical.expect("0–10 Hz is not 0..1");
        assert_eq!((speed.from, speed.to, speed.unit), (0.0, 10.0, PhysicalUnit::Hertz));
        assert_eq!(ft.parameters[0].default_value, ParameterValue::Float(0.1), "1 Hz of 10");
        assert_eq!(ft.parameters[1].default_value, ParameterValue::Float(0.0), "1 is the bottom of 1–16");
        assert!(ft.parameters[2].physical.is_none(), "0..1 is already the console's");
        assert!(ft.parameters[3].physical.is_none(), "a reading is kept in its own units");
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
                { "port": 0, "name": "Strip color", "access": "readwrite", "dataType": "color",
                  "effects": { "shapes": ["sine", "steps-are-not-a-shape"], "steps": true } },
                { "port": 1, "name": "Brightness", "access": "readwrite", "dataType": "number",
                  "effects": { "shapes": ["sine", "square"], "steps": false, "transitions": true } },
                { "port": 2, "name": "Contact", "access": "readonly", "dataType": "boolean" },
            ],
        });

        let caps = effect_capability_from(&info).expect("two ports advertise");
        assert_eq!(caps.ports.len(), 2, "the silent port contributes nothing");

        let color = caps.port(0).unwrap();
        assert!(color.has_shape("sine"));
        assert!(color.steps);
        assert!(!color.transitions, "absent means no");

        let brightness = caps.port(1).unwrap();
        assert!(brightness.has_shape("square"));
        assert!(!brightness.has_shape("triangle"));
        assert!(brightness.transitions);

        assert!(caps.port(2).is_none());
    }

    /// The trap this whole arrangement exists to avoid.
    ///
    /// `fixture_type_id` hashes the serialized description, so if `effects` were a
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
