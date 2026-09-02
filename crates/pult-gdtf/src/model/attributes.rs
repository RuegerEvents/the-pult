//! `<AttributeDefinitions>`: what the fixture's channels *mean*.
//!
//! This is the half of GDTF the console actually maps onto its own model. A
//! channel's `Attribute` is a name out of the spec's list — `Dimmer`, `Pan`,
//! `ColorAdd_R`, `Gobo1`, `Zoom` — and turning that name into a `ParameterKind` is
//! the whole of the import's vocabulary problem.

use serde::{Deserialize, Serialize};

use crate::values::{de_from_str_opt, de_number_opt, de_value_opt, ser_display_opt, ColorCie, Node};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AttributeDefinitions {
    #[serde(
        rename = "ActivationGroups",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub activation_groups: Option<ActivationGroups>,
    #[serde(rename = "FeatureGroups", default)]
    pub feature_groups: FeatureGroups,
    #[serde(rename = "Attributes", default)]
    pub attributes: Attributes,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActivationGroups {
    #[serde(
        rename = "ActivationGroup",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub items: Vec<ActivationGroup>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActivationGroup {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FeatureGroups {
    #[serde(
        rename = "FeatureGroup",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub items: Vec<FeatureGroup>,
}

/// A grouping the console keeps as `feature_group` on the parameter, because it is
/// what an operator's encoder pages are built from: Dimmer, Position, Colour, Beam.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FeatureGroup {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Pretty", default, skip_serializing_if = "String::is_empty")]
    pub pretty: String,
    #[serde(rename = "Feature", default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Feature {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Attributes {
    #[serde(rename = "Attribute", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Attribute {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Pretty", default, skip_serializing_if = "String::is_empty")]
    pub pretty: String,
    #[serde(
        rename = "@ActivationGroup",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub activation_group: Option<Node>,
    #[serde(rename = "@Feature", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub feature: Option<Node>,
    /// The coarser attribute this one refines: `Pan` for `PanRotate`, and the reason
    /// a fine channel does not become a parameter of its own.
    #[serde(
        rename = "@MainAttribute",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub main_attribute: Option<Node>,
    #[serde(
        rename = "@PhysicalUnit",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub physical_unit: Option<PhysicalUnit>,
    #[serde(rename = "@Color", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_value_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color: Option<ColorCie>,
    #[serde(
        rename = "SubPhysicalUnit",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub sub_physical_units: Vec<SubPhysicalUnit>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SubPhysicalUnit {
    #[serde(rename = "@Type", default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(
        rename = "@PhysicalUnit",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub physical_unit: Option<PhysicalUnit>,
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
}

/// What the numbers on a channel are measured in.
///
/// The console keeps this on the parameter so a pan reads in degrees and a zoom in
/// degrees of beam angle rather than both being "a number between 0 and 1".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PhysicalUnit {
    #[default]
    None,
    Percent,
    Length,
    Mass,
    Time,
    Temperature,
    LuminousIntensity,
    Angle,
    Force,
    Frequency,
    Current,
    Voltage,
    Power,
    Energy,
    Area,
    Volume,
    Speed,
    Acceleration,
    AngularSpeed,
    AngularAccc,
    WaveLength,
    ColorComponent,
}

impl PhysicalUnit {
    /// The unit's short name, for a label beside a number.
    pub fn symbol(self) -> &'static str {
        match self {
            PhysicalUnit::None | PhysicalUnit::ColorComponent => "",
            PhysicalUnit::Percent => "%",
            PhysicalUnit::Length => "m",
            PhysicalUnit::Mass => "kg",
            PhysicalUnit::Time => "s",
            PhysicalUnit::Temperature => "K",
            PhysicalUnit::LuminousIntensity => "cd",
            PhysicalUnit::Angle => "°",
            PhysicalUnit::Force => "N",
            PhysicalUnit::Frequency => "Hz",
            PhysicalUnit::Current => "A",
            PhysicalUnit::Voltage => "V",
            PhysicalUnit::Power => "W",
            PhysicalUnit::Energy => "J",
            PhysicalUnit::Area => "m²",
            PhysicalUnit::Volume => "m³",
            PhysicalUnit::Speed => "m/s",
            PhysicalUnit::Acceleration => "m/s²",
            PhysicalUnit::AngularSpeed => "°/s",
            PhysicalUnit::AngularAccc => "°/s²",
            PhysicalUnit::WaveLength => "nm",
        }
    }
}
