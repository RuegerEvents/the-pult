use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::dmx_mode::{DmxBreak, DmxChannelLayout, DmxMode};
use super::effect::{RunningEffect, RunningFade};
use super::programmer::ProgrammerValue;
use crate::PultSchema;

/// A point in the rig, in metres, from whatever origin the show uses.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Where a fixture is, and for a moving one, where it points.
///
/// The spec asks for positions to be either positional (XYZ) or axial (a position
/// and a direction vector). Nothing forces a position to be accurate: a rig can be
/// laid out roughly and corrected later, or updated from tracking data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FixturePosition {
    /// Just where it hangs.
    Point(Vec3),
    /// Where it hangs and the direction it faces at rest.
    Axial { position: Vec3, direction: Vec3 },
}

impl FixturePosition {
    pub fn position(&self) -> Vec3 {
        match self {
            FixturePosition::Point(p) => *p,
            FixturePosition::Axial { position, .. } => *position,
        }
    }
}

/// How output reaches a fixture.
///
/// Not every fixture is on a DMX line. An OpenHaunt node is addressed by the serial
/// of the node it lives on, and only a DMX gateway module also carries a universe —
/// for a relay or a sensor there is no universe to speak of.
///
/// A DMX address carries the **mode** as well as the place, because the place alone
/// does not say what the bytes mean: the same head at universe 1 address 1 occupies
/// nine channels in one mode and thirty-one in another. And it carries a place *per
/// break*, because a fixture with a separate dimmer break sits in two spans that need
/// not be in the same universe.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub enum FixtureAddress {
    Dmx {
        /// Which of the type's modes this unit is set to. A name the type may not
        /// have: a show patched against a GDTF file that has since been revised names
        /// a mode the new file dropped, and it is the type that resolves that, not
        /// the address.
        mode: String,
        /// Where each DMX break lands, break 1 first. Never empty.
        breaks: Vec<DmxBreak>,
    },
    OpenHaunt {
        serial: String,
        universe: Option<u16>,
    },
}

/// `fixtures.address` is one JSON column, so a showfile written before modes existed
/// holds `{"Dmx": {"universe": 1, "address": 5}}`. Reading it back through this shape
/// is the whole of that migration: there is no column to alter, and the old form
/// becomes mode `"Default"` with one break, which is exactly what it meant.
///
/// Written out by hand rather than through `#[serde(from = ...)]`, for the reason
/// [`ParameterDefinitionWire`] is — that attribute is one more thing for ts-rs to fail
/// to parse and warn about.
#[derive(Deserialize)]
enum FixtureAddressWire {
    Dmx(DmxAddressWire),
    OpenHaunt { serial: String, universe: Option<u16> },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DmxAddressWire {
    Modal { mode: String, breaks: Vec<DmxBreak> },
    Legacy { universe: u16, address: u16 },
}

impl<'de> Deserialize<'de> for FixtureAddress {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match FixtureAddressWire::deserialize(deserializer)? {
            FixtureAddressWire::Dmx(DmxAddressWire::Modal { mode, breaks }) => {
                FixtureAddress::Dmx { mode, breaks }
            }
            FixtureAddressWire::Dmx(DmxAddressWire::Legacy { universe, address }) => {
                FixtureAddress::dmx(universe, address)
            }
            FixtureAddressWire::OpenHaunt { serial, universe } => {
                FixtureAddress::OpenHaunt { serial, universe }
            }
        })
    }
}

impl Default for FixtureAddress {
    fn default() -> Self {
        FixtureAddress::dmx(1, 1)
    }
}

impl FixtureAddress {
    /// A single-break DMX address in the default mode: what the patch panel makes when
    /// somebody types a universe and a channel.
    pub fn dmx(universe: u16, address: u16) -> Self {
        FixtureAddress::Dmx {
            mode: DEFAULT_MODE.into(),
            breaks: vec![DmxBreak { universe, address }],
        }
    }

    /// Universe and start address of the first break, for the fixtures that have one.
    ///
    /// Named apart from the old `dmx()` on purpose: every caller that had one break
    /// still means the first, and a caller that wants all of them should have had to
    /// notice. [`FixtureAddress::breaks`] is that one.
    pub fn dmx_start(&self) -> Option<(u16, u16)> {
        match self {
            FixtureAddress::Dmx { breaks, .. } => {
                breaks.first().map(|first| (first.universe, first.address))
            }
            // A gateway module carries a universe but its own address is the node,
            // not a slot in that universe: it owns all 512 channels.
            FixtureAddress::OpenHaunt { .. } => None,
        }
    }

    /// Where every break lands.
    pub fn breaks(&self) -> &[DmxBreak] {
        match self {
            FixtureAddress::Dmx { breaks, .. } => breaks,
            FixtureAddress::OpenHaunt { .. } => &[],
        }
    }

    /// Which of the type's modes this unit is patched in.
    pub fn mode(&self) -> Option<&str> {
        match self {
            FixtureAddress::Dmx { mode, .. } => Some(mode),
            FixtureAddress::OpenHaunt { .. } => None,
        }
    }

