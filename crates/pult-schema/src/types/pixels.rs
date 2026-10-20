//! A pixel strip on an OpenHaunt node, patched in one of its two modes.
//!
//! `OpenHaunt/node/docs/14-pixels.md` is the node's half. A strip is a head of the node
//! (see [`crate::types::openhaunt::NodeDescription::for_head`]), and how it is patched
//! for the night lives on that head's address, as a DMX mode does: in `effect` mode the
//! node draws every pixel from nine ports the head's own fixture type carries, and in
//! `sacn` mode the console draws every pixel and the node reads universes.
//!
//! **In `sacn` mode a pixel is a fixture.** A cell is an ordinary DMX-addressed fixture
//! with a color, hung off one scene object that is the strip, so everything the console
//! already does to a selection of fixtures — spread, phase, width, a chase in the order
//! somebody picked — it does to a strip's cells, and the DMX family renders them with
//! nothing taught. The alternative, one fixture with a cell count, would have needed
//! the evaluator, both compilations of it and every consumer to learn what a cell is.
//!
//! Everything here is pure: [`plan`] says what the show should hold for a strip, and the
//! station makes the writes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::types::dmx_mode::{DmxBreak, DmxChannelLayout, DmxMode};
use crate::types::fixture::{
    rgb_emitters, Emitter, Fixture, FixtureAddress, FixturePhysical, FixtureType,
    FixtureTypeSource, ParameterDefinition, ParameterKind, ParameterValue, Vec3, DEFAULT_MODE,
};
use crate::types::group::{
    Group, SelectionClause, SelectionCombine, SelectionOrder, SelectionQuery, SelectionTerm,
};
use crate::types::scene::{SceneObject, SceneObjectKind, Transform};

/// Fixed for ever, like every namespace here: changing it orphans every cell.
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

// ── The fixtures a strip becomes ──────────────────────────────────────────────

/// The scene object a strip's cells hang off: the strip itself, to the rig view.
pub fn strip_object_id(serial: &str, head: u8) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("strip/{serial}/{head}").as_bytes())
}

/// The saved group that is one strip's cells, in order along it.
pub fn strip_group_id(serial: &str, head: u8) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("group/{serial}/{head}").as_bytes())
}

/// One cell's id. Derived, so a count that shrinks and grows again finds the same rows
/// and whatever a cue captured against pixel 37 is pixel 37 again.
pub fn cell_id(serial: &str, head: u8, index: u16) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("cell/{serial}/{head}/{index}").as_bytes())
}

