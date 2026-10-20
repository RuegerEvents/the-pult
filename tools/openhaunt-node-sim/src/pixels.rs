//! A node's WS2812 outputs, as `OpenHaunt/node/docs/14-pixels.md` describes them.
//!
//! Written again from the document rather than shared with the console, for the
//! reason the rest of this crate is: the spill rule here and the one the console
//! patches cells by are two readings of one page, and a test that drives both is what
//! says they agree.
//!
//! What this node does not do is draw anything. In `effect` mode the nine ports hold
//! their values like any other port; in `sacn` mode the frames are counted and the last
//! one per universe is kept, which is what a test needs to see a chase arrive.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::PortDescription;

/// The effects the firmware draws, in its order.
pub const EFFECTS: [&str; 10] =
    ["solid", "gradient", "chase", "comet", "wave", "breathe", "fire", "candle", "sparkle", "lightning"];

pub const ORDERS: [&str; 8] = ["RGB", "RBG", "GRB", "GBR", "BRG", "BGR", "RGBW", "GRBW"];

pub const MAX_COUNT: u16 = 1024;

/// One strip output, in a config file and on the node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StripConfig {
    pub output: u8,
    pub head: u8,
    /// `effect` or `sacn`.
    #[serde(default = "effect")]
    pub mode: String,
    #[serde(default = "sixty")]
    pub count: u16,
    #[serde(default = "grb")]
    pub order: String,
    #[serde(default)]
    pub universe: u16,
    #[serde(default = "one")]
    pub address: u16,
    #[serde(default = "ten_seconds")]
    pub hold_ms: u32,
}

fn effect() -> String {
    "effect".into()
}
fn sixty() -> u16 {
    60
}
fn grb() -> String {
    "GRB".into()
}
fn one() -> u16 {
    1
}
fn ten_seconds() -> u32 {
    10_000
}

impl StripConfig {
    /// A strip as a node comes out of the box: `effect`, 60 pixels, GRB.
    pub fn default_for(output: u8) -> Self {
        StripConfig {
            output,
            head: output + 1,
            mode: effect(),
            count: sixty(),
            order: grb(),
            universe: 0,
            address: one(),
            hold_ms: ten_seconds(),
        }
    }

    fn channels(&self) -> u16 {
        if self.order.contains('W') { 4 } else { 3 }
    }

    /// Where each universe starts, the document's rule: the first holds
    /// `floor((513 - address) / c)` pixels, every later one `floor(512 / c)`, and an
    /// address with no room for one pixel starts in the next universe.
    pub fn universes(&self) -> Vec<u16> {
        if self.mode != "sacn" {
            return Vec::new();
        }
        let c = self.channels();
        let mut list = Vec::new();
        let mut universe = self.universe;
        let mut room = (513u16.saturating_sub(self.address)) / c;
        if room == 0 {
            universe += 1;
            room = 512 / c;
        }
        let mut left = self.count;
        while left > 0 {
            list.push(universe);
            left = left.saturating_sub(room);
            universe += 1;
            room = 512 / c;
        }
        list
    }

    /// The nine ports a strip in `effect` mode owns. Strip 1 is 6–14, strip 2 15–23.
    pub fn ports(&self) -> Vec<PortDescription> {
        use PortDescription as P;
        if self.mode != "effect" {
            return Vec::new();
        }
        let base = 6 + 9 * self.output;
        let head = Some(self.head);
        let port = |offset: u8, p: P| P { head, ..p.with_port(base + offset) };
        vec![
            port(0, P::new(0, "Color", "readwrite", "color").class("color")),
            port(1, P::new(0, "Brightness", "readwrite", "number")
                .unit("percent")
                .range(0.0, 1.0)
                .default_at(0.0)
                .class("intensity")),
            port(2, P::new(0, "Color 2", "readwrite", "color")),
            port(3, P::new(0, "Effect", "readwrite", "string").class("pixel-effect")),
            port(4, P::new(0, "Speed", "readwrite", "number").unit("hertz").range(0.0, 10.0).default_at(1.0).class("speed")),
            port(5, P::new(0, "Size", "readwrite", "number").unit("percent").range(0.0, 1.0).default_at(0.25).class("size")),
            port(6, P::new(0, "Reverse", "readwrite", "boolean").default_at(0.0).class("reverse")),
            port(7, P::new(0, "Mirror", "readwrite", "boolean").default_at(0.0).class("mirror")),
            port(8, P::new(0, "Segments", "readwrite", "number").unit("unitless").range(1.0, 16.0).default_at(1.0).class("segments")),
        ]
    }

    fn describe(&self) -> Value {
        json!({
            "output": self.output,
            "head": self.head,
            "mode": self.mode,
            "count": self.count,
            "order": self.order,
            "universe": self.universe,
            "address": self.address,
            "hold_ms": self.hold_ms,
            "universes": self.universes(),
            "max_fps": (1_000_000 / (self.count as u32 * 30 + 300)).min(40),
        })
    }
}

/// What a strip in `sacn` mode has been sent: how many frames, and the last one per
/// universe, channels from 1.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StripFrames {
    pub frames: u64,
    pub universes: BTreeMap<u16, Vec<u8>>,
}

/// The node's strips, as they run.
pub struct Pixels {
    strips: Mutex<Vec<StripConfig>>,
    frames: watch::Sender<BTreeMap<u8, StripFrames>>,
}

impl Pixels {
    pub fn new(strips: Vec<StripConfig>) -> (Self, watch::Receiver<BTreeMap<u8, StripFrames>>) {
        let (frames, rx) = watch::channel(BTreeMap::new());
        (Pixels { strips: Mutex::new(strips), frames }, rx)
    }