    /// The node serial, for fixtures that live on one.
    pub fn serial(&self) -> Option<&str> {
        match self {
            FixtureAddress::OpenHaunt { serial, .. } => Some(serial),
            FixtureAddress::Dmx { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterKind {
    Intensity,
    ColorRgb,
    Pan,
    Tilt,
    GoboIndex,
    /// Beam angle.
    Zoom,
    /// Sharpness, not angle: what a hard edge is made with.
    Focus,
    Iris,
    /// The mechanical shutter, as a level: closed to open.
    Shutter,
    /// Rate, while the shutter is strobing.
    Strobe,
    /// A gobo wheel, numbered as the fixture numbers its own: `Gobo(1)` is the first.
    /// Indexed because a head commonly has two, and calling the second one something
    /// else would put it under a key nothing looks for.
    Gobo(u8),
    GoboRotation(u8),
    ColorWheel(u8),
    Prism(u8),
    Frost(u8),
    /// Correlated colour temperature in kelvin, for a variable-white fixture. Not a
    /// colour: a tunable-white bar has this and no RGB at all.
    ColorTemperature,
    Raw(u8),
    /// A switched output: a relay, a dry contact the console closes.
    Switch(u8),
    /// A switch or button the console reads.
    Contact(u8),
    Temperature,
    Humidity,
    AirQuality,
    /// A line of text, for a display module.
    Text,
    /// Something a device declared that this console has no name for.
    ///
    /// A node describes its own ports, and it is allowed to describe one the
    /// console has never heard of. The name it gave is the whole identity: it is
    /// what the operator sees and what the parameter key is built from.
    Named(String),
}

// What a parameter can be, and the arithmetic over it, live in `pult-render`: the
// browser evaluates values now and cannot depend on this crate. Re-exported under the
// path it has always had.
pub use pult_render::value::ParameterValue;

/// Which way a parameter flows.
///
/// Everything the console has driven so far is an output. A sensor node reverses
/// that: the device writes the value and the show reads it, which is what lets a
/// contact closure be an ordinary fixture parameter rather than a separate concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterDirection {
    #[default]
    Output,
    Input,
}

/// Where a parameter sits on the thing that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterBinding {
    /// An offset from the fixture's DMX start address, 1-based.
    ///
    /// **Legacy, and read-only.** Where a DMX channel is is a fact about a *mode*, not
    /// about a parameter, so new code writes a [`DmxChannelLayout`] and nothing writes
    /// this. It survives because every showfile and every demo seed written before
    /// modes existed says it, and [`FixtureType::mode`] reads those to build the
    /// implicit default mode — which is what lets an old show open unchanged rather
    /// than be migrated.
    Dmx { channel: u8 },
    /// A port on an I/O module, 0-based, as the module numbers its own terminals.
    Port { index: u8 },
}

impl ParameterBinding {
    pub fn dmx_channel(&self) -> Option<u8> {
        match self {
            ParameterBinding::Dmx { channel } => Some(*channel),
            ParameterBinding::Port { .. } => None,
        }
    }

    pub fn port(&self) -> Option<u8> {
        match self {
            ParameterBinding::Port { index } => Some(*index),
            ParameterBinding::Dmx { .. } => None,
        }
    }
}

/// What a number on a parameter is measured in.
///
/// The console's own list, and a subset of GDTF's: the units a fixture's channels are
/// actually declared in. Without it a pan is "a number between 0 and 1", and an
/// operator who wants the head at −90° has to know how wide the head's travel is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PhysicalUnit {
    #[default]
    None,
    Percent,
    Degrees,
    Seconds,
    Hertz,
    Kelvin,
    Metres,
    Watts,
}

/// What a parameter's 0..1 actually spans.
///
/// A pan whose range is −270 to 270 reads and writes in degrees; one with no range
/// reads as a fraction, which is the honest answer for a fixture that never said.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PhysicalRange {
    pub from: f32,
    pub to: f32,
    #[serde(default)]
    pub unit: PhysicalUnit,
}

impl PhysicalRange {
    /// A normalised value as a physical one.
    pub fn to_physical(&self, normalised: f32) -> f32 {
        self.from + (self.to - self.from) * normalised
    }

    /// And back. A zero-width range answers zero rather than dividing by nothing.
    pub fn to_normalised(&self, physical: f32) -> f32 {
        if (self.to - self.from).abs() < f32::EPSILON {
            return 0.0;
        }
        (physical - self.from) / (self.to - self.from)
    }
}

/// One named position on a wheel: a gobo, a colour, a prism facet.
///
/// What turns "Gobo 1 at 37%" into "Gobo 1: Breakup".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Slot {
    pub name: String,
    /// The colour of this slot, where it has one, as linear RGB. A colour wheel's
    /// slots do; a gobo wheel's do not.
    #[serde(default)]
    pub color: Option<Vec3>,
    /// The gobo image, in the asset store, where the file carried one.
    #[serde(default)]
    pub media: Option<String>,
}

/// One light source in the fixture.
///
/// The list that makes a colour mixable. Given a colour and these, the console works
/// out a level per emitter; without them an RGBW head's fourth channel is a number
/// with no meaning and stays dark.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Emitter {
    /// The GDTF emitter name — `Red`, `White`, `Lime` — which is also how a
    /// per-emitter override in a colour value names it.
    pub name: String,
    /// Where this emitter sits in linear RGB, from its CIE colour. `None` for one the
    /// file did not measure, which then mixes by its name and nothing else.
    #[serde(default)]
    pub rgb: Option<Vec3>,
    /// A subtractive element — a CMY flag — rather than a source. Full means *less*
    /// light, so it is the complement of the colour it is named for.
    #[serde(default)]
    pub subtractive: bool,
}

/// What a fixture type weighs, draws, and is.
///
/// Nothing here drives a light. It is what the paperwork needs — a truss loading, a
/// power run, a case list — and until a GDTF file could be imported the console had no
/// place to keep any of it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FixturePhysical {
    #[serde(default)]
    pub weight_kg: Option<f32>,
    #[serde(default)]
    pub power_w: Option<f32>,
    /// Length, width, height in metres.
    #[serde(default)]
    pub dimensions_m: Option<Vec3>,
    #[serde(default)]
    pub connectors: Vec<Connector>,
    /// Floor to the bottom of the fixture, for one that stands.
    #[serde(default)]
    pub leg_height_m: Option<f32>,
    /// The range it is rated for, in Celsius.
    #[serde(default)]
    pub operating_temperature: Option<(f32, f32)>,
    /// The beam's full angle in degrees, where the file said one. What the rig view's
    /// cone should be drawn at, instead of a constant that is right for no fixture.
    #[serde(default)]
    pub beam_angle_deg: Option<f32>,
}