/// The fixture type every cell of one channel count shares.
pub fn cell_type(rgbw: bool) -> FixtureType {
    let mut emitters = rgb_emitters();
    if rgbw {
        emitters.push(Emitter {
            name: "White".into(),
            rgb: Some(Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            subtractive: false,
        });
    }
    let key = crate::types::fixture::parameter_key(&ParameterKind::ColorRgb);
    let name = if rgbw { "Pixel RGBW" } else { "Pixel RGB" };
    FixtureType {
        id: Uuid::new_v5(&NAMESPACE, name.as_bytes()),
        name: name.into(),
        manufacturer: "OpenHaunt".into(),
        long_name: name.into(),
        channel_count: emitters.len() as u16,
        parameters: vec![ParameterDefinition {
            emitters: emitters.clone(),
            ..ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
        }],
        // Always R, G, B (, W) in a universe: the node reorders for its own wire.
        dmx_modes: vec![DmxMode {
            name: DEFAULT_MODE.into(),
            breaks: vec![emitters.len() as u16],
            channels: emitters
                .iter()
                .enumerate()
                .map(|(index, emitter)| DmxChannelLayout {
                    parameter_key: key.clone(),
                    break_index: 0,
                    offsets: vec![index as u16 + 1],
                    default: 0,
                    functions: Vec::new(),
                    emitter: Some(emitter.name.clone()),
                })
                .collect(),
        }],
        physical: FixturePhysical {
            dimensions_m: Some(Vec3 { x: 0.01, y: 0.01, z: 0.01 }),
            pixel: true,
            ..FixturePhysical::default()
        },
        source: FixtureTypeSource::Node,
        ..FixtureType::default()
    }
}

/// What the show should hold for one strip head, worked out before anything is written.
#[derive(Debug, Clone)]
pub struct StripPlan {
    /// The head with its new address.
    pub head: Fixture,
    /// Rows to create where the show has none: the strip's object, its group and the
    /// cells' type. Written only if absent, so a strip somebody has moved stays moved.
    pub object: SceneObject,
    pub cell_type: Option<FixtureType>,
    /// The group is always written, because its order is the cells.
    pub group: Option<Group>,
    pub create: Vec<Fixture>,
    /// Cells that exist and have to move to another address or place.
    pub update: Vec<Fixture>,
    pub delete: Vec<Uuid>,
}

/// What patching `head` as `strip` takes.
///
/// `cells` are the fixtures hanging off the strip's object now. A cell keeps where an
/// operator put it only along the strip: its address and its place on the strip are
/// the strip's, and are rewritten. Everything else about it — a name somebody typed, a
/// layer — survives a count change.
pub fn plan(head: &Fixture, strip: PixelStrip, cells: &[Fixture]) -> StripPlan {
    let (serial, head_no) = match &head.address {
        FixtureAddress::OpenHaunt { serial, head, .. } => (serial.clone(), head.unwrap_or(0)),
        FixtureAddress::Dmx { .. } => (String::new(), 0),
    };
    let object_id = strip_object_id(&serial, head_no);
    let mut new_head = head.clone();
    if let FixtureAddress::OpenHaunt { strip: s, .. } = &mut new_head.address {
        *s = Some(strip.clone());
    }

    let object = SceneObject {
        id: object_id,
        name: head.name.clone(),
        kind: SceneObjectKind::Group,
        transform: Transform::default(),
        parent: None,
        layer: head.layer,
        class: None,
        geometry: Vec::new(),
        symbol: None,
        catalog: None,
        properties: serde_json::Value::Null,
        locked: false,
        weight_kg: None,
    };

    let wanted: Vec<(Uuid, DmxBreak)> = if strip.mode == StripMode::Sacn {
        strip
            .layout()
            .into_iter()
            .enumerate()
            .map(|(index, at)| (cell_id(&serial, head_no, index as u16), at))
            .collect()
    } else {
        Vec::new()
    };
    let cell_type = cell_type(strip.rgbw());

    let mut create = Vec::new();
    let mut update = Vec::new();
    for (index, (id, at)) in wanted.iter().enumerate() {
        let address = FixtureAddress::Dmx { mode: DEFAULT_MODE.into(), breaks: vec![*at] };
        let position = Some(Transform::at(Vec3 { x: index as f32 * strip.pitch_m, y: 0.0, z: 0.0 }));
        match cells.iter().find(|c| c.id == *id) {
            Some(existing) => {
                let mut cell = existing.clone();
                cell.address = address;
                cell.position = position;
                cell.parent = Some(object_id);
                cell.fixture_type_id = cell_type.id;
                let same = serde_json::to_value(&cell).ok() == serde_json::to_value(existing).ok();
                if !same {
                    update.push(cell);
                }
            }
            None => create.push(Fixture {
                id: *id,
                name: format!("{} {}", head.name, index + 1),
                fixture_type_id: cell_type.id,
                address,
                position,
                parent: Some(object_id),
                layer: head.layer,
                unit_number: Some(index as u32 + 1),
                ..Fixture::default()
            }),
        }
    }
    let delete = cells
        .iter()
        .filter(|c| !wanted.iter().any(|(id, _)| *id == c.id))
        .map(|c| c.id)
        .collect();

    let sacn = strip.mode == StripMode::Sacn;
    let group = sacn.then(|| Group {
        id: strip_group_id(&serial, head_no),
        name: head.name.clone(),
        color: None,
        query: SelectionQuery {
            clauses: vec![
                SelectionClause {
                    combine: SelectionCombine::Add,
                    term: SelectionTerm::Hanging { object: object_id },
                },
                SelectionClause {
                    combine: SelectionCombine::Keep,
                    term: SelectionTerm::OfType { type_id: cell_type.id },
                },
            ],
            // Along the strip, pixel 0 first, which is the order a chase runs in.
            order: SelectionOrder::Manual { order: wanted.iter().map(|(id, _)| *id).collect() },
        },
    });

    StripPlan {
        head: new_head,
        object,
        cell_type: sacn.then_some(cell_type),
        group,
        create,
        update,
        delete,
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

    fn head() -> Fixture {
        Fixture {
            id: crate::types::openhaunt::head_fixture_id("4d5e6f", 2),
            name: "Gate strip".into(),
            fixture_type_id: Uuid::nil(),
            address: FixtureAddress::OpenHaunt {
                serial: "4d5e6f".into(),
                universe: None,
                head: Some(2),
                strip: None,
            },
            ..Fixture::default()
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
    fn an_effect_strip_reads_no_universes_and_has_no_cells() {
        let mut s = strip(60, "GRB", 1, 1);
        s.mode = StripMode::Effect;
        assert!(s.universes().is_empty());
        let p = plan(&head(), s, &[]);
        assert!(p.create.is_empty() && p.group.is_none() && p.cell_type.is_none());
    }

    #[test]
    fn a_sacn_strip_is_a_cell_per_pixel_along_its_object() {
        let p = plan(&head(), strip(4, "GRB", 9, 1), &[]);
        assert_eq!(p.create.len(), 4);
        let object = strip_object_id("4d5e6f", 2);
        assert!(p.create.iter().all(|c| c.parent == Some(object)));
        assert_eq!(p.create[3].position.unwrap().position.x, 0.3);
        assert_eq!(p.create[1].address.breaks()[0], DmxBreak { universe: 9, address: 4 });
        assert_eq!(p.head.address.strip().unwrap().count, 4);
        let group = p.group.unwrap();
        assert_eq!(
            group.query.order,
            SelectionOrder::Manual { order: p.create.iter().map(|c| c.id).collect() },
            "a chase runs along the strip",
        );
    }

    #[test]
    fn a_strip_that_shrinks_and_grows_keeps_its_cells() {
        let first = plan(&head(), strip(4, "GRB", 9, 1), &[]);
        let shrunk = plan(&head(), strip(2, "GRB", 9, 1), &first.create);
        assert_eq!(shrunk.delete, vec![first.create[2].id, first.create[3].id]);
        assert!(shrunk.create.is_empty() && shrunk.update.is_empty(), "nothing else moved");

        let mut named = first.create[1].clone();
        named.name = "The one by the gate".into();
        let moved = plan(&head(), strip(4, "GRB", 20, 1), &[first.create[0].clone(), named]);
        assert_eq!(moved.create.len(), 2, "pixels 3 and 4 come back with their own ids");
        assert_eq!(moved.create[0].id, first.create[2].id);
        assert_eq!(moved.update.len(), 2, "and the two that stayed move universe");
        assert_eq!(moved.update[1].name, "The one by the gate");
    }

    #[test]
    fn rgbw_cells_carry_a_white_channel() {
        let t = cell_type(true);
        assert_eq!(t.dmx_modes[0].breaks, vec![4]);
        assert_eq!(t.dmx_modes[0].channels[3].emitter.as_deref(), Some("White"));
        assert!(t.physical.pixel);
        assert_ne!(cell_type(true).id, cell_type(false).id);
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
}
