//! `<Models>`: the meshes, and the box to draw when there is no mesh.
//!
//! A `Model` names a file without an extension — the same name may exist as
//! `models/gltf/head.glb` and `models/3ds/head.3ds`, and picking between them is
//! [`crate::resolve`]'s job. Its `Length`/`Width`/`Height` are **metres**, unlike
//! geometry translations, which are millimetres. That inconsistency is the spec's,
//! not ours, and it is written down here because it is the sort of thing that gets
//! silently unified by whoever touches this next.

use serde::{Deserialize, Serialize};

use crate::values::de_number_opt;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Models {
    #[serde(rename = "Model", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Model>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Model {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Metres.
    #[serde(rename = "@Length", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub length: Option<f32>,
    #[serde(rename = "@Width", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub width: Option<f32>,
    #[serde(rename = "@Height", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub height: Option<f32>,
    #[serde(
        rename = "@PrimitiveType",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub primitive_type: Option<PrimitiveType>,
    /// The file's stem, with no directory and no extension.
    #[serde(rename = "@File", default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(
        rename = "@SVGOffsetX",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub svg_offset_x: Option<f32>,
    #[serde(
        rename = "@SVGOffsetY",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub svg_offset_y: Option<f32>,
    #[serde(
        rename = "@SVGSideOffsetX",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub svg_side_offset_x: Option<f32>,
    #[serde(
        rename = "@SVGSideOffsetY",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub svg_side_offset_y: Option<f32>,
    #[serde(
        rename = "@SVGFrontOffsetX",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub svg_front_offset_x: Option<f32>,
    #[serde(
        rename = "@SVGFrontOffsetY",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub svg_front_offset_y: Option<f32>,
}

/// A shape a renderer can draw without a mesh file at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PrimitiveType {
    #[default]
    Undefined,
    Cube,
    Cylinder,
    Sphere,
    Base,
    Yoke,
    Head,
    Scanner,
    Conventional,
    Pigtail,
    Base1_1,
    Scanner1_1,
    Conventional1_1,
}