/// A socket on the fixture.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Connector {
    pub name: String,
    /// `XLR5`, `powerCON`, `RJ45` — the file's own spelling.
    pub kind: String,
    /// Which DMX break this one carries, for a data connector.
    #[serde(default)]
    pub dmx_break: Option<u16>,
}

/// Where a fixture type came from, which decides whether the console may rewrite it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FixtureTypeSource {
    /// Typed into the editor, or put there by the demo seed. The console leaves it
    /// alone.
    #[default]
    Manual,
    /// Derived from what an OpenHaunt node said about its own ports, and **rebuilt
    /// whenever the node says it again**. That rebuilding is why this distinction has
    /// to exist at all: doing it to an imported type would throw the file away.
    Node,
    /// Imported from a GDTF file, whose bytes are kept whole in the asset store.
    Gdtf {
        /// The `.gdtf`'s sha256. The file is the record; this row is a reading of it.
        asset: String,
        /// The file's own `FixtureTypeID`, which is also this type's `id`.
        uuid: String,
        /// The revision text the file carried, so an operator can tell two files of
        /// the same fixture apart when the name cannot.
        revision: String,
        /// The GDTF Share revision it was downloaded as, where it was.
        #[serde(default)]
        share_rid: Option<u32>,
    },
}

impl FixtureTypeSource {
    /// Whether a device re-describing itself may overwrite this type.
    ///
    /// A node's own type: yes, that is how a node's ports stay true. Anything else: no
    /// — a GDTF import and a hand-edited type are both somebody's decision, and a node
    /// happening to report the same fixture id must not undo it.
    pub fn is_derived_from_a_node(&self) -> bool {
        matches!(self, FixtureTypeSource::Node)
    }
}

/// One node of a fixture's own geometry, flattened.
///
/// A GDTF file describes the fixture as a tree — a base, a yoke that turns, a head
/// that turns inside it, a beam that comes out of the head — and that tree is what
/// makes a 3D body articulate instead of swinging as a block. Flattened rather than
/// nested because a parent name is enough to rebuild the tree and a recursive type
/// costs the frontend a recursive renderer for nothing.
///
/// A summary rather than the file: the console parses the GDTF once, at import, and
/// what it read is here. The alternative — the browser fetching the `.gdtf` and
/// parsing the XML itself — would be a second GDTF reader, in a second language,
/// disagreeing with the first about which axis pan turns.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FixtureGeometry {
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub kind: GeometryKind,
    /// Where this part sits relative to its parent, in metres, in console axes.
    pub offset: Vec3,
    /// Length, width, height in metres, for the box to draw when there is no mesh.
    #[serde(default)]
    pub size: Option<Vec3>,
    /// The mesh in the asset store, where one was extracted.
    #[serde(default)]
    pub model_asset: Option<String>,
    /// Degrees, on a beam.
    #[serde(default)]
    pub beam_angle_deg: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum GeometryKind {
    /// A part that does not move.
    #[default]
    Body,
    /// A part that turns: what pan and tilt drive. The outermost one is pan.
    Axis,
    /// Where the light comes out.
    Beam,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ParameterDefinition {
    pub kind: ParameterKind,
    #[serde(default)]
    pub direction: ParameterDirection,
    /// Where this parameter sits on the *device*, for a device that has ports.
    ///
    /// `None` for a DMX-only parameter, which is most of them: where a DMX channel is
    /// belongs to a mode, and a parameter has no one answer to it.
    #[serde(default)]
    pub binding: Option<ParameterBinding>,
    pub default_value: ParameterValue,
    /// What this parameter's 0..1 spans, where the fixture said.
    #[serde(default)]
    pub physical: Option<PhysicalRange>,
    /// The named positions on it, for a wheel.
    #[serde(default)]
    pub slots: Vec<Slot>,
    /// Which encoder page this belongs on: `Dimmer`, `Position`, `Color`, `Beam`.
    #[serde(default)]
    pub feature_group: Option<String>,
    /// The light sources a colour parameter mixes across. Empty on everything else.
    #[serde(default)]
    pub emitters: Vec<Emitter>,
}

/// `fixture_types.parameters` is one JSON column, so a showfile written before
/// bindings existed holds `dmx_channel` where `binding` now goes. Reading it back
/// through this shape is the whole migration for that field — there is no column to
/// alter, and a show that has never been reopened stays readable.
///
/// Written out by hand rather than through `#[serde(from = ...)]`, because that
/// attribute is one more thing for ts-rs to fail to parse and warn about.
#[derive(Deserialize)]
struct ParameterDefinitionWire {
    kind: ParameterKind,
    #[serde(default)]
    direction: ParameterDirection,
    binding: Option<ParameterBinding>,
    dmx_channel: Option<u8>,
    default_value: ParameterValue,
    #[serde(default)]
    physical: Option<PhysicalRange>,
    #[serde(default)]
    slots: Vec<Slot>,
    #[serde(default)]
    feature_group: Option<String>,
    #[serde(default)]
    emitters: Vec<Emitter>,
}

impl<'de> Deserialize<'de> for ParameterDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ParameterDefinitionWire::deserialize(deserializer)?;
        Ok(ParameterDefinition {
            kind: wire.kind,
            direction: wire.direction,
            // The legacy `dmx_channel` still folds into a `Dmx` binding rather than
            // being dropped, because that binding is what the implicit default mode is
            // computed from. Losing it would repatch every old show at channel 1.
            binding: wire
                .binding
                .or_else(|| wire.dmx_channel.map(|channel| ParameterBinding::Dmx { channel })),
            default_value: wire.default_value,
            physical: wire.physical,
            slots: wire.slots,
            feature_group: wire.feature_group,
            emitters: wire.emitters,
        })
    }
}

