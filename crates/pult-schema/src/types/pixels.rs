//! A pixel strip on an OpenHaunt node, patched in one of its two modes.
//!
//! `OpenHaunt/node/docs/14-pixels.md` is the node's half. A strip is a head of the node
//! (see [`crate::types::openhaunt::NodeDescription::for_head`]), and how it is patched
//! for the night lives on that head's address, as a DMX mode does: in `effect` mode the
//! node draws every pixel from nine ports the head's own fixture type carries, and in
//! `sacn` mode the console draws every pixel and the node reads universes.
//!
//! **In `sacn` mode the strip is one fixture with a cell per pixel.** Its type is
//! [`strip_type`]: a color per cell, and a mode whose breaks are the universes the
//! 170-to-a-universe spill puts it across — so the DMX family renders it from
//! [`crate::types::fixture::FixtureAddress::dmx_placement`] like any other fixture, and
//! everything that walks a selection's elements walks its pixels. Task 74 made every
//! pixel its own fixture instead; see task 82 for why that changed.
//!
//! Everything here is pure: the station makes the writes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::types::dmx_mode::{DmxBreak, DmxChannelLayout, DmxMode};
use crate::types::fixture::{
    parameter_key, rgb_emitters, Emitter, FixturePhysical, FixtureType, FixtureTypeSource,
    ParameterDefinition, ParameterKind, ParameterValue, Vec3, DEFAULT_MODE,
};

/// Fixed for ever, like every namespace here: changing it re-types every strip.
const NAMESPACE: Uuid = Uuid::from_u128(0x7069_7865_6c73_4f48_9a1d_6f6f7e2c8a02);

/// The most pixels the node drives on one output, from its own `max_count`. A strip is
/// refused past it, and the console never asks for more than the node can take.
pub const MAX_PIXELS: u16 = 1024;

/// Channels a universe carries.
const UNIVERSE: u16 = 512;

// ── What the node says ────────────────────────────────────────────────────────

/// Which of the two ways a strip runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum StripMode {
    /// The node draws the pixels, from parameters.
    #[default]
    Effect,
    /// The console draws the pixels, and sends them as sACN.
    Sacn,
}

/// One strip, as the node's `pixels.strips` describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeStrip {
    pub output: u8,
    pub head: u8,
    pub mode: StripMode,
    pub count: u16,
    pub order: String,
    #[serde(default)]
    pub universe: u16,
    #[serde(default = "first_address")]
    pub address: u16,
    #[serde(default)]
    pub hold_ms: u32,
    #[serde(default)]
    pub universes: Vec<u16>,
    #[serde(default)]
    pub max_fps: u32,
}

fn first_address() -> u16 {
    1
}

/// The `pixels` object of a node's `/info`.
///
/// Read from the raw body beside the description rather than out of it, for the reason
/// effects and audio are: the fixture type's id hashes the description.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodePixels {
    #[serde(default)]
    pub effects: Vec<String>,
    #[serde(default)]
    pub orders: Vec<String>,
    #[serde(default)]
    pub max_count: u16,
    #[serde(default)]
    pub multicast_universes: u16,
    #[serde(default)]
    pub strips: Vec<NodeStrip>,
}

impl NodePixels {
    pub fn from_info(info: &serde_json::Value) -> Option<NodePixels> {
        serde_json::from_value(info.get("pixels")?.clone()).ok()
    }

    pub fn strip_for_head(&self, head: u8) -> Option<&NodeStrip> {
        self.strips.iter().find(|s| s.head == head)
    }
}

// ── What the show says ────────────────────────────────────────────────────────

/// How a strip head is patched: on its address, beside the serial and the head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PixelStrip {
    /// The strip output on the board, the `<n>` in `/api/v1/pixels/<n>`.
    pub output: u8,
    pub mode: StripMode,
    pub count: u16,
    /// The color order on the strip's own wire. The universes always carry R, G, B
    /// (and W), so this matters to the node and to nothing here but the channel count.
    pub order: String,
    /// Where pixel 0's first channel is, in `sacn` mode. Kept in `effect` mode too, so
    /// switching back and forth does not lose it.
    pub universe: u16,
    pub address: u16,
    /// How long the node holds its last frame when every source has gone.
    #[serde(default = "default_hold")]
    pub hold_ms: u32,
    /// Distance from one pixel to the next, in meters: 60 to the meter unless said.
    #[serde(default = "default_pitch")]
    pub pitch_m: f32,
}

