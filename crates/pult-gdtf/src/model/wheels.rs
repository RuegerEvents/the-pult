//! `<Wheels>`: the discrete things a fixture can put in the beam.
//!
//! A gobo wheel, a colour wheel, a prism, an animation disc. Each slot becomes a
//! named position the console offers instead of a raw number, which is the
//! difference between "Gobo 1 at 37%" and "Gobo 1: Breakup".

use serde::{Deserialize, Serialize};

use crate::values::{ColorCie, Node, de_from_str_opt, de_number_opt, de_value_opt, ser_display_opt};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Wheels {
    #[serde(rename = "Wheel", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Wheel>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Wheel {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "Slot", default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<WheelSlot>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WheelSlot {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Color", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_value_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color: Option<ColorCie>,
    /// A file in `wheels/` — the gobo image, which the browser can draw.
    #[serde(
        rename = "@MediaFileName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub media_file_name: String,
    #[serde(rename = "@Filter", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub filter: Option<Node>,
    #[serde(rename = "Facet", default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<PrismFacet>,
    #[serde(
        rename = "AnimationSystem",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub animation_system: Option<AnimationSystem>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PrismFacet {
    #[serde(rename = "@Color", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_value_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color: Option<ColorCie>,
    #[serde(rename = "@Rotation", default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<crate::values::Rotation>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AnimationSystem {
    #[serde(rename = "@P1", default, skip_serializing_if = "String::is_empty")]
    pub p1: String,
    #[serde(rename = "@P2", default, skip_serializing_if = "String::is_empty")]
    pub p2: String,
    #[serde(rename = "@P3", default, skip_serializing_if = "String::is_empty")]
    pub p3: String,
    #[serde(rename = "@Radius", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub radius: Option<f32>,
}