impl ParameterDefinition {
    /// A parameter with nothing but a kind and a resting value: what the editor and
    /// the seed make, and what every field added since fills in for itself.
    pub fn new(kind: ParameterKind, default_value: ParameterValue) -> Self {
    ParameterDefinition {
    kind,
    direction: ParameterDirection::Output,
    binding: None,
    default_value,
    physical: None,
    slots: Vec::new(),
    feature_group: None,
    emitters: Vec::new(),
    }
    }

    /// The same, bound to a port on an I/O module.
    pub fn on_port(kind: ParameterKind, index: u8, default_value: ParameterValue) -> Self {
    ParameterDefinition {
        binding: Some(ParameterBinding::Port { index }),
        ..ParameterDefinition::new(kind, default_value)
    }
    }
}

/// The mode every fixture is in until somebody says otherwise, and the name the
/// implicit one takes.
pub const DEFAULT_MODE: &str = "Default";

/// Template describing what parameters a fixture type has.
///
/// The parameter list is what the light can *do*; [`FixtureType::modes`] is how those
/// parameters reach a DMX line, and which one a given unit uses is on its address.
///
/// `Default` is a blank type — no id, no name, no channels, and
/// [`FixtureTypeSource::Manual`], which is what a type nobody has filled in yet
/// honestly is. It is what the fields added since the patch went in are filled from,
/// so a caller that names the five it cares about does not have to name the rest.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "fixture_types")]
pub struct FixtureType {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub manufacturer: String,
    /// The four-or-so characters a patch sheet has room for.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub short_name: String,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub long_name: String,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub description: String,
    /// How many channels the default mode's first break occupies.
    ///
    /// Kept as a plain number because it is what the patch panel has always shown and
    /// what an operator means by "how big is it". The real answer, for a fixture with
    /// more than one break or more than one mode, is [`FixtureType::footprint`]; this
    /// is maintained alongside it rather than replaced, so nothing that read it had to
    /// change.
    #[pult(lifecycle = PERSISTED)]
    pub channel_count: u16,
    #[pult(lifecycle = PERSISTED)]
    pub parameters: Vec<ParameterDefinition>,
    /// The ways this type can be addressed over DMX.
    ///
    /// Empty on everything the console made for itself, and then there is still one
    /// mode — see [`FixtureType::mode`], which computes it rather than a load-time
    /// rewrite writing it. Defaulted on the wire so a showfile from before modes
    /// existed opens with this field simply absent.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub dmx_modes: Vec<DmxMode>,
    /// What it weighs and draws.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub physical: FixturePhysical,
    /// The parts it is made of, for a 3D body that articulates.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub geometry: Vec<FixtureGeometry>,
    /// Where this type came from, and so whether the console may rewrite it.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub source: FixtureTypeSource,
}

impl FixtureType {
    /// Every mode this type has, real or implicit.
    ///
    /// A type with no `dmx_modes` still has one: everything patched before modes
    /// existed carries a `Dmx { channel }` binding per parameter, and that is a
    /// layout. Computing it here rather than rewriting the row at load is deliberate,
    /// and the reason is the SQLite read path — `from_columns` reads each column on
    /// its own, so a deserialize-time rewrite would never see the other columns it
    /// needs, and a NULL one would panic. A function sees the whole row and gives
    /// every station the same answer.
    pub fn modes(&self) -> Cow<'_, [DmxMode]> {
        if self.dmx_modes.is_empty() {
            Cow::Owned(vec![self.implicit_mode()])
        } else {
            Cow::Borrowed(&self.dmx_modes)
        }
    }

    /// The mode of this name, falling back to the first.
    ///
    /// Unknown rather than missing: a show patched against a GDTF file that has since
    /// been revised names a mode the new file dropped, and going dark is worse than
    /// going to the first mode. The connector says so, once.
    pub fn mode(&self, name: &str) -> Cow<'_, DmxMode> {
        match self.dmx_modes.iter().find(|mode| mode.name == name) {
            Some(mode) => Cow::Borrowed(mode),
            None => match self.dmx_modes.first() {
                Some(first) => Cow::Borrowed(first),
                None => Cow::Owned(self.implicit_mode()),
            },
        }
    }

    /// Whether this type names a mode of its own by that name.
    ///
    /// What the connector checks before warning: the implicit mode is not a choice
    /// anybody made, so a fixture in it is not a fixture in the wrong one.
    pub fn has_mode(&self, name: &str) -> bool {
        self.dmx_modes.iter().any(|mode| mode.name == name)
    }

    /// How many channels a mode occupies, per break.
    pub fn footprint(&self, mode: &str) -> Vec<u16> {
        self.mode(mode).breaks.clone()
    }

    /// The one mode a type without modes has.
    ///
    /// Built from the legacy `Dmx` bindings where any parameter still carries one — a
    /// colour spanning three consecutive channels from its own, which is what the
    /// connector did before layouts existed — and from parameter order otherwise, one
    /// byte each and three for a colour.
    fn implicit_mode(&self) -> DmxMode {
        let outputs: Vec<&ParameterDefinition> = output_parameters(self).collect();
        let bound = outputs.iter().any(|p| p.binding.and_then(|b| b.dmx_channel()).is_some());

        let mut channels = Vec::new();
        let mut next = 1u16;
        for parameter in outputs {
            let key = parameter_key(&parameter.kind);
            let is_colour = matches!(parameter.default_value, ParameterValue::Color { .. });
            let width = if is_colour { 3 } else { 1 };

            // A parameter on a *port* has no DMX channel at all, whatever the fixture
            // is addressed to. Giving it a slot in the sequential fallback would put a
            // relay's state on some other fixture's dimmer.
            let on_a_port = parameter.binding.is_some_and(|b| b.port().is_some());
            let start = match (on_a_port, bound, parameter.binding.and_then(|b| b.dmx_channel())) {
                (true, _, _) => 0,
                (_, true, Some(channel)) => channel as u16,
                // A parameter with no channel in a type where the others have one has
                // nowhere to go: inventing a slot would land it on top of one that does.
                (_, true, None) => 0,
                (_, false, _) => {
                    let start = next;
                    next += width;
                    start
                }
            };

            if is_colour {
                // Three channels rather than one three-byte channel: an emitter per
                // byte is how every other mode says which die a byte drives, and one
                // shape for both keeps the connector from having a colour special case.
                for (index, emitter) in rgb_emitters().into_iter().enumerate() {
                    channels.push(DmxChannelLayout {
                        parameter_key: key.clone(),
                        break_index: 0,
                        offsets: if start == 0 {
                            Vec::new()
                        } else {
                            vec![start + index as u16]
                        },
                        default: 0,
                        functions: Vec::new(),
                        emitter: Some(emitter.name),
                    });
                }
            } else {
                channels.push(DmxChannelLayout {
                    parameter_key: key,
                    break_index: 0,
                    offsets: if start == 0 { Vec::new() } else { vec![start] },
                    default: 0,
                    functions: Vec::new(),
                    emitter: None,
                });
            }
        }

        let footprint =
            channels.iter().flat_map(|channel| channel.offsets.iter().copied()).max().unwrap_or(0);

        DmxMode { name: DEFAULT_MODE.into(), breaks: vec![footprint], channels }
    }
}

