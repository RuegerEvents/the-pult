//! Which index is which control, read out of MA's own file.
//!
//! The wire carries numbers; `hardware_configurations.xml`, which ships inside every
//! grandMA3 installation, says what they are. That file is MA's, so this parses it
//! where it lies rather than converting it into something to carry around — a derived
//! copy is still their table, and one fewer build step is one fewer thing to be stale.
//!
//! **An index is a position, not an annotation.** The file numbers its entries in XML
//! comments (`<!-- 002 -->`), and those numbers are exactly the element's position in
//! its section — checked across all 449 hardkeys. So this counts elements and never
//! reads a comment, which is the difference between parsing a document and parsing a
//! convention somebody might stop following.
//!
//! Every block size in the protocol matches its table exactly: 449 hardkey slots
//! against 448 bits, 12 faders against 12 words, 77 encoder slots against 77 bytes,
//! and an LED table whose highest byte offset is 252 against a 253-byte frame. That
//! agreement is the evidence that these indices *are* the wire format rather than an
//! internal detail of the console.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What this wing is called in MA's file.
pub const COMMAND_WING: &str = "grandMA3 onPC command wing";

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("reading hardware_configurations.xml: {0}")]
    Xml(String),
    #[error("no module named {0:?} in this file; it describes: {1}")]
    NoSuchModule(String, String),
}

/// One control, as MA's table describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    /// A readable name: `STORE`, `EXEC exec 201`, `Inside1`.
    pub label: String,
    /// MA's own `Code`, where it has one.
    #[serde(rename = "Code", skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// MA's `Type`, e.g. `Inside1` / `Outside3` for a dual-concentric encoder.
    #[serde(rename = "Type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(rename = "Key", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(rename = "ExecutorIndex", skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
    #[serde(rename = "SpecialExecutor", skip_serializing_if = "Option::is_none")]
    pub special: Option<String>,
}

impl Control {
    /// Is this a fader's touch contact rather than a key somebody presses?
    ///
    /// MA models touching a fader as a hardkey, which is how a surface gets grab and
    /// release without inferring them from motion.
    pub fn is_fader_touch(&self) -> bool {
        self.code.as_deref() == Some("FADER")
    }

    fn named(&self) -> bool {
        !self.label.is_empty()
    }

    fn build_label(&mut self) {
        let mut bits = Vec::new();
        for s in [&self.code, &self.kind, &self.key] {
            if let Some(v) = s.as_deref() {
                bits.push(v.to_string());
                break;
            }
        }
        if let Some(e) = &self.executor {
            bits.push(format!("exec {e}"));
        }
        if let Some(s) = &self.special {
            bits.push(s.clone());
        }
        self.label = bits.join(" ");
    }
}

/// One LED, and where its channels are in the frame.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Led {
    pub label: String,
    #[serde(rename = "Code", skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(rename = "R", skip_serializing_if = "Option::is_none")]
    pub r: Option<usize>,
    #[serde(rename = "G", skip_serializing_if = "Option::is_none")]
    pub g: Option<usize>,
    #[serde(rename = "B", skip_serializing_if = "Option::is_none")]
    pub b: Option<usize>,
    #[serde(rename = "ExecutorIndex", skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
}

impl Led {
    pub fn is_rgb(&self) -> bool {
        self.g.is_some() && self.b.is_some()
    }
}

/// One module's controls, keyed by the index the wire uses.
///
/// The key is a number and not a string on purpose: a `BTreeMap<String, _>` would
/// order them `"190", "283", "78"`, so a panel walking the map would draw the keys in
/// an order nobody could explain.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WingMap {
    pub module: String,
    pub hardkeys: BTreeMap<u16, Control>,
    pub faders: BTreeMap<u8, Control>,
    pub encoders: BTreeMap<u8, Control>,
    pub leds: Vec<Led>,
}