fn default_hold() -> u32 {
    10_000
}

fn default_pitch() -> f32 {
    1.0 / 60.0
}

impl PixelStrip {
    /// What the show takes on when it adopts a strip: whatever the node is doing now.
    pub fn from_node(strip: &NodeStrip) -> PixelStrip {
        PixelStrip {
            output: strip.output,
            mode: strip.mode,
            count: strip.count.max(1),
            order: strip.order.clone(),
            universe: strip.universe,
            address: strip.address.max(1),
            hold_ms: strip.hold_ms,
            pitch_m: default_pitch(),
        }
    }

    pub fn rgbw(&self) -> bool {
        self.order.contains('W')
    }

    /// Bytes per pixel in a universe.
    pub fn channels(&self) -> u16 {
        if self.rgbw() { 4 } else { 3 }
    }

    /// Where every pixel's first channel lands, pixel 0 first.
    ///
    /// The node's rule, which is WLED's and xLights': a pixel never straddles two
    /// universes, the first holds what fits after `address`, and every later one starts
    /// at 1 and holds 170 RGB or 128 RGBW. An address too close to the end for one
    /// pixel starts the strip in the next universe.
    pub fn layout(&self) -> Vec<DmxBreak> {
        let c = self.channels();
        let mut universe = self.universe.max(1);
        let mut address = self.address.clamp(1, UNIVERSE);
        let mut out = Vec::with_capacity(self.count as usize);
        for _ in 0..self.count {
            if address + c - 1 > UNIVERSE {
                universe = universe.saturating_add(1);
                address = 1;
            }
            out.push(DmxBreak { universe, address });
            address += c;
        }
        out
    }

    /// Every universe the strip reads, in order. Empty in `effect` mode.
    pub fn universes(&self) -> Vec<u16> {
        if self.mode != StripMode::Sacn {
            return Vec::new();
        }
        let mut list: Vec<u16> = self.layout().iter().map(|b| b.universe).collect();
        list.dedup();
        list
    }

    /// What `PUT /api/v1/pixels/<output>` is sent.
    pub fn node_body(&self) -> serde_json::Value {
        serde_json::json!({
            "mode": self.mode,
            "count": self.count,
            "order": self.order,
            "universe": self.universe,
            "address": self.address,
            "hold_ms": self.hold_ms,
        })
    }

    /// Whether the node is running this strip as the show says. The show is the truth,
    /// so anything different is put back.
    pub fn agrees_with(&self, node: &NodeStrip) -> bool {
        let placed = self.mode == StripMode::Effect
            || (self.universe == node.universe && self.address == node.address);
        self.mode == node.mode
            && self.count == node.count
            && self.order == node.order
            && self.hold_ms == node.hold_ms
            && placed
    }

    /// Whether this is a strip the node could run at all.
    pub fn problem(&self, pixels: Option<&NodePixels>) -> Option<String> {
        let max = pixels.map(|p| p.max_count).filter(|m| *m > 0).unwrap_or(MAX_PIXELS);
        if self.count == 0 || self.count > max {
            return Some(format!("a strip is 1 to {max} pixels, not {}", self.count));
        }
        if let Some(orders) = pixels.map(|p| &p.orders).filter(|o| !o.is_empty()) {
            if !orders.contains(&self.order) {
                return Some(format!("the node drives {}, not {}", orders.join(", "), self.order));
            }
        }
        if self.mode == StripMode::Sacn {
            if self.universe == 0 || self.universe > 63_999 {
                return Some(format!("universe {} is not an sACN universe", self.universe));
            }
            if self.address == 0 || self.address > UNIVERSE {
                return Some(format!("address {} is not in a universe", self.address));
            }
        }
        None
    }
}

// ── The fixture a strip is ───────────────────────────────────────────────────