/// The three emitters a colour parameter has when the fixture never named any.
///
/// Every colour the console could send before GDTF existed was three consecutive
/// bytes in this order, and a type derived from an OpenHaunt node's colour port still
/// is. Naming them makes that the same shape as an RGBW head's four, so the mixer and
/// the connector have one path rather than a legacy one beside a real one.
pub fn rgb_emitters() -> Vec<Emitter> {
    vec![
        Emitter { name: "Red".into(), rgb: Some(Vec3 { x: 1.0, y: 0.0, z: 0.0 }), subtractive: false },
        Emitter { name: "Green".into(), rgb: Some(Vec3 { x: 0.0, y: 1.0, z: 0.0 }), subtractive: false },
        Emitter { name: "Blue".into(), rgb: Some(Vec3 { x: 0.0, y: 0.0, z: 1.0 }), subtractive: false },
    ]
}

/// The emitters a parameter mixes across, filled in for a colour that named none.
///
/// One place rather than at each caller, because a colour with no emitters is not a
/// colour with nothing to drive — it is one whose fixture predates the question.
pub fn emitters_of(parameter: &ParameterDefinition) -> Vec<Emitter> {
    if !parameter.emitters.is_empty() {
        return parameter.emitters.clone();
    }
    if matches!(parameter.default_value, ParameterValue::Color { .. }) {
        return rgb_emitters();
    }
    Vec::new()
}

/// A patched fixture instance — a specific unit in the rig.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "fixtures")]
pub struct Fixture {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub fixture_type_id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub address: FixtureAddress,
    /// Where this fixture is in the rig. None until it has been placed.
    #[pult(lifecycle = PERSISTED)]
    pub position: Option<FixturePosition>,
    /// What this fixture's devices have *told* the console, keyed by parameter key:
    /// a contact closure, a temperature, a humidity — anything a device reports.
    ///
    /// The one kind of value the console still stores, and the reason it does is that
    /// it cannot work it out. What a fixture is being *driven* to is a function of the
    /// fades and effects on it, the programmer over them and its home value beneath,
    /// evaluated for whatever moment a consumer asks about; what it is *sensing*
    /// arrived off a wire and is not a function of anything the console holds.
    ///
    /// SYNCED, and for a reason that survived the removal of its driven half: nothing
    /// else on the network can work this value out, because the wire it came off is
    /// attached to this station.
    ///
    /// Defaulted on the wire so a showfile or a peer from before the split — where one
    /// map carried both halves under the name `live_values` — opens with this field
    /// simply absent. The old name is *ignored* rather than aliased onto this one: what
    /// it mostly carried was driven values, and those are now a function of what is
    /// driving them. Taking them for readings would be filing the console's own output
    /// as something a device reported.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub sensed_values: HashMap<String, ParameterValue>,
    /// What shape is driving each of this fixture's parameters, keyed by parameter
    /// key.
    ///
    /// Together with `live_fades` this is *what the fixture is doing*, and the whole of
    /// it: nothing stores the values these come to. A shape and its anchor are enough
    /// for anybody holding the row to work out what the parameter is at any moment they
    /// care about, which is what lets an output connector run at its protocol's rate, a
    /// node trace the shape itself, and a browser draw it at its own refresh.
    ///
    /// LOCAL rather than SYNCED. Every station works these out for itself from
    /// replicated cue state, so broadcasting them would be sending each console a
    /// slower copy of what it has already computed.
    ///
    /// Defaulted on the wire: a LOCAL field is worked out by the station that holds
    /// it, so a client creating a fixture has nothing to say about it and should not
    /// have to name it. Without this, `fixtures/__create` refuses a body that is
    /// otherwise complete because it omitted a field only the engine can fill.
    #[serde(default)]
    #[pult(lifecycle = LOCAL)]
    pub live_effects: HashMap<String, RunningEffect>,
    /// The fades on each of this fixture's parameters, keyed by parameter key. LOCAL
    /// and defaulted for the same reasons as above.
    ///
    /// **Including the ones that have arrived**, which is what makes this the record of
    /// where the rig is rather than a list of what is in flight. A landed fade is a
    /// constant function of time, and since nothing stores the number it landed on, it
    /// is the only thing that remembers it. One entry per parameter anything has ever
    /// driven, replaced when a cue, a release or an action takes the key.
    #[serde(default)]
    #[pult(lifecycle = LOCAL)]
    pub live_fades: HashMap<String, RunningFade>,
    /// What this fixture's parameters rest at when nothing is driving them, keyed by
    /// parameter key. Empty on nearly every fixture, and then the answer is
    /// whatever its type declares.
    ///
    /// On the fixture rather than on the type because a house light that comes up
    /// when nothing is controlling it is a fact about *this* rig, and because a type
    /// is derived: the node describes its ports again and the console rebuilds it,
    /// which would take an override with it. Defaulted on the wire so a show written
    /// before this existed opens with every fixture saying nothing.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub home_values: HashMap<String, ParameterValue>,
}