    pub fn is_empty(&self) -> bool {
        self.strips.lock().unwrap().is_empty()
    }

    pub fn strips(&self) -> Vec<StripConfig> {
        self.strips.lock().unwrap().clone()
    }

    /// The ports every strip in `effect` mode owns, in output order.
    pub fn ports(&self) -> Vec<PortDescription> {
        self.strips().iter().flat_map(StripConfig::ports).collect()
    }

    pub fn heads(&self) -> Value {
        json!(self
            .strips()
            .iter()
            .map(|s| json!({ "head": s.head, "name": format!("Strip {}", s.output + 1), "kind": "pixel-strip" }))
            .collect::<Vec<_>>())
    }

    /// The `pixels` object of `/info`.
    pub fn describe(&self) -> Value {
        json!({
            "effects": EFFECTS,
            "orders": ORDERS,
            "max_count": MAX_COUNT,
            "multicast_universes": 4,
            "strips": self.strips().iter().map(StripConfig::describe).collect::<Vec<_>>(),
        })
    }

    /// `GET /api/v1/pixels`: the same, with what each strip has been sent.
    pub fn status(&self) -> Value {
        let mut body = self.describe();
        let frames = self.frames.borrow().clone();
        if let Some(strips) = body["strips"].as_array_mut() {
            for strip in strips {
                let output = strip["output"].as_u64().unwrap_or(0) as u8;
                let seen = frames.get(&output).cloned().unwrap_or_default();
                strip["status"] = json!({ "frames": seen.frames, "sources": u8::from(seen.frames > 0), "holding": false, "fps": 0.0 });
            }
        }
        body
    }

    /// `PUT /api/v1/pixels/<output>`: merged over what the strip has, checked, kept.
    pub fn put(&self, output: u8, body: &Value) -> Result<Value, (u16, String)> {
        let mut strips = self.strips.lock().unwrap();
        let Some(strip) = strips.iter_mut().find(|s| s.output == output) else {
            return Err((404, format!("no strip output {output}")));
        };
        let mut next = strip.clone();
        if let Some(mode) = body["mode"].as_str() {
            if mode != "effect" && mode != "sacn" {
                return Err((400, format!("mode {mode:?} is not effect or sacn")));
            }
            next.mode = mode.into();
        }
        if let Some(count) = body["count"].as_u64() {
            if count == 0 || count > MAX_COUNT as u64 {
                return Err((400, format!("count {count} is out of range")));
            }
            next.count = count as u16;
        }
        if let Some(order) = body["order"].as_str() {
            if !ORDERS.contains(&order) {
                return Err((400, format!("order {order:?} is not one this node drives")));
            }
            next.order = order.into();
        }
        if let Some(universe) = body["universe"].as_u64() {
            next.universe = universe as u16;
        }
        if let Some(address) = body["address"].as_u64() {
            if address == 0 || address > 512 {
                return Err((400, format!("address {address} is out of range")));
            }
            next.address = address as u16;
        }
        if let Some(hold) = body["hold_ms"].as_u64() {
            next.hold_ms = hold as u32;
        }
        if next.mode == "sacn" && (next.universe == 0 || next.universe > 63_999) {
            return Err((400, format!("universe {} is out of range", next.universe)));
        }
        *strip = next.clone();
        Ok(next.describe())
    }

    /// An E1.31 frame arrived for a universe. Counted against every strip reading it.
    pub fn receive(&self, universe: u16, channels: &[u8]) {
        let readers: Vec<u8> = self
            .strips()
            .iter()
            .filter(|s| s.universes().contains(&universe))
            .map(|s| s.output)
            .collect();
        if readers.is_empty() {
            return;
        }
        self.frames.send_modify(|frames| {
            for output in readers {
                let seen = frames.entry(output).or_default();
                seen.frames += 1;
                seen.universes.insert(universe, channels.to_vec());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sacn(count: u16, order: &str, universe: u16, address: u16) -> StripConfig {
        StripConfig {
            mode: "sacn".into(),
            count,
            order: order.into(),
            universe,
            address,
            ..StripConfig::default_for(0)
        }
    }

    #[test]
    fn the_documents_own_examples_spill_where_it_says() {
        assert_eq!(sacn(300, "GRB", 5, 1).universes(), vec![5, 6]);
        assert_eq!(sacn(128, "GRBW", 1, 1).universes(), vec![1]);
        assert_eq!(sacn(129, "GRBW", 1, 1).universes(), vec![1, 2]);
        assert_eq!(sacn(2, "RGB", 7, 511).universes(), vec![8]);
    }

    #[test]
    fn a_strip_in_effect_mode_owns_nine_ports_on_its_head() {
        let ports = StripConfig::default_for(1).ports();
        assert_eq!(ports.len(), 9);
        assert_eq!(ports[0].port, 15);
        assert!(ports.iter().all(|p| p.head == Some(2)));
        assert!(sacn(60, "GRB", 1, 1).ports().is_empty(), "and none in sacn mode");
    }

    #[test]
    fn a_put_merges_and_refuses_what_the_node_cannot_run() {
        let (pixels, _) = Pixels::new(vec![StripConfig::default_for(0)]);
        let strip = pixels.put(0, &json!({ "mode": "sacn", "universe": 5 })).unwrap();
        assert_eq!(strip["count"], 60);
        assert_eq!(strip["universes"], json!([5]));
        assert_eq!(pixels.put(0, &json!({ "count": 5000 })).unwrap_err().0, 400);
        assert_eq!(pixels.put(3, &json!({})).unwrap_err().0, 404);
    }
}
