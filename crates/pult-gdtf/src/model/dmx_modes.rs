//! `<DMXModes>`: how the channels are laid out, which is where a mode differs from
//! another mode.
//!
//! Four levels deep and each level means something. A `DMXChannel` is a place in the
//! universe — a break, and one to four byte offsets, which need not be adjacent. A
//! `LogicalChannel` is what that place controls. A `ChannelFunction` is what it does
//! over a range of that place's values, and a fixture that dims over 0–200 and
//! strobes over 201–255 says so with two of them. A `ChannelSet` names a slice of a
//! function: "Gobo 3", "Open", "Slow".

use serde::{Deserialize, Serialize};

use crate::values::{DmxValue, Node, de_from_str_opt, de_number_opt, ser_display_opt};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxModes {
    #[serde(rename = "DMXMode", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DmxMode>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxMode {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "@Description",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub description: String,
    /// The geometry this mode drives — the root of the subtree its channels address.
    #[serde(
        rename = "@Geometry",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub geometry: String,
    #[serde(rename = "DMXChannels", default)]
    pub dmx_channels: DmxChannels,
    #[serde(rename = "Relations", default, skip_serializing_if = "Option::is_none")]
    pub relations: Option<Relations>,
    #[serde(rename = "FTMacros", default, skip_serializing_if = "Option::is_none")]
    pub ft_macros: Option<FtMacros>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxChannels {
    #[serde(rename = "DMXChannel", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DmxChannel>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxChannel {
    #[serde(
        rename = "@DMXBreak",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub dmx_break: String,
    /// `"1,2"` for a 16-bit channel, `"None"` for one that occupies no slot. 1-based
    /// from the fixture's start address, coarse first.
    #[serde(rename = "@Offset", default, skip_serializing_if = "String::is_empty")]
    pub offset: String,
    #[serde(
        rename = "@InitialFunction",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub initial_function: Option<Node>,
    #[serde(
        rename = "@Highlight",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub highlight: Option<DmxValue>,
    #[serde(
        rename = "@Geometry",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub geometry: String,
    #[serde(
        rename = "LogicalChannel",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub logical_channels: Vec<LogicalChannel>,
}

impl DmxChannel {
    /// The byte offsets this channel occupies, 1-based, coarse to fine.
    ///
    /// Empty for `Offset="None"` and for a missing attribute — a virtual channel that
    /// occupies no DMX slot, which a footprint must not count and a writer must not
    /// write.
    pub fn offsets(&self) -> Vec<u16> {
        self.offset
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty() && !part.eq_ignore_ascii_case("none"))
            .filter_map(|part| part.parse().ok())
            .collect()
    }

    /// Which break this channel is in. The spec's default is 1, and `"Overwrite"`
    /// means "whatever the reference that expanded me said", resolved during
    /// expansion rather than here.
    pub fn break_number(&self) -> Option<u16> {
        match self.dmx_break.trim() {
            "" => Some(1),
            text if text.eq_ignore_ascii_case("overwrite") => None,
            text => text.parse().ok(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LogicalChannel {
    #[serde(
        rename = "@Attribute",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub attribute: Option<Node>,
    #[serde(rename = "@Snap", default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<Snap>,
    #[serde(rename = "@Master", default, skip_serializing_if = "Option::is_none")]
    pub master: Option<Master>,
    #[serde(rename = "@MibFade", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub mib_fade: Option<f32>,
    #[serde(
        rename = "@DMXChangeTimeLimit",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub dmx_change_time_limit: Option<f32>,
    #[serde(
        rename = "ChannelFunction",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub channel_functions: Vec<ChannelFunction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Snap {
    #[default]
    No,
    Yes,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Master {
    #[default]
    None,
    Grand,
    Group,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChannelFunction {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "@Attribute",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub attribute: Option<Node>,
    #[serde(
        rename = "@OriginalAttribute",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub original_attribute: String,
    #[serde(rename = "@DMXFrom", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub dmx_from: Option<DmxValue>,
    #[serde(rename = "@Default", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub default: Option<DmxValue>,
    #[serde(
        rename = "@PhysicalFrom",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub physical_from: Option<f32>,
    #[serde(
        rename = "@PhysicalTo",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub physical_to: Option<f32>,
    #[serde(rename = "@RealFade", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub real_fade: Option<f32>,
    #[serde(
        rename = "@RealAcceleration",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub real_acceleration: Option<f32>,
    #[serde(rename = "@Wheel", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub wheel: Option<Node>,
    #[serde(rename = "@Emitter", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub emitter: Option<Node>,
    #[serde(rename = "@Filter", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub filter: Option<Node>,
    #[serde(
        rename = "@ColorSpace",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color_space: Option<Node>,
    #[serde(rename = "@Gamut", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub gamut: Option<Node>,
    /// The channel whose value decides whether this function is in play at all — how
    /// a fixture says "this is a strobe rate only while the shutter channel is in its
    /// strobe range".
    #[serde(
        rename = "@ModeMaster",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub mode_master: Option<Node>,
    #[serde(rename = "@ModeFrom", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub mode_from: Option<DmxValue>,
    #[serde(rename = "@ModeTo", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub mode_to: Option<DmxValue>,
    #[serde(
        rename = "@DMXProfile",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub dmx_profile: Option<Node>,
    #[serde(rename = "@Min", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub min: Option<f32>,
    #[serde(rename = "@Max", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub max: Option<f32>,
    #[serde(
        rename = "@CustomName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub custom_name: String,
    #[serde(rename = "ChannelSet", default, skip_serializing_if = "Vec::is_empty")]
    pub channel_sets: Vec<ChannelSet>,
    #[serde(
        rename = "SubChannelSet",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub sub_channel_sets: Vec<SubChannelSet>,
}

/// A named slice of a function's range: the thing an operator picks by name.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChannelSet {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@DMXFrom", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub dmx_from: Option<DmxValue>,
    #[serde(
        rename = "@PhysicalFrom",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub physical_from: Option<f32>,
    #[serde(
        rename = "@PhysicalTo",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub physical_to: Option<f32>,
    /// 1-based into the function's wheel. Zero means "no slot", which is not the same
    /// as the first one.
    #[serde(
        rename = "@WheelSlotIndex",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub wheel_slot_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SubChannelSet {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "@PhysicalFrom",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub physical_from: Option<f32>,
    #[serde(
        rename = "@PhysicalTo",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub physical_to: Option<f32>,
    #[serde(
        rename = "@SubPhysicalUnit",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub sub_physical_unit: Option<Node>,
    #[serde(
        rename = "@DMXProfile",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub dmx_profile: Option<Node>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Relations {
    #[serde(rename = "Relation", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Relation>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Relation {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Master", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub master: Option<Node>,
    #[serde(rename = "@Follower", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub follower: Option<Node>,
    #[serde(rename = "@Type", default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FtMacros {
    #[serde(rename = "FTMacro", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<FtMacro>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FtMacro {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "@ChannelFunction",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub channel_function: Option<Node>,
    #[serde(rename = "MacroDMX", default, skip_serializing_if = "Option::is_none")]
    pub macro_dmx: Option<MacroDmx>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MacroDmx {
    #[serde(
        rename = "MacroDMXStep",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub steps: Vec<MacroDmxStep>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MacroDmxStep {
    #[serde(rename = "@Duration", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub duration: Option<f32>,
    #[serde(
        rename = "MacroDMXValue",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub values: Vec<MacroDmxValue>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MacroDmxValue {
    #[serde(rename = "@Value", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub value: Option<DmxValue>,
    #[serde(
        rename = "@DMXChannel",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub dmx_channel: Option<Node>,
}
