//! `<PhysicalDescriptions>`: what the light actually is.
//!
//! Emitters and filters are how the console works out that an RGBW head's fourth
//! channel is white rather than a fourth colour it should ignore; `Properties`
//! carries the weight and the power draw the paperwork wants.

use serde::{Deserialize, Serialize};

use crate::values::{ColorCie, DmxValue, Node, de_from_str_opt, de_number_opt, ser_display_opt};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PhysicalDescriptions {
    #[serde(rename = "Emitters", default, skip_serializing_if = "Option::is_none")]
    pub emitters: Option<Emitters>,
    #[serde(rename = "Filters", default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Filters>,
    #[serde(
        rename = "ColorSpace",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub color_space: Option<ColorSpace>,
    #[serde(
        rename = "AdditionalColorSpaces",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_color_spaces: Option<AdditionalColorSpaces>,
    #[serde(rename = "Gamuts", default, skip_serializing_if = "Option::is_none")]
    pub gamuts: Option<Gamuts>,
    #[serde(
        rename = "DMXProfiles",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub dmx_profiles: Option<DmxProfiles>,
    #[serde(rename = "CRIs", default, skip_serializing_if = "Option::is_none")]
    pub cris: Option<Cris>,
    #[serde(
        rename = "Connectors",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub connectors: Option<Connectors>,
    #[serde(
        rename = "Properties",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub properties: Option<Properties>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Emitters {
    #[serde(rename = "Emitter", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Emitter>,
}

/// One light source in the head: a red die, a lime die, a discharge lamp.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Emitter {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Color", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color: Option<ColorCie>,
    #[serde(
        rename = "@DominantWaveLength",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub dominant_wave_length: Option<f32>,
    #[serde(
        rename = "@DiodePart",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub diode_part: String,
    #[serde(rename = "Measurement", default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<Measurement>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Filters {
    #[serde(rename = "Filter", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Filter>,
}

/// A subtractive element: a CMY flag, a colour wheel's glass.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Filter {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Color", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color: Option<ColorCie>,
    #[serde(rename = "Measurement", default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<Measurement>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(rename = "@Physical", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub physical: Option<f32>,
    #[serde(
        rename = "@LuminousIntensity",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub luminous_intensity: Option<f32>,
    #[serde(
        rename = "@Transmission",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub transmission: Option<f32>,
    #[serde(
        rename = "@InterpolationTo",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub interpolation_to: String,
    #[serde(
        rename = "MeasurementPoint",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub points: Vec<MeasurementPoint>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MeasurementPoint {
    #[serde(
        rename = "@WaveLength",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub wave_length: Option<f32>,
    #[serde(rename = "@Energy", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub energy: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AdditionalColorSpaces {
    #[serde(rename = "ColorSpace", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ColorSpace>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ColorSpace {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Mode", default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(rename = "@Red", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub red: Option<ColorCie>,
    #[serde(rename = "@Green", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub green: Option<ColorCie>,
    #[serde(rename = "@Blue", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub blue: Option<ColorCie>,
    #[serde(
        rename = "@WhitePoint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub white_point: Option<ColorCie>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Gamuts {
    #[serde(rename = "Gamut", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Gamut>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Gamut {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Points", default, skip_serializing_if = "String::is_empty")]
    pub points: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxProfiles {
    #[serde(rename = "DMXProfile", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DmxProfile>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxProfile {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "Point", default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<DmxProfilePoint>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxProfilePoint {
    #[serde(
        rename = "@DMXPercentage",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub dmx_percentage: Option<f32>,
    #[serde(rename = "@CFC0", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub cfc0: Option<f32>,
    #[serde(rename = "@CFC1", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub cfc1: Option<f32>,
    #[serde(rename = "@CFC2", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub cfc2: Option<f32>,
    #[serde(rename = "@CFC3", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub cfc3: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cris {
    #[serde(rename = "CRIGroup", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<CriGroup>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CriGroup {
    #[serde(
        rename = "@ColorTemperature",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub color_temperature: Option<f32>,
    #[serde(rename = "CRI", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Cri>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cri {
    #[serde(rename = "@CES", default, skip_serializing_if = "String::is_empty")]
    pub ces: String,
    #[serde(
        rename = "@ColorRenderingIndex",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub color_rendering_index: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Connectors {
    #[serde(rename = "Connector", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Connector>,
}

/// A socket on the fixture. What a plot needs to say what plugs into what.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Connector {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Type", default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(rename = "@DMXBreak", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub dmx_break: Option<u16>,
    #[serde(rename = "@Gender", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub gender: Option<i32>,
    #[serde(rename = "@Length", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub length: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Properties {
    #[serde(
        rename = "OperatingTemperature",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub operating_temperature: Option<OperatingTemperature>,
    #[serde(rename = "Weight", default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<Weight>,
    #[serde(
        rename = "PowerConsumption",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub power_consumption: Vec<PowerConsumption>,
    #[serde(rename = "LegHeight", default, skip_serializing_if = "Option::is_none")]
    pub leg_height: Option<LegHeight>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OperatingTemperature {
    #[serde(rename = "@Low", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub low: Option<f32>,
    #[serde(rename = "@High", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub high: Option<f32>,
}

/// Kilograms, as the spec has it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Weight {
    #[serde(rename = "@Value", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub value: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PowerConsumption {
    #[serde(rename = "@Value", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub value: Option<f32>,
    #[serde(
        rename = "@PowerFactor",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub power_factor: Option<f32>,
    #[serde(
        rename = "@Connector",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub connector: String,
    #[serde(
        rename = "@VoltageLow",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub voltage_low: Option<f32>,
    #[serde(
        rename = "@VoltageHigh",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub voltage_high: Option<f32>,
    #[serde(
        rename = "@FrequencyLow",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub frequency_low: Option<f32>,
    #[serde(
        rename = "@FrequencyHigh",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub frequency_high: Option<f32>,
}

/// Metres from the floor to the bottom of the fixture, for a floor-standing one.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LegHeight {
    #[serde(rename = "@Value", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub value: Option<f32>,
}

/// Not part of `PhysicalDescriptions`, but the same idea and needed by the same
/// callers: a `ChannelFunction`'s `DMXProfile` reference resolves to one of the
/// profiles above.
pub type ProfileRef = Node;

/// Re-exported so a caller matching on channel defaults does not have to reach into
/// `values` for the one type it needs.
pub type ChannelDefault = DmxValue;