/// The map key for a parameter, in `home_values`, `sensed_values`, `live_fades` and
/// `live_effects` alike.
///
/// Here rather than in the backend because three places derive it — the engine, the
/// browser, and the command-line plugin — and a fourth spelling of it would be a
/// fixture whose values quietly land under a key nothing reads.
pub fn parameter_key(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Intensity => "Intensity".into(),
        ParameterKind::ColorRgb => "ColorRgb".into(),
        ParameterKind::Pan => "Pan".into(),
        ParameterKind::Tilt => "Tilt".into(),
        ParameterKind::GoboIndex => "GoboIndex".into(),
        ParameterKind::Zoom => "Zoom".into(),
        ParameterKind::Focus => "Focus".into(),
        ParameterKind::Iris => "Iris".into(),
        ParameterKind::Shutter => "Shutter".into(),
        ParameterKind::Strobe => "Strobe".into(),
        ParameterKind::Gobo(n) => format!("Gobo:{n}"),
        ParameterKind::GoboRotation(n) => format!("GoboRotation:{n}"),
        ParameterKind::ColorWheel(n) => format!("ColorWheel:{n}"),
        ParameterKind::Prism(n) => format!("Prism:{n}"),
        ParameterKind::Frost(n) => format!("Frost:{n}"),
        ParameterKind::ColorTemperature => "ColorTemperature".into(),
        ParameterKind::Raw(channel) => format!("Raw:{channel}"),
        ParameterKind::Switch(n) => format!("Switch:{n}"),
        ParameterKind::Contact(n) => format!("Contact:{n}"),
        ParameterKind::Temperature => "Temperature".into(),
        ParameterKind::Humidity => "Humidity".into(),
        ParameterKind::AirQuality => "AirQuality".into(),
        ParameterKind::Text => "Text".into(),
        ParameterKind::Named(name) => format!("Named:{name}"),
    }
}

/// What a parameter rests at when nothing is driving it.
///
/// The fixture's own override where it has one, and what its type declares
/// otherwise. `None` where the type has no such parameter, which is the only honest
/// answer: a fixture that cannot pan has nowhere for a pan to rest.
///
/// One resolution, in the schema, because the engine resolving a relative write, the
/// engine sending a selection home and playback letting go of a key all have to
/// agree about it — and because `default_value` is what a *device* said, which is
/// the answer only until somebody overrides it.
pub fn home_value(
    fixture: &Fixture,
    fixture_type: &FixtureType,
    kind: &ParameterKind,
) -> Option<ParameterValue> {
    home_value_by_key(fixture, Some(fixture_type), &parameter_key(kind))
}

/// The same question, asked with the key instead of the kind.
///
/// Everything downstream of the engine already holds the key — a running fade, a held
/// programmer entry — and going back to a kind to come forward to the same string
/// again would be a second place for the two to disagree. So this is the resolution
/// and [`home_value`] is the spelling of it that starts from a kind.
///
/// The type is optional because an override does not need it: where a fixture is
/// patched to a type this station has not received yet, its own answer is still its
/// own answer, and a house light should not go dark waiting for a row to replicate.
pub fn home_value_by_key(
    fixture: &Fixture,
    fixture_type: Option<&FixtureType>,
    key: &str,
) -> Option<ParameterValue> {
    home_value_ref_by_key(fixture, fixture_type, key).cloned()
}

/// The same answer, borrowed rather than cloned.
///
/// What [`driving`] needs: a parameter's home value is looked up once per parameter
/// per output frame, and cloning a colour at that rate to hand it straight to an
/// evaluator that only reads it is the sort of cost that becomes the whole frame on a
/// rig of thousands.
pub fn home_value_ref_by_key<'a>(
    fixture: &'a Fixture,
    fixture_type: Option<&'a FixtureType>,
    key: &str,
) -> Option<&'a ParameterValue> {
    if let Some(overridden) = fixture.home_values.get(key) {
        return Some(overridden);
    }
    fixture_type?
        .parameters
        .iter()
        .find(|p| parameter_key(&p.kind) == key)
        .map(|p| &p.default_value)
}

// ── What is driving a parameter ──────────────────────────────────────

/// The programmer's entries, indexed by the parameter each one holds.
///
/// Built once by whoever is about to evaluate a rig. The alternative — scanning the
/// entries per parameter — is what makes a frame quadratic in the size of a look, and
/// a busy programmer is thousands of entries.
#[derive(Default)]
pub struct HeldByProgrammer<'a>(HashMap<(Uuid, String), &'a ProgrammerValue>);

impl<'a> HeldByProgrammer<'a> {
    pub fn of(entries: &'a [ProgrammerValue]) -> Self {
        Self(
            entries
                .iter()
                .map(|entry| ((entry.fixture_id, parameter_key(&entry.parameter_kind)), entry))
                .collect(),
        )
    }