impl PixelStrip {
    /// One break per universe the strip reaches, each at the address its first pixel
    /// lands on there. What [`strip_type`]'s mode is laid out against, and what the
    /// strip's address answers for `dmx_placement`.
    pub fn placement(&self) -> Vec<DmxBreak> {
        let mut breaks: Vec<DmxBreak> = Vec::new();
        for at in self.layout() {
            if breaks.last().is_none_or(|b| b.universe != at.universe) {
                breaks.push(at);
            }
        }
        breaks
    }
}

/// The type a strip in `sacn` mode is: one color per cell, laid out across universes.
///
/// Derived from the count, the channel count and the start address — which decides
/// how many pixels fit in the first universe — and from nothing else, so every strip
/// of one shape shares a row, and moving a strip to another universe moves its address
/// and not its type.
pub fn strip_type(strip: &PixelStrip) -> FixtureType {
    let mut emitters = rgb_emitters();
    if strip.rgbw() {
        emitters.push(Emitter {
            name: "White".into(),
            rgb: Some(Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            subtractive: false,
        });
    }
    let address = strip.address.clamp(1, UNIVERSE);
    let name = format!("Pixel strip, {} {}", strip.count, if strip.rgbw() { "RGBW" } else { "RGB" });

    let breaks = strip.placement();
    let mut channels = Vec::new();
    let mut footprint = vec![0u16; breaks.len()];
    for (index, at) in strip.layout().into_iter().enumerate() {
        let break_index = breaks.iter().position(|b| b.universe == at.universe).unwrap_or(0);
        let start = at.address - breaks[break_index].address + 1;
        let key = parameter_key(&ParameterKind::ColorRgb.in_cell(index as u16 + 1));
        // Always R, G, B (, W) in a universe: the node reorders for its own wire.
        for (n, emitter) in emitters.iter().enumerate() {
            let offset = start + n as u16;
            footprint[break_index] = footprint[break_index].max(offset);
            channels.push(DmxChannelLayout {
                parameter_key: key.clone(),
                break_index: break_index as u8,
                offsets: vec![offset],
                default: 0,
                functions: Vec::new(),
                emitter: Some(emitter.name.clone()),
            });
        }
    }

    FixtureType {
        id: Uuid::new_v5(
            &NAMESPACE,
            format!("strip/{}/{}/{address}", strip.count, strip.channels()).as_bytes(),
        ),
        name: name.clone(),
        manufacturer: "OpenHaunt".into(),
        long_name: name,
        channel_count: footprint.first().copied().unwrap_or(0),
        parameters: vec![ParameterDefinition {
            emitters,
            per_cell: true,
            ..ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
        }],
        dmx_modes: vec![DmxMode { name: DEFAULT_MODE.into(), breaks: footprint, channels }],
        physical: FixturePhysical::default(),
        source: FixtureTypeSource::Node,
        cells: strip.count,
        ..FixtureType::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(count: u16, order: &str, universe: u16, address: u16) -> PixelStrip {
        PixelStrip {
            output: 1,
            mode: StripMode::Sacn,
            count,
            order: order.into(),
            universe,
            address,
            hold_ms: 10_000,
            pitch_m: 0.1,
        }
    }


    #[test]
    fn three_hundred_rgb_pixels_spill_into_a_second_universe_at_170() {
        let layout = strip(300, "GRB", 5, 1).layout();
        assert_eq!(layout[0], DmxBreak { universe: 5, address: 1 });
        assert_eq!(layout[169], DmxBreak { universe: 5, address: 508 });
        assert_eq!(layout[170], DmxBreak { universe: 6, address: 1 });
        assert_eq!(layout[299], DmxBreak { universe: 6, address: 388 });
        assert_eq!(strip(300, "GRB", 5, 1).universes(), vec![5, 6]);
    }

    #[test]
    fn rgbw_holds_128_to_a_universe() {
        let layout = strip(200, "GRBW", 1, 1).layout();
        assert_eq!(layout[127], DmxBreak { universe: 1, address: 509 });
        assert_eq!(layout[128], DmxBreak { universe: 2, address: 1 });
    }

    #[test]
    fn the_first_universe_holds_what_fits_after_the_address() {
        // floor((513 - 100) / 3) = 137
        let layout = strip(150, "RGB", 3, 100).layout();
        assert_eq!(layout[136], DmxBreak { universe: 3, address: 508 });
        assert_eq!(layout[137], DmxBreak { universe: 4, address: 1 });
    }

    #[test]
    fn an_address_with_no_room_for_one_pixel_starts_in_the_next_universe() {
        let layout = strip(2, "RGB", 7, 511).layout();
        assert_eq!(layout[0], DmxBreak { universe: 8, address: 1 });
        assert_eq!(strip(2, "RGB", 7, 511).universes(), vec![8]);
    }





    #[test]
    fn a_node_running_something_else_is_put_back() {
        let s = strip(300, "GRB", 5, 1);
        let mut node = NodeStrip {
            output: 1,
            head: 2,
            mode: StripMode::Sacn,
            count: 300,
            order: "GRB".into(),
            universe: 5,
            address: 1,
            hold_ms: 10_000,
            universes: vec![5, 6],
            max_fps: 40,
        };
        assert!(s.agrees_with(&node));
        node.mode = StripMode::Effect;
        assert!(!s.agrees_with(&node));
        node.mode = StripMode::Sacn;
        node.universe = 6;
        assert!(!s.agrees_with(&node));
    }

    #[test]
    fn a_strip_the_node_cannot_run_is_refused_in_words() {
        let pixels = NodePixels { max_count: 1024, orders: vec!["GRB".into()], ..Default::default() };
        assert!(strip(2000, "GRB", 1, 1).problem(Some(&pixels)).is_some());
        assert!(strip(10, "RGBW", 1, 1).problem(Some(&pixels)).is_some());
        assert!(strip(10, "GRB", 0, 1).problem(Some(&pixels)).is_some());
        assert!(strip(10, "GRB", 1, 1).problem(Some(&pixels)).is_none());
    }

    #[test]
    fn an_effect_strip_reads_no_universes() {
        let mut s = strip(60, "GRB", 1, 1);
        s.mode = StripMode::Effect;
        assert!(s.universes().is_empty());
    }

    #[test]
    fn a_sacn_strip_is_one_fixture_with_a_cell_per_pixel() {
        let t = strip_type(&strip(4, "GRB", 9, 1));
        assert_eq!(t.cells, 4);
        assert!(t.has_cells());
        let keys: Vec<&str> = t.dmx_modes[0].channels.iter().map(|c| c.parameter_key.as_str()).collect();
        assert_eq!(&keys[..4], ["ColorRgb@1", "ColorRgb@1", "ColorRgb@1", "ColorRgb@2"]);
        assert_eq!(t.dmx_modes[0].channels[3].offsets, vec![4], "pixel 2 starts at 4");
        assert_eq!(t.dmx_modes[0].breaks, vec![12]);
    }

    #[test]
    fn a_strip_across_two_universes_is_two_breaks() {
        let s = strip(300, "GRB", 5, 1);
        assert_eq!(s.placement(), vec![DmxBreak { universe: 5, address: 1 }, DmxBreak { universe: 6, address: 1 }]);
        let t = strip_type(&s);
        assert_eq!(t.dmx_modes[0].breaks, vec![510, 390]);
        let pixel_171: Vec<_> =
            t.dmx_modes[0].channels.iter().filter(|c| c.parameter_key == "ColorRgb@171").collect();
        assert!(pixel_171.iter().all(|c| c.break_index == 1));
        assert_eq!(pixel_171[0].offsets, vec![1]);
    }

    #[test]
    fn rgbw_pixels_carry_a_white_channel_and_a_type_of_their_own() {
        let rgbw = strip_type(&strip(2, "GRBW", 1, 1));
        assert_eq!(rgbw.dmx_modes[0].channels[3].emitter.as_deref(), Some("White"));
        assert_eq!(rgbw.dmx_modes[0].breaks, vec![8]);
        assert_ne!(rgbw.id, strip_type(&strip(2, "GRB", 1, 1)).id);
    }

    #[test]
    fn a_strip_moved_to_another_universe_keeps_its_type() {
        assert_eq!(strip_type(&strip(60, "GRB", 1, 1)).id, strip_type(&strip(60, "GRB", 7, 1)).id);
        assert_ne!(strip_type(&strip(60, "GRB", 1, 1)).id, strip_type(&strip(60, "GRB", 1, 100)).id);
    }
}