impl WingMap {
    /// Every module `hardware_configurations.xml` describes, in file order.
    pub fn modules(xml: &str) -> Vec<String> {
        let mut reader = Reader::from_str(xml);
        let mut out = Vec::new();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e))
                    if e.name().as_ref() == "HardwareConfiguration" =>
                {
                    if let Some(name) = attr(&e, "Name") {
                        out.push(name);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        out
    }

    /// Read one module out of MA's file.
    pub fn from_xml(xml: &str, module: &str) -> Result<WingMap, MapError> {
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut map = WingMap { module: module.to_string(), ..Default::default() };

        let mut inside = false;
        // The index of a control is where it sits in its list, so each list counts.
        let (mut hk, mut fd, mut ed) = (0u32, 0u32, 0u32);

        loop {
            let event = reader.read_event_into(&mut buf);
            match event {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    match e.name().as_ref() {
                        "HardwareConfiguration" => {
                            inside = attr(e, "Name").as_deref() == Some(module);
                            if inside {
                                (hk, fd, ed) = (0, 0, 0);
                            }
                        }
                        _ if !inside => {}
                        "Hardkey" => {
                            let c = control(e);
                            if c.named() {
                                map.hardkeys.insert(hk as u16, c);
                            }
                            hk += 1;
                        }
                        "FaderDefinition" => {
                            let c = control(e);
                            if c.named() {
                                map.faders.insert(fd as u8, c);
                            }
                            fd += 1;
                        }
                        "EncoderDefinition" => {
                            let c = control(e);
                            if c.named() {
                                map.encoders.insert(ed as u8, c);
                            }
                            ed += 1;
                        }
                        "LedDefinition" => map.leds.push(led(e)),
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) if e.name().as_ref() == "HardwareConfiguration" => {
                    if inside {
                        return Ok(map);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(MapError::Xml(e.to_string())),
                _ => {}
            }
            buf.clear();
        }

        let known = WingMap::modules(xml).join(", ");
        Err(MapError::NoSuchModule(module.to_string(), known))
    }

    pub fn hardkey(&self, index: u16) -> Option<&Control> {
        self.hardkeys.get(&index)
    }
    pub fn fader(&self, index: u8) -> Option<&Control> {
        self.faders.get(&index)
    }
    pub fn encoder(&self, index: u8) -> Option<&Control> {
        self.encoders.get(&index)
    }

    /// Every hardkey that actually has a meaning, in index order.
    pub fn assigned_hardkeys(&self) -> impl Iterator<Item = (u16, &Control)> {
        self.hardkeys.iter().map(|(k, c)| (*k, c))
    }

    /// The LED whose code matches, for lighting a named key.
    pub fn led_for_code(&self, code: &str) -> Option<&Led> {
        self.leds.iter().find(|l| l.code.as_deref() == Some(code))
    }
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).and_then(|a| {
        // MA's file declares 1.0; attribute values here are plain ASCII anyway.
        let v = a.normalized_value(quick_xml::XmlVersion::Explicit1_0).ok()?.into_owned();
        (!v.is_empty()).then_some(v)
    })
}

fn control(e: &BytesStart<'_>) -> Control {
    let mut c = Control {
        label: String::new(),
        code: attr(e, "Code"),
        kind: attr(e, "Type"),
        key: attr(e, "Key"),
        executor: attr(e, "ExecutorIndex"),
        special: attr(e, "SpecialExecutor"),
    };
    c.build_label();
    c
}

fn led(e: &BytesStart<'_>) -> Led {
    let num = |n: &str| attr(e, n).and_then(|v| v.parse().ok());
    let code = attr(e, "Code");
    let executor = attr(e, "ExecutorIndex");
    Led {
        label: code
            .clone()
            .or_else(|| executor.as_ref().map(|x| format!("exec {x}")))
            .unwrap_or_default(),
        code,
        r: num("R"),
        g: num("G"),
        b: num("B"),
        executor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cut-down file in exactly the shape MA's is, comments and all.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GMA3 DataVersion="0.0.57.0">
    <HardwareConfigurations>
        <HardwareConfiguration Name="something else" IsWing="0">
            <Hardkeys>
                <!-- 000 --> <Hardkey Code="NOPE" />
            </Hardkeys>
        </HardwareConfiguration>
        <HardwareConfiguration Name="grandMA3 onPC command wing" IsWing="1">
            <Hardkeys>
                <!-- 000 --> <Hardkey Code="" />
                <!-- 001 --> <Hardkey Code="" />
                <!-- 002 --> <Hardkey Code="CLEAR" />
                <!-- 003 --> <Hardkey Code="LEARN" />
            </Hardkeys>
            <FaderDefinitions>
                <FaderDefinition ExecutorIndex="201" />
                <FaderDefinition ExecutorIndex="202" />
            </FaderDefinitions>
            <EncoderDefinitions>
                <!-- 000 --> <EncoderDefinition />
                <!-- 001 --> <EncoderDefinition Type="Inside1" Key="ENCODER_INSIDE1" Linked="2" />
            </EncoderDefinitions>
            <LedDefinitions HasBootAnimation="Yes">
                <LedDefinition Code="NUM0" R="75" IsButton="1" />
                <LedDefinition ExecutorIndex="291" R="88" G="93" B="106" IsButton="0" />
            </LedDefinitions>
        </HardwareConfiguration>
    </HardwareConfigurations>
</GMA3>"#;

    #[test]
    fn an_index_is_a_position_not_a_comment() {
        let m = WingMap::from_xml(SAMPLE, COMMAND_WING).unwrap();
        // Slots 0 and 1 are blank and carry no control, but they still occupy their
        // places — CLEAR is hardkey 2 because it is the third element.
        assert_eq!(m.hardkey(2).unwrap().code.as_deref(), Some("CLEAR"));
        assert_eq!(m.hardkey(3).unwrap().code.as_deref(), Some("LEARN"));
        assert!(m.hardkey(0).is_none());
        assert_eq!(m.encoder(1).unwrap().kind.as_deref(), Some("Inside1"));
        assert_eq!(m.fader(1).unwrap().executor.as_deref(), Some("202"));
    }

    #[test]
    fn takes_the_module_it_was_asked_for() {
        let m = WingMap::from_xml(SAMPLE, COMMAND_WING).unwrap();
        assert_eq!(m.module, COMMAND_WING);
        // The other configuration in the file must not bleed in.
        assert!(m.assigned_hardkeys().all(|(_, c)| c.code.as_deref() != Some("NOPE")));
        assert_eq!(WingMap::modules(SAMPLE).len(), 2);
    }

    #[test]
    fn an_unknown_module_says_what_the_file_does_have() {
        let e = WingMap::from_xml(SAMPLE, "grandMA3 fader wing").unwrap_err();
        assert!(e.to_string().contains("grandMA3 onPC command wing"), "{e}");
    }

    #[test]
    fn reads_leds_and_their_channels() {
        let m = WingMap::from_xml(SAMPLE, COMMAND_WING).unwrap();
        assert_eq!(m.led_for_code("NUM0").unwrap().r, Some(75));
        assert!(!m.led_for_code("NUM0").unwrap().is_rgb());
        assert!(m.leds[1].is_rgb());
    }

    #[test]
    fn unassigned_slots_are_not_controls() {
        let m = WingMap::from_xml(SAMPLE, COMMAND_WING).unwrap();
        let idx: Vec<u16> = m.assigned_hardkeys().map(|(i, _)| i).collect();
        assert_eq!(idx, vec![2, 3]);
    }
}