    pub fn get(&self, fixture_id: Uuid, key: &str) -> Option<&'a ProgrammerValue> {
        self.0.get(&(fixture_id, key.to_string())).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Everything acting on one parameter, gathered from the show.
///
/// The station publishes the winner of playback's two layers into `live_effects` and
/// `live_fades`, so those two are read straight off the fixture. The programmer is
/// read from the SYNCED collection instead, because it is show data rather than
/// something a station worked out — and only where it holds a plain value: an entry
/// carrying a shape has already been resolved against its speed master and published
/// as the running effect, which is what keeps rate-following out of the evaluator.
pub fn driving<'a>(
    fixture: &'a Fixture,
    fixture_type: Option<&'a FixtureType>,
    held: Option<&'a ProgrammerValue>,
    key: &str,
) -> pult_render::Driving<'a> {
    pult_render::Driving {
        programmer: held.filter(|entry| entry.effect.is_none()).map(|entry| &entry.value),
        effect: fixture.live_effects.get(key),
        fade: fixture.live_fades.get(key),
        home: home_value_ref_by_key(fixture, fixture_type, key),
    }
}

/// What one parameter of one fixture is putting out at `now_ms`.
///
/// The whole of "what is this light doing" in one call, for the callers that want an
/// answer rather than the layers behind it.
pub fn value_at(
    fixture: &Fixture,
    fixture_type: Option<&FixtureType>,
    held: Option<&ProgrammerValue>,
    key: &str,
    now_ms: u64,
) -> Option<ParameterValue> {
    pult_render::value_at(&driving(fixture, fixture_type, held, key), now_ms)
}

/// The parameters of a type an operator can set, in the order it lists them.
///
/// Inputs are left out: a contact closure is a parameter a device writes and the
/// show reads, and there is nothing to send home.
pub fn output_parameters(fixture_type: &FixtureType) -> impl Iterator<Item = &ParameterDefinition> {
    fixture_type.parameters.iter().filter(|p| p.direction == ParameterDirection::Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_written_before_bindings_existed_still_loads() {
        let legacy = serde_json::json!({
            "kind": "Intensity",
            "dmx_channel": 4,
            "default_value": { "type": "Float", "value": 0.0 }
        });

        let parsed: ParameterDefinition = serde_json::from_value(legacy).unwrap();

        assert_eq!(parsed.binding, Some(ParameterBinding::Dmx { channel: 4 }));
        assert_eq!(parsed.direction, ParameterDirection::Output);
    }

    #[test]
    fn a_parameter_round_trips_through_its_current_shape() {
        let definition = ParameterDefinition {
            direction: ParameterDirection::Input,
            binding: Some(ParameterBinding::Port { index: 3 }),
            ..ParameterDefinition::new(ParameterKind::Contact(3), ParameterValue::Bool(false))
        };

        let json = serde_json::to_value(&definition).unwrap();
        let back: ParameterDefinition = serde_json::from_value(json).unwrap();

        assert_eq!(back, definition);
    }

    fn a_type(parameters: Vec<ParameterDefinition>) -> FixtureType {
        FixtureType {
            id: Uuid::nil(),
            name: "Par".into(),
            manufacturer: "Nobody".into(),
            channel_count: 1,
            parameters,
            ..FixtureType::default()
        }
    }

    fn a_fixture(home_values: HashMap<String, ParameterValue>) -> Fixture {
        Fixture {
            id: Uuid::nil(),
            name: "House left".into(),
            fixture_type_id: Uuid::nil(),
            address: FixtureAddress::default(),
            position: None,
            sensed_values: HashMap::new(),
            live_effects: HashMap::new(),
            live_fades: HashMap::new(),
            home_values,
        }
    }

    fn an_intensity(default_value: ParameterValue) -> ParameterDefinition {
        ParameterDefinition {
            binding: Some(ParameterBinding::Dmx { channel: 1 }),
            ..ParameterDefinition::new(ParameterKind::Intensity, default_value)
        }
    }

    /// `home_values` is a column that did not exist, so a fixture written before it
    /// has to read back as one with nothing to say rather than as a parse failure.
    #[test]
    fn a_fixture_written_before_home_values_existed_still_loads() {
        let legacy = serde_json::json!({
            "id": Uuid::nil(),
            "name": "House left",
            "fixture_type_id": Uuid::nil(),
            "address": { "Dmx": { "universe": 1, "address": 1 } },
            "position": null,
            "live_values": {},
        });

        let parsed: Fixture = serde_json::from_value(legacy).unwrap();

        assert!(parsed.home_values.is_empty(), "nothing to say, rather than nothing at all");
    }

    /// A row from before values stopped being stored, opened here.
    ///
    /// `live_values` is *ignored* rather than read into `sensed_values`. Most of what
    /// it carried was the console's own output, which is now a function of what is
    /// driving the parameter; filing that as something a device reported would be a
    /// station claiming to have been told what it had in fact decided.
    #[test]
    fn a_fixture_written_before_values_stopped_being_stored_still_loads() {
        let legacy = serde_json::json!({
            "id": Uuid::nil(),
            "name": "House left",
            "fixture_type_id": Uuid::nil(),
            "address": { "Dmx": { "universe": 1, "address": 1 } },
            "position": null,
            "live_values": {
                "Intensity": { "type": "Float", "value": 0.8 },
                "Contact:0": { "type": "Bool", "value": true },
            },
            "home_values": { "Intensity": { "type": "Float", "value": 1.0 } },
        });

        let parsed: Fixture = serde_json::from_value(legacy).expect("an older row still reads");

        assert!(parsed.sensed_values.is_empty(), "the old map is ignored, not adopted");
        assert!(parsed.live_fades.is_empty());
        assert!(parsed.live_effects.is_empty());
        // And nothing else was lost getting there — the fields that outlived the
        // change arrive as they were written.
        assert_eq!(parsed.name, "House left");
        assert_eq!(parsed.home_values.get("Intensity"), Some(&ParameterValue::Float(1.0)));
    }

    /// And the other direction: a station on an older build receiving a row from this
    /// one, which simply does not carry the field it is looking for.
    #[test]
    fn a_row_from_here_omits_the_field_an_older_station_looks_for() {
        let fixture = a_fixture(HashMap::new());
        let written = serde_json::to_value(&fixture).expect("a fixture serialises");

        assert!(written.get("live_values").is_none(), "there is no such field any more");
        assert!(written.get("sensed_values").is_some(), "and what replaced half of it is there");
        // An older build defaults what it cannot find, which is the whole of what it
        // has to do: `live_values` was `#[serde(default)]`-adjacent there too.
        assert_eq!(written["sensed_values"], serde_json::json!({}));
    }

    #[test]
    fn a_fixture_with_no_override_rests_where_its_type_says() {
        let fixture_type = a_type(vec![an_intensity(ParameterValue::Float(0.0))]);
        let fixture = a_fixture(HashMap::new());

        assert_eq!(
            home_value(&fixture, &fixture_type, &ParameterKind::Intensity),
            Some(ParameterValue::Float(0.0))
        );
    }

    /// The case that forces the override to exist: a house light is on when nothing
    /// is controlling it, and its type — derived from what the node said about its
    /// ports — has no way to know that.
    #[test]
    fn a_fixture_with_an_override_rests_there_instead() {
        let fixture_type = a_type(vec![an_intensity(ParameterValue::Float(0.0))]);
        let fixture = a_fixture(HashMap::from([(
            "Intensity".to_string(),
            ParameterValue::Float(1.0),
        )]));

        assert_eq!(
            home_value(&fixture, &fixture_type, &ParameterKind::Intensity),
            Some(ParameterValue::Float(1.0))
        );
    }

    #[test]
    fn a_parameter_the_type_does_not_have_rests_nowhere() {
        let fixture_type = a_type(vec![an_intensity(ParameterValue::Float(0.0))]);
        let fixture = a_fixture(HashMap::new());

        assert_eq!(
            home_value(&fixture, &fixture_type, &ParameterKind::Pan),
            None,
            "a fixture that cannot pan has nowhere for a pan to rest"
        );
    }

    #[test]
    fn an_input_is_not_something_to_send_home() {
        let fixture_type = a_type(vec![
            an_intensity(ParameterValue::Float(0.0)),
            ParameterDefinition {
                direction: ParameterDirection::Input,
                binding: Some(ParameterBinding::Port { index: 0 }),
                ..ParameterDefinition::new(ParameterKind::Contact(0), ParameterValue::Bool(false))
            },
        ]);

        let kinds: Vec<&ParameterKind> =
            output_parameters(&fixture_type).map(|p| &p.kind).collect();

        assert_eq!(kinds, vec![&ParameterKind::Intensity]);
    }

    #[test]
    fn a_parameter_key_says_which_one_it_is() {
        assert_eq!(parameter_key(&ParameterKind::Intensity), "Intensity");
        assert_ne!(
            parameter_key(&ParameterKind::Raw(5)),
            parameter_key(&ParameterKind::Raw(6)),
            "two raw channels are two keys"
        );
        assert_eq!(parameter_key(&ParameterKind::Named("Fog".into())), "Named:Fog");
    }

    #[test]
    fn an_address_answers_only_for_the_kind_it_is() {
        let dmx = FixtureAddress::dmx(2, 17);
        assert_eq!(dmx.dmx_start(), Some((2, 17)));
        assert_eq!(dmx.mode(), Some(DEFAULT_MODE));
        assert_eq!(dmx.serial(), None);

        let node = FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: Some(5) };
        assert_eq!(node.dmx_start(), None, "a node fixture has no slot in a universe");
        assert!(node.breaks().is_empty());
        assert_eq!(node.serial(), Some("1a2b3c"));
    }

    #[test]
    fn a_nudge_moves_what_arithmetic_means_something_for() {
        assert_eq!(
            ParameterValue::Float(0.5).nudged(0.1).unwrap(),
            ParameterValue::Float(0.6)
        );
        assert_eq!(ParameterValue::Int(3).nudged(2.0).unwrap(), ParameterValue::Int(5));
        assert_eq!(
            ParameterValue::rgb(0.2, 0.5, 0.9).nudged(0.1).unwrap(),
            ParameterValue::rgb(0.3, 0.6, 1.0),
            "every channel moves, so a nudge means brighter rather than redder"
        );
    }

    /// Past the top is the top. An operator holding a fader at full and asking for
    /// more should get full, not a value the output has to clamp behind their back.
    #[test]
    fn a_nudge_comes_to_rest_inside_the_range() {
        assert_eq!(ParameterValue::Float(0.95).nudged(0.2).unwrap(), ParameterValue::Float(1.0));
        assert_eq!(ParameterValue::Float(0.05).nudged(-0.2).unwrap(), ParameterValue::Float(0.0));

        // Approximately, because f32 subtraction is: what is being asserted is that
        // the channel already at the bottom stops there while the others carry on.
        let ParameterValue::Color { r, g, b, .. } =
            ParameterValue::rgb(0.9, 0.1, 0.5).nudged(-0.3).unwrap()
        else {
            panic!("a nudged colour is a colour")
        };
        assert!((r - 0.6).abs() < 1e-5, "{r}");
        assert_eq!(g, 0.0, "the channel that would have gone under stops at the bottom");
        assert!((b - 0.2).abs() < 1e-5, "{b}");
    }

    #[test]
    fn a_nudge_says_so_where_there_is_no_halfway() {
        let switch = ParameterValue::Bool(true).nudged(0.1).expect_err("a relay has no halfway");
        assert!(switch.contains("on or off"), "{switch}");
        let text =
            ParameterValue::Text("HELLO".into()).nudged(1.0).expect_err("text cannot be nudged");
        assert!(text.contains("text"), "{text}");
    }
}
